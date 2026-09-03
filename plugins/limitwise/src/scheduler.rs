use crate::config::{
    codex_binary, poll_seconds, set_private_file, Paths, FIVE_HOUR_RESERVE_PERCENT,
    MISSED_GRACE_SECONDS,
};
use crate::store::{now_epoch, Batch, RunFinish, Store, Task};
use crate::transcript::token_usage;
use crate::usage::{UsageClient, UsageSnapshot};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

const INTERRUPT_WAIT_STEPS: usize = 20;

pub fn daemon(once: bool) -> Result<(), String> {
    loop {
        let mut store = Store::open()?;
        loop {
            let failed_dependencies = store.failed_dependency_tasks()?;
            let due = store.due_tasks(now_epoch())?;
            if failed_dependencies.is_empty() && due.is_empty() {
                break;
            }
            for task in failed_dependencies {
                if !store.claim_task(&task.id)? {
                    continue;
                }
                let dependency = task.depends_on_task_id.as_deref().unwrap_or("unknown");
                let status = store
                    .task(dependency)?
                    .map(|value| value.status)
                    .unwrap_or_else(|| "missing".to_string());
                record_without_launch(
                    &store,
                    &task,
                    "blocked",
                    &format!(
                        "prerequisite task {dependency} did not complete successfully ({status})"
                    ),
                )?;
            }
            for task in due {
                if !store.claim_task(&task.id)? {
                    continue;
                }
                if let Err(error) = process_claimed_task(&mut store, &task) {
                    let _ = store.set_status(&task.id, "failed", Some(&error));
                    notify(&task.title, &format!("failed: {error}"));
                }
            }
        }
        if once {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn process_claimed_task(store: &mut Store, task: &Task) -> Result<(), String> {
    let eligible_at = if task.depends_on_task_id.is_some() {
        store
            .dependency_completed_at(task)?
            .ok_or_else(|| "prerequisite task is not completed".to_string())?
    } else {
        task.run_at
    };
    let lateness = now_epoch().saturating_sub(eligible_at);
    if lateness > MISSED_GRACE_SECONDS {
        return record_without_launch(
            store,
            task,
            "missed",
            &format!("task was {lateness} seconds late; grace is {MISSED_GRACE_SECONDS} seconds"),
        );
    }

    let client = UsageClient::default();
    let before = match client.fetch() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return record_without_launch(
                store,
                task,
                "quota_skipped",
                &format!("quota telemetry unavailable: {error}"),
            )
        }
    };
    let before_json = serde_json::to_string(&before).map_err(|e| e.to_string())?;
    let run_id = store.start_run(&task.id, &before_json)?;
    let mut budget = store
        .batch(&task.batch_id)?
        .ok_or_else(|| "task batch not found".to_string())?;
    if budget.budget_mode == "percentage" {
        store.ensure_budget_window(
            &task.batch_id,
            before.weekly.used_percent,
            before.weekly.resets_at,
        )?;
        budget = store.reconcile_consumption(&task.batch_id, before.weekly.used_percent)?;
    }

    if before.five_hour.used_percent >= 100.0 - FIVE_HOUR_RESERVE_PERCENT {
        return finish_without_launch(
            store,
            task,
            &run_id,
            "quota_skipped",
            "rolling 5-hour usage is at or above 90%",
            Some(&before),
        );
    }
    if before.weekly.used_percent >= 100.0 {
        return finish_without_launch(
            store,
            task,
            &run_id,
            "quota_skipped",
            "weekly usage is exhausted",
            Some(&before),
        );
    }
    if budget_exhausted(&budget) {
        return finish_without_launch(
            store,
            task,
            &run_id,
            "quota_skipped",
            budget_exhausted_reason(&budget),
            Some(&before),
        );
    }

    let paths = Paths::discover()?;
    paths.ensure()?;
    let transcript = paths.logs_dir.join(format!("{}-{run_id}.jsonl", task.id));
    let output = private_output_file(&transcript)?;
    let mut child = spawn_codex(task, &output)?;
    let mut last = before.clone();
    let mut recorded_task_tokens = 0;
    let mut interrupt_reason = None;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        thread::sleep(Duration::from_secs(poll_seconds()));
        match client.fetch() {
            Ok(snapshot) => {
                if budget.budget_mode == "percentage" {
                    budget = update_percentage_budget(store, &task.batch_id, &last, &snapshot)?;
                }
                last = snapshot;
                if last.five_hour.used_percent >= 100.0 - FIVE_HOUR_RESERVE_PERCENT {
                    interrupt_reason = Some("rolling 5-hour usage reached 90%".to_string());
                } else if last.weekly.used_percent >= 100.0 {
                    interrupt_reason = Some("weekly usage was exhausted".to_string());
                } else if budget_exhausted(&budget) {
                    interrupt_reason = Some(budget_exhausted_reason(&budget).to_string());
                }
            }
            Err(error) => {
                interrupt_reason = Some(format!("quota telemetry became unavailable: {error}"));
            }
        }
        if budget.budget_mode == "tokens" {
            if let Some(observed) = read_text(&transcript).and_then(|text| token_usage(&text)) {
                if observed > recorded_task_tokens {
                    budget = store
                        .add_token_consumption(&task.batch_id, observed - recorded_task_tokens)?;
                    recorded_task_tokens = observed;
                }
                if budget_exhausted(&budget) {
                    interrupt_reason = Some(budget_exhausted_reason(&budget).to_string());
                }
            }
        }
        if interrupt_reason.is_some() {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                interrupt_reason = None;
                break status;
            }
            break interrupt_child(&mut child)?;
        }
    };

    let after = client.fetch().ok();
    if budget.budget_mode == "percentage" {
        if let Some(snapshot) = &after {
            budget = update_percentage_budget(store, &task.batch_id, &last, snapshot)?;
        }
    }
    let transcript_text = read_text(&transcript);
    let observed_tokens = transcript_text.as_deref().and_then(token_usage);
    if budget.budget_mode == "tokens" {
        if let Some(observed) = observed_tokens {
            if observed > recorded_task_tokens {
                budget =
                    store.add_token_consumption(&task.batch_id, observed - recorded_task_tokens)?;
            }
        }
    }
    let session_id = transcript_text.as_deref().and_then(find_session_id);
    let transcript_string = transcript.to_string_lossy().to_string();
    let after_json = after
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok());

    let token_accounting_error = if budget.budget_mode == "tokens" && observed_tokens.is_none() {
        budget = store.exhaust_token_budget(&task.batch_id)?;
        let _ = budget;
        Some("Codex transcript contained no token usage; token budget closed".to_string())
    } else {
        None
    };
    let (final_status, error) = if let Some(reason) = interrupt_reason {
        ("quota_interrupted", Some(reason))
    } else if let Some(reason) = token_accounting_error {
        ("failed", Some(reason))
    } else if status.success() {
        ("completed", None)
    } else if looks_blocked(transcript_text.as_deref().unwrap_or("")) {
        (
            "blocked",
            Some("Codex required an approval or disallowed capability".to_string()),
        )
    } else {
        ("failed", Some(format!("Codex exited with {status}")))
    };
    store.finish_run(
        &run_id,
        RunFinish {
            status: final_status,
            usage_json: after_json.as_deref(),
            session_id: session_id.as_deref(),
            transcript: Some(&transcript_string),
            tokens_used: observed_tokens,
            error: error.as_deref(),
        },
    )?;
    store.set_status(&task.id, final_status, error.as_deref())?;
    notify_with_tokens(&task.title, final_status, observed_tokens);
    Ok(())
}

fn record_without_launch(
    store: &Store,
    task: &Task,
    status: &str,
    reason: &str,
) -> Result<(), String> {
    let run_id = store.start_run(&task.id, "{}")?;
    store.finish_run(
        &run_id,
        RunFinish {
            status,
            usage_json: None,
            session_id: None,
            transcript: None,
            tokens_used: Some(0),
            error: Some(reason),
        },
    )?;
    store.set_status(&task.id, status, Some(reason))?;
    notify_with_tokens(&task.title, status, Some(0));
    Ok(())
}

fn finish_without_launch(
    store: &Store,
    task: &Task,
    run_id: &str,
    status: &str,
    reason: &str,
    usage: Option<&UsageSnapshot>,
) -> Result<(), String> {
    let usage_json = usage.and_then(|value| serde_json::to_string(value).ok());
    store.finish_run(
        run_id,
        RunFinish {
            status,
            usage_json: usage_json.as_deref(),
            session_id: None,
            transcript: None,
            tokens_used: Some(0),
            error: Some(reason),
        },
    )?;
    store.set_status(&task.id, status, Some(reason))?;
    notify_with_tokens(&task.title, status, Some(0));
    Ok(())
}

fn update_percentage_budget(
    store: &Store,
    batch_id: &str,
    previous: &UsageSnapshot,
    current: &UsageSnapshot,
) -> Result<Batch, String> {
    let mut budget = store.ensure_budget_window(
        batch_id,
        current.weekly.used_percent,
        current.weekly.resets_at,
    )?;
    if previous.weekly.resets_at == current.weekly.resets_at {
        let delta = (current.weekly.used_percent - previous.weekly.used_percent).max(0.0);
        if delta > 0.0 {
            budget = store.add_consumption(batch_id, delta)?;
        }
    }
    store
        .reconcile_consumption(batch_id, current.weekly.used_percent)
        .or(Ok(budget))
}

fn budget_exhausted(batch: &Batch) -> bool {
    if batch.budget_mode == "tokens" {
        match batch.token_cap {
            Some(cap) => batch.consumed_tokens >= cap,
            None => true,
        }
    } else {
        batch.allowance_points <= 0.0
            || batch.consumed_points + f64::EPSILON >= batch.allowance_points
    }
}

fn budget_exhausted_reason(batch: &Batch) -> &'static str {
    if batch.budget_mode == "tokens" {
        "shared batch token cap is exhausted"
    } else {
        "shared weekly batch allowance is exhausted"
    }
}

fn private_output_file(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    set_private_file(path)?;
    Ok(file)
}

fn spawn_codex(task: &Task, output: &File) -> Result<Child, String> {
    let prompt = codex_prompt(task);
    let stdout = output.try_clone().map_err(|e| e.to_string())?;
    let stderr = output.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(codex_binary());
    command
        .arg("exec")
        .arg("--model")
        .arg(&task.model)
        .arg("-c")
        .arg(format!("model_reasoning_effort=\"{}\"", task.effort))
        .arg("-c")
        .arg("approval_policy=\"never\"")
        .arg("-c")
        .arg("sandbox_workspace_write.network_access=false")
        .arg("-c")
        .arg("web_search=\"disabled\"")
        .arg("-c")
        .arg("features.apps=false")
        .args(["--sandbox", "workspace-write"])
        .args(["--ignore-user-config", "--skip-git-repo-check", "--json"])
        .arg("--cd")
        .arg(&task.cwd)
        .arg(prompt)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map_err(|e| format!("cannot start Codex task: {e}"))
}

fn codex_prompt(task: &Task) -> String {
    format!(
        "Execute this previously confirmed one-off task.\n\nTask: {}\n\nPrompt:\n{}\n\nSuccess criteria:\n{}\n\nConstraints: work only inside the selected project; do not use network or external apps; do not perform destructive or approval-dependent actions. If any such action is required, report that the task is blocked.\n\nOutput style: keep final summaries terse. Remove filler, repeated summaries, and unnecessary explanation. Preserve exact commands, file paths, code, JSON, timestamps, IDs, model names, effort values, and error text. Do not shorten safety warnings or any wording where brevity could change meaning.",
        task.title, task.prompt, task.success_criteria
    )
}

fn interrupt_child(child: &mut Child) -> Result<ExitStatus, String> {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGINT);
    }
    for _ in 0..INTERRUPT_WAIT_STEPS {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(500));
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    child.wait().map_err(|e| e.to_string())
}

fn read_text(path: &Path) -> Option<String> {
    let mut content = String::new();
    File::open(path).ok()?.read_to_string(&mut content).ok()?;
    Some(content)
}

fn find_session_id(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if let Some(found) = find_string_key(
                &value,
                &["thread_id", "threadId", "session_id", "sessionId"],
            ) {
                return Some(found);
            }
        }
    }
    None
}

fn find_string_key(value: &Value, names: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for name in names {
                if let Some(found) = map.get(*name).and_then(Value::as_str) {
                    return Some(found.to_string());
                }
            }
            map.values().find_map(|value| find_string_key(value, names))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|value| find_string_key(value, names)),
        _ => None,
    }
}

fn looks_blocked(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    [
        "approval required",
        "permission denied",
        "sandbox violation",
        "network is disabled",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn notify(title: &str, result: &str) {
    if cfg!(target_os = "macos") {
        let script = format!(
            "display notification {:?} with title {:?}",
            format!("LimitWise task: {result}"),
            title
        );
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let _ = Command::new("notify-send")
            .args([&format!("LimitWise: {title}"), result])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn notify_with_tokens(title: &str, status: &str, tokens_used: Option<i64>) {
    let result = match tokens_used {
        Some(tokens) => format!("{status}; {tokens} tokens"),
        None => format!("{status}; token usage unavailable"),
    };
    notify(title, &result);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustion_includes_equality() {
        let batch = Batch {
            id: "b".into(),
            idempotency_key: "i".into(),
            budget_mode: "percentage".into(),
            weekly_cap_percent: Some(50.0),
            token_cap: None,
            consumed_tokens: 0,
            cap_percent: 50.0,
            cap_basis: "total_weekly_percent".into(),
            created_at: 0,
            window_reset_at: Some(1),
            baseline_weekly_used_percent: Some(40.0),
            allowance_points: 30.0,
            consumed_points: 30.0,
        };
        assert!(budget_exhausted(&batch));
    }

    #[test]
    fn token_exhaustion_includes_equality() {
        let batch = Batch {
            id: "b".into(),
            idempotency_key: "i".into(),
            budget_mode: "tokens".into(),
            weekly_cap_percent: None,
            token_cap: Some(100),
            consumed_tokens: 100,
            cap_percent: 0.0,
            cap_basis: "total_tokens".into(),
            created_at: 0,
            window_reset_at: None,
            baseline_weekly_used_percent: None,
            allowance_points: 0.0,
            consumed_points: 0.0,
        };
        assert!(budget_exhausted(&batch));
    }

    #[test]
    fn extracts_nested_session_id() {
        assert_eq!(
            find_session_id("{\"type\":\"thread.started\",\"thread_id\":\"abc\"}"),
            Some("abc".into())
        );
    }

    #[test]
    fn codex_prompt_requests_terse_output_without_losing_constraints() {
        let task = Task {
            id: "task-1".into(),
            batch_id: "batch-1".into(),
            title: "Update status".into(),
            prompt: "Write status.txt exactly.".into(),
            success_criteria: "status.txt contains expected text.".into(),
            cwd: "/tmp/project".into(),
            run_at: 4_102_444_800,
            run_at_iso: "2100-01-01T00:00:00+00:00".into(),
            position: 0,
            depends_on_task_id: None,
            timezone: "UTC".into(),
            difficulty: "simple".into(),
            model: "gpt-5.6-luna".into(),
            effort: "low".into(),
            status: "scheduled".into(),
            created_at: 0,
            updated_at: 0,
            last_error: None,
        };

        let prompt = codex_prompt(&task);

        assert!(prompt.contains("Task: Update status"));
        assert!(prompt.contains("Prompt:\nWrite status.txt exactly."));
        assert!(prompt.contains("Success criteria:\nstatus.txt contains expected text."));
        assert!(prompt.contains("work only inside the selected project"));
        assert!(prompt.contains("do not use network or external apps"));
        assert!(prompt.contains("do not perform destructive or approval-dependent actions"));
        assert!(prompt.contains("keep final summaries terse"));
        assert!(prompt.contains("Preserve exact commands, file paths, code, JSON, timestamps, IDs, model names, effort values, and error text"));
    }
}
