use crate::config::codex_binary;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const ADAPTER_VERSION: &str = "codex-app-server-v1";
const REQUEST_TIMEOUT_SECONDS: u64 = 15;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RateWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub duration_minutes: i64,
    pub resets_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct UsageSnapshot {
    pub adapter: String,
    pub captured_at: i64,
    pub five_hour: RateWindow,
    pub weekly: RateWindow,
}

#[derive(Debug)]
pub struct UsageClient {
    timeout: Duration,
    session: Mutex<Option<AppServer>>,
}

#[derive(Debug)]
struct AppServer {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    receiver: mpsc::Receiver<Value>,
    next_id: i64,
}

impl Default for UsageClient {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
            session: Mutex::new(None),
        }
    }
}

impl UsageClient {
    pub fn fetch(&self) -> Result<UsageSnapshot, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "quota adapter lock poisoned".to_string())?;
        if guard.is_none() {
            *guard = Some(AppServer::start(self.timeout)?);
        }
        let result = guard
            .as_mut()
            .expect("session initialized")
            .fetch(self.timeout);
        if result.is_err() {
            *guard = None;
        }
        result
    }
}

impl AppServer {
    fn start(timeout: Duration) -> Result<Self, String> {
        let mut child = Command::new(codex_binary())
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start Codex app-server: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "app-server stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "app-server stdout unavailable".to_string())?;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&line) {
                            let _ = sender.send(value);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut server = Self {
            child,
            stdin,
            receiver,
            next_id: 2,
        };
        write_message(
            &mut server.stdin,
            &json!({
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {"name": "limitwise", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true}
                }
            }),
        )?;
        wait_for_response(&server.receiver, 1, timeout)?;
        write_message(&mut server.stdin, &json!({"method": "initialized"}))?;
        Ok(server)
    }

    fn fetch(&mut self, timeout: Duration) -> Result<UsageSnapshot, String> {
        let id = self.next_id;
        self.next_id += 1;
        write_message(
            &mut self.stdin,
            &json!({"id": id, "method": "account/rateLimits/read"}),
        )?;
        let response = wait_for_response(&self.receiver, id, timeout)?;
        if let Some(error) = response.get("error") {
            return Err(format!("quota request failed: {error}"));
        }
        let result = response
            .get("result")
            .ok_or_else(|| "quota response has no result".to_string())?;
        snapshot_from_value(result)
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_message(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

fn wait_for_response(
    receiver: &mpsc::Receiver<Value>,
    id: i64,
    timeout: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Codex app-server quota telemetry timed out".to_string());
        }
        let value = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Codex app-server quota telemetry timed out".to_string())?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
}

pub fn snapshot_from_value(value: &Value) -> Result<UsageSnapshot, String> {
    let mut candidates = Vec::new();
    collect_windows(value, &mut candidates);
    let mut seen = HashSet::new();
    candidates.retain(|window| {
        seen.insert((
            window.duration_minutes,
            window.resets_at,
            (window.used_percent * 1000.0).round() as i64,
        ))
    });

    let five_hour: Vec<_> = candidates
        .iter()
        .filter(|window| window.duration_minutes == 300)
        .cloned()
        .collect();
    if five_hour.len() != 1 {
        return Err("5-hour quota telemetry is missing or ambiguous".to_string());
    }
    let longest = candidates
        .iter()
        .filter(|window| window.duration_minutes > 300)
        .map(|window| window.duration_minutes)
        .max()
        .ok_or_else(|| "weekly quota telemetry is missing".to_string())?;
    let weekly: Vec<_> = candidates
        .iter()
        .filter(|window| window.duration_minutes == longest)
        .cloned()
        .collect();
    if weekly.len() != 1 {
        return Err("weekly quota telemetry is ambiguous".to_string());
    }
    Ok(UsageSnapshot {
        adapter: ADAPTER_VERSION.to_string(),
        captured_at: crate::store::now_epoch(),
        five_hour: five_hour[0].clone(),
        weekly: weekly[0].clone(),
    })
}

fn collect_windows(value: &Value, output: &mut Vec<RateWindow>) {
    match value {
        Value::Object(map) => {
            let used = map
                .get("usedPercent")
                .or_else(|| map.get("used_percent"))
                .and_then(Value::as_f64);
            let duration = map
                .get("windowDurationMins")
                .or_else(|| map.get("window_duration_mins"))
                .and_then(Value::as_i64);
            let reset = map
                .get("resetsAt")
                .or_else(|| map.get("resets_at"))
                .and_then(parse_reset);
            if let (Some(used_percent), Some(duration_minutes), Some(resets_at)) =
                (used, duration, reset)
            {
                if used_percent.is_finite()
                    && (0.0..=100.0).contains(&used_percent)
                    && duration_minutes > 0
                {
                    output.push(RateWindow {
                        used_percent,
                        remaining_percent: (100.0 - used_percent).max(0.0),
                        duration_minutes,
                        resets_at,
                    });
                }
            }
            for nested in map.values() {
                collect_windows(nested, output);
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_windows(nested, output);
            }
        }
        _ => {}
    }
}

fn parse_reset(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value.as_str().and_then(|text| {
            text.parse::<i64>().ok().or_else(|| {
                DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|parsed| parsed.timestamp())
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_five_hour_and_longest_window() {
        let input = json!({
            "rateLimits": {
                "primary": {"usedPercent": 75.0, "windowDurationMins": 300, "resetsAt": 10},
                "secondary": {"usedPercent": 40.0, "windowDurationMins": 10080, "resetsAt": 20}
            }
        });
        let snapshot = snapshot_from_value(&input).unwrap();
        assert_eq!(snapshot.five_hour.duration_minutes, 300);
        assert_eq!(snapshot.weekly.remaining_percent, 60.0);
    }

    #[test]
    fn telemetry_fails_closed_when_missing_or_ambiguous() {
        assert!(snapshot_from_value(&json!({})).is_err());
        let ambiguous = json!({
            "a": {"usedPercent": 1.0, "windowDurationMins": 300, "resetsAt": 1},
            "b": {"usedPercent": 2.0, "windowDurationMins": 300, "resetsAt": 2},
            "w": {"usedPercent": 2.0, "windowDurationMins": 10080, "resetsAt": 3}
        });
        assert!(snapshot_from_value(&ambiguous).is_err());
    }
}
