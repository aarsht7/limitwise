#![cfg_attr(test, allow(dead_code))]

use crate::config::codex_binary;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const CATALOG_TIMEOUT: Duration = Duration::from_secs(10);
const CATALOG_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEffortOption {
    pub reasoning_effort: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub default_reasoning_effort: String,
    pub supported_reasoning_efforts: Vec<ReasoningEffortOption>,
    pub is_default: bool,
    #[serde(default)]
    pub upgrade: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ModelCatalog {
    pub source: &'static str,
    pub warning: Option<String>,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug)]
pub struct ModelCatalogClient {
    binary: PathBuf,
    timeout: Duration,
}

impl Default for ModelCatalogClient {
    fn default() -> Self {
        Self {
            binary: codex_binary(),
            timeout: CATALOG_TIMEOUT,
        }
    }
}

impl ModelCatalogClient {
    pub fn fetch(&self) -> Result<ModelCatalog, String> {
        fetch_catalog(&self.binary, self.timeout)
    }
}

struct CatalogCache {
    loaded_at: Instant,
    catalog: ModelCatalog,
}

static MODEL_CATALOG: OnceLock<Mutex<Option<CatalogCache>>> = OnceLock::new();

pub fn model_catalog() -> ModelCatalog {
    #[cfg(test)]
    {
        built_in_catalog(None)
    }
    #[cfg(not(test))]
    {
        let cache = MODEL_CATALOG.get_or_init(|| Mutex::new(None));
        if let Ok(guard) = cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.loaded_at.elapsed() < CATALOG_TTL {
                    return cached.catalog.clone();
                }
            }
        }
        let catalog = ModelCatalogClient::default()
            .fetch()
            .unwrap_or_else(|error| built_in_catalog(Some(error)));
        if let Ok(mut guard) = cache.lock() {
            *guard = Some(CatalogCache {
                loaded_at: Instant::now(),
                catalog: catalog.clone(),
            });
        }
        catalog
    }
}

fn fetch_catalog(binary: &Path, timeout: Duration) -> Result<ModelCatalog, String> {
    let mut child = Command::new(binary)
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start Codex model discovery: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex model discovery stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex model discovery stdout unavailable".to_string())?;
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

    let result = (|| {
        write_message(
            &mut stdin,
            &json!({
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {"name": "limitwise", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true}
                }
            }),
        )?;
        response_result(wait_for_response(&receiver, 1, timeout)?, "initialize")?;
        write_message(&mut stdin, &json!({"method": "initialized"}))?;

        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        let mut request_id = 2_i64;
        loop {
            write_message(
                &mut stdin,
                &json!({
                    "id": request_id,
                    "method": "model/list",
                    "params": {"cursor": cursor, "includeHidden": false, "limit": 100}
                }),
            )?;
            let value = response_result(
                wait_for_response(&receiver, request_id, timeout)?,
                "model/list",
            )?;
            let page: ModelListPage = serde_json::from_value(value)
                .map_err(|error| format!("invalid Codex model/list response: {error}"))?;
            models.extend(
                page.data
                    .into_iter()
                    .filter(|model| !model.model.trim().is_empty()),
            );
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
            request_id += 1;
        }
        if models.is_empty() {
            return Err("Codex model/list returned no visible models".to_string());
        }
        Ok(ModelCatalog {
            source: "codex",
            warning: None,
            models,
        })
    })();

    let _ = child.kill();
    let _ = child.wait();
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelListPage {
    data: Vec<ModelInfo>,
    #[serde(default)]
    next_cursor: Option<String>,
}

fn write_message(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
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
            return Err("Codex model discovery timed out".to_string());
        }
        let value = match receiver.recv_timeout(remaining) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("Codex model discovery timed out".to_string())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Codex model discovery stopped before returning a response".to_string())
            }
        };
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
}

fn response_result(response: Value, operation: &str) -> Result<Value, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("Codex model discovery {operation} failed: {error}"));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| format!("Codex model discovery {operation} returned no result"))
}

fn built_in_catalog(error: Option<String>) -> ModelCatalog {
    fn efforts(values: &[&str]) -> Vec<ReasoningEffortOption> {
        values
            .iter()
            .map(|value| ReasoningEffortOption {
                reasoning_effort: (*value).to_string(),
                description: match *value {
                    "low" => "Fast responses with lighter reasoning",
                    "medium" => "Balances speed and reasoning depth for everyday tasks",
                    "high" => "Greater reasoning depth for complex problems",
                    "xhigh" => "Extra high reasoning depth for complex problems",
                    "max" => "Maximum reasoning depth for the hardest problems",
                    "ultra" => "Maximum reasoning with automatic task delegation",
                    _ => "Reasoning effort",
                }
                .to_string(),
            })
            .collect()
    }
    fn model(
        id: &str,
        display_name: &str,
        description: &str,
        default_effort: &str,
        supported: &[&str],
        is_default: bool,
        upgrade: Option<&str>,
    ) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            model: id.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            default_reasoning_effort: default_effort.to_string(),
            supported_reasoning_efforts: efforts(supported),
            is_default,
            upgrade: upgrade.map(str::to_string),
        }
    }
    let warning = error
        .map(|error| format!("Codex model discovery failed; using the bundled catalog: {error}"));
    ModelCatalog {
        source: if warning.is_some() {
            "fallback"
        } else {
            "bundled"
        },
        warning,
        models: vec![
            model(
                "gpt-6-astra",
                "GPT-6-Astra",
                "Frontier intelligence for the most demanding work.",
                "low",
                &["low", "medium", "high", "xhigh", "max", "ultra"],
                true,
                None,
            ),
            model(
                "gpt-6-sol",
                "GPT-6-Sol",
                "Workhorse model for coding and everyday work.",
                "medium",
                &["low", "medium", "high", "xhigh", "max", "ultra"],
                false,
                None,
            ),
            model(
                "gpt-6-luna",
                "GPT-6-Luna",
                "Fast and affordable model for easier tasks.",
                "medium",
                &["low", "medium", "high", "xhigh", "max"],
                false,
                None,
            ),
            model(
                "gpt-5.6-sol",
                "GPT-5.6-Sol",
                "Older coding model for complex work.",
                "low",
                &["low", "medium", "high", "xhigh", "max", "ultra"],
                false,
                Some("gpt-6-sol"),
            ),
            model(
                "gpt-5.6-terra",
                "GPT-5.6-Terra",
                "Older balanced model for straightforward work.",
                "medium",
                &["low", "medium", "high", "xhigh", "max", "ultra"],
                false,
                Some("gpt-6-sol"),
            ),
            model(
                "gpt-5.6-luna",
                "GPT-5.6-Luna",
                "Older fast and efficient model.",
                "medium",
                &["low", "medium", "high", "xhigh", "max"],
                false,
                Some("gpt-6-luna"),
            ),
            model(
                "gpt-5.5",
                "GPT-5.5",
                "Legacy coding model.",
                "medium",
                &["low", "medium", "high", "xhigh"],
                false,
                Some("gpt-6-sol"),
            ),
        ],
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    #[default]
    Restricted,
    Networked,
}

impl fmt::Display for PermissionProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Restricted => "restricted",
            Self::Networked => "networked",
        })
    }
}

impl FromStr for PermissionProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "restricted" => Ok(Self::Restricted),
            "networked" => Ok(Self::Networked),
            _ => Err("permission_profile must be restricted or networked".to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Simple,
    Standard,
    Complex,
    Exceptional,
}

impl FromStr for Difficulty {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "simple" => Ok(Self::Simple),
            "standard" | "normal" => Ok(Self::Standard),
            "complex" => Ok(Self::Complex),
            "exceptional" => Ok(Self::Exceptional),
            _ => Err("difficulty must be simple, standard, complex, or exceptional".to_string()),
        }
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Simple => "simple",
            Self::Standard => "standard",
            Self::Complex => "complex",
            Self::Exceptional => "exceptional",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub model: String,
    pub effort: String,
}

impl Route {
    fn new(model: &str, effort: &str) -> Self {
        Self {
            model: model.to_string(),
            effort: effort.to_string(),
        }
    }
}

pub fn route(difficulty: Difficulty) -> Route {
    match difficulty {
        Difficulty::Simple => Route::new("gpt-6-luna", "low"),
        Difficulty::Standard => Route::new("gpt-6-sol", "medium"),
        Difficulty::Complex => Route::new("gpt-6-astra", "high"),
        Difficulty::Exceptional => Route::new("gpt-6-astra", "xhigh"),
    }
}

pub fn validate_route(model: &str, effort: &str) -> Result<(), String> {
    validate_route_in(&model_catalog(), model, effort)
}

fn validate_route_in(catalog: &ModelCatalog, model: &str, effort: &str) -> Result<(), String> {
    let selected = catalog
        .models
        .iter()
        .find(|candidate| candidate.model == model)
        .ok_or_else(|| {
            format!(
                "model '{model}' is not available from Codex; available models: {}",
                catalog
                    .models
                    .iter()
                    .map(|candidate| candidate.model.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    if selected
        .supported_reasoning_efforts
        .iter()
        .any(|candidate| candidate.reasoning_effort == effort)
    {
        return Ok(());
    }
    Err(format!(
        "effort '{effort}' is not supported by model '{model}'; supported efforts: {}",
        selected
            .supported_reasoning_efforts
            .iter()
            .map(|candidate| candidate.reasoning_effort.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_every_difficulty() {
        assert_eq!(route(Difficulty::Simple), Route::new("gpt-6-luna", "low"));
        assert_eq!(
            route(Difficulty::Standard),
            Route::new("gpt-6-sol", "medium")
        );
        assert_eq!(
            route(Difficulty::Complex),
            Route::new("gpt-6-astra", "high")
        );
        assert_eq!(
            route(Difficulty::Exceptional),
            Route::new("gpt-6-astra", "xhigh")
        );
    }
    #[test]
    fn validates_effort_against_the_selected_model() {
        assert!(validate_route("gpt-6-astra", "ultra").is_ok());
        assert!(validate_route("gpt-6-luna", "max").is_ok());
        assert!(validate_route("gpt-6-luna", "ultra").is_err());
        assert!(validate_route("gpt-5.5", "max").is_err());
    }

    #[test]
    fn bundled_catalog_contains_current_and_legacy_codex_models() {
        let catalog = model_catalog();
        assert_eq!(catalog.source, "bundled");
        assert!(catalog
            .models
            .iter()
            .any(|model| model.model == "gpt-6-astra"));
        assert!(catalog
            .models
            .iter()
            .any(|model| model.model == "gpt-5.6-sol"));
        assert!(catalog.models.iter().any(|model| model.model == "gpt-5.5"));
    }

    #[test]
    fn permission_profile_is_closed_and_defaults_restricted() {
        assert_eq!(PermissionProfile::default(), PermissionProfile::Restricted);
        assert_eq!(
            serde_json::from_str::<PermissionProfile>("\"networked\"").unwrap(),
            PermissionProfile::Networked
        );
        assert!(serde_json::from_str::<PermissionProfile>(
            "\"networked --sandbox danger-full-access\""
        )
        .is_err());
        assert!(serde_json::from_str::<PermissionProfile>("\"danger-full-access\"").is_err());
    }
}
