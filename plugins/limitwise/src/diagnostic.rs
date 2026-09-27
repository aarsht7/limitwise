use crate::config::{codex_binary, home_dir, Paths};
use crate::usage::UsageClient;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const DIAGNOSTIC_SCHEMA_VERSION: &str = "1";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const QUOTA_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_COMMAND_OUTPUT: usize = 16 * 1024;
const MAX_LOG_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStatus {
    Pass,
    Warn,
    Fail,
}

pub type DiagnosticSeverity = DiagnosticStatus;

impl DiagnosticStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticPlatform {
    pub os: String,
    pub architecture: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticEvidence {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticRemediation {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticCheck {
    pub id: String,
    pub status: DiagnosticSeverity,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<DiagnosticEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<DiagnosticRemediation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub schema_version: String,
    pub overall: DiagnosticSeverity,
    pub generated_at: i64,
    pub platform: DiagnosticPlatform,
    pub checks: Vec<DiagnosticCheck>,
}

impl DiagnosticReport {
    fn new(generated_at: i64, platform: DiagnosticPlatform, checks: Vec<DiagnosticCheck>) -> Self {
        let overall = aggregate_status(&checks);
        Self {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION.to_string(),
            overall,
            generated_at,
            platform,
            checks,
        }
    }
}

#[derive(Debug)]
struct DiagnosticContext {
    paths: Paths,
    home: PathBuf,
    current_exe: PathBuf,
    codex: PathBuf,
    os: String,
    architecture: String,
    generated_at: i64,
    command_timeout: Duration,
    quota_timeout: Duration,
    command_environment: Vec<(OsString, OsString)>,
    systemctl: PathBuf,
    launchctl: PathBuf,
}

impl DiagnosticContext {
    fn discover() -> Result<Self, String> {
        Ok(Self {
            paths: Paths::discover()?,
            home: home_dir()?,
            current_exe: std::env::current_exe().map_err(|error| {
                format!("cannot resolve the current LimitWise executable: {error}")
            })?,
            codex: codex_binary(),
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            generated_at: crate::store::now_epoch(),
            command_timeout: COMMAND_TIMEOUT,
            quota_timeout: QUOTA_TIMEOUT,
            command_environment: Vec::new(),
            systemctl: PathBuf::from("systemctl"),
            launchctl: PathBuf::from("launchctl"),
        })
    }
}

#[derive(Debug)]
struct CommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

pub fn snapshot() -> Result<DiagnosticReport, String> {
    snapshot_with(&DiagnosticContext::discover()?)
}

fn snapshot_with(context: &DiagnosticContext) -> Result<DiagnosticReport, String> {
    let mut checks = Vec::new();
    checks.push(check_platform(context));
    checks.push(check_current_binary(context));
    checks.push(check_installed_binary(context));
    checks.extend(check_codex(context));
    checks.push(check_plugin(context));
    checks.push(check_mcp_configuration(context));
    checks.push(check_storage_paths(context));
    checks.push(check_database(context));
    checks.push(check_service(context));
    checks.push(check_daemon_log(context));
    Ok(DiagnosticReport::new(
        context.generated_at,
        DiagnosticPlatform {
            os: context.os.clone(),
            architecture: context.architecture.clone(),
        },
        checks,
    ))
}

pub fn run_cli(arguments: &[String], output: &mut impl Write) -> Result<i32, String> {
    run_cli_with(arguments, output, snapshot)
}

fn run_cli_with<F>(
    arguments: &[String],
    output: &mut impl Write,
    create_report: F,
) -> Result<i32, String>
where
    F: FnOnce() -> Result<DiagnosticReport, String>,
{
    let json = match arguments {
        [] => false,
        [argument] if argument == "--json" => true,
        _ => return Err("usage: limitwise doctor [--json]".to_string()),
    };
    let report = create_report()?;
    if json {
        write_json_report(&report, output)?;
    } else {
        write_human(&report, output)?;
    }
    Ok(if report.overall == DiagnosticStatus::Fail {
        1
    } else {
        0
    })
}

pub(crate) fn write_json_report(
    report: &DiagnosticReport,
    output: &mut impl Write,
) -> Result<(), String> {
    serde_json::to_writer_pretty(&mut *output, report).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())
}

fn write_human(report: &DiagnosticReport, output: &mut impl Write) -> Result<(), String> {
    writeln!(output, "LimitWise doctor: {}", report.overall.label())
        .map_err(|error| error.to_string())?;
    writeln!(output, "Schema: {}", report.schema_version).map_err(|error| error.to_string())?;
    writeln!(output, "Generated: {}", report.generated_at).map_err(|error| error.to_string())?;
    writeln!(
        output,
        "Platform: {}/{}",
        report.platform.os, report.platform.architecture
    )
    .map_err(|error| error.to_string())?;
    for check in &report.checks {
        write!(
            output,
            "{} {}: {}",
            check.status.label(),
            check.id,
            check.summary
        )
        .map_err(|error| error.to_string())?;
        if !check.evidence.is_empty() {
            let evidence = check
                .evidence
                .iter()
                .map(|item| format!("{}={}", item.label, item.value))
                .collect::<Vec<_>>()
                .join("; ");
            write!(output, " | evidence: {evidence}").map_err(|error| error.to_string())?;
        }
        if let Some(remediation) = &check.remediation {
            write!(output, " | remediation: {}", remediation.summary)
                .map_err(|error| error.to_string())?;
            if let Some(command) = &remediation.command {
                write!(output, " ({command})").map_err(|error| error.to_string())?;
            }
        }
        writeln!(output).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn aggregate_status(checks: &[DiagnosticCheck]) -> DiagnosticStatus {
    if checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Fail)
    {
        DiagnosticStatus::Fail
    } else if checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Warn)
    {
        DiagnosticStatus::Warn
    } else {
        DiagnosticStatus::Pass
    }
}

fn check_platform(context: &DiagnosticContext) -> DiagnosticCheck {
    let supported = matches!(
        (context.os.as_str(), context.architecture.as_str()),
        ("linux", "x86_64") | ("linux", "aarch64") | ("macos", "x86_64") | ("macos", "aarch64")
    );
    if !supported {
        return check(
            "platform.supported",
            DiagnosticStatus::Fail,
            "No LimitWise release target supports this platform",
            [
                evidence("os", &context.os),
                evidence("architecture", &context.architecture),
            ],
            Some(remediation(
                "Use a supported Linux or macOS architecture",
                None,
            )),
        );
    }
    let tested = context.os == "linux" && context.architecture == "x86_64";
    check(
        "platform.supported",
        if tested {
            DiagnosticStatus::Pass
        } else {
            DiagnosticStatus::Warn
        },
        if tested {
            "Platform is supported and tested"
        } else {
            "A release target exists, but this platform is not yet validated"
        },
        [
            evidence("os", &context.os),
            evidence("architecture", &context.architecture),
        ],
        if tested {
            None
        } else {
            Some(remediation(
                "Keep the compatibility warning until platform acceptance passes",
                None,
            ))
        },
    )
}

fn check_current_binary(context: &DiagnosticContext) -> DiagnosticCheck {
    match fs::metadata(&context.current_exe) {
        Ok(metadata) => {
            let mut problems = Vec::new();
            if !metadata.is_file() {
                problems.push("path is not a regular file");
            }
            if !is_executable(&metadata) {
                problems.push("file is not executable");
            }
            let owner = owner_label(&metadata);
            if owner == "other" {
                problems.push("owner is neither the current user nor root");
            }
            check(
                "binary.current",
                if problems.is_empty() {
                    DiagnosticStatus::Pass
                } else {
                    DiagnosticStatus::Fail
                },
                if problems.is_empty() {
                    "Current LimitWise binary is executable"
                } else {
                    "Current LimitWise binary is not usable"
                },
                [
                    evidence("path", &redacted_path(&context.current_exe, &context.home)),
                    evidence("version", env!("CARGO_PKG_VERSION")),
                    evidence("owner", &owner),
                    evidence("problems", &list_or_none(&problems)),
                ],
                if problems.is_empty() {
                    None
                } else {
                    Some(remediation("Reinstall the LimitWise binary", None))
                },
            )
        }
        Err(_) => check(
            "binary.current",
            DiagnosticStatus::Fail,
            "Current LimitWise binary metadata is unavailable",
            [evidence(
                "path",
                &redacted_path(&context.current_exe, &context.home),
            )],
            Some(remediation("Reinstall the LimitWise binary", None)),
        ),
    }
}

fn check_installed_binary(context: &DiagnosticContext) -> DiagnosticCheck {
    let path = &context.paths.installed_binary;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return check(
                "binary.installed",
                DiagnosticStatus::Warn,
                "No managed LimitWise binary is installed",
                [evidence("path", "$DATA/bin/limitwise")],
                Some(remediation(
                    "Run setup only if scheduled background execution is needed",
                    Some("limitwise setup"),
                )),
            )
        }
        Err(_) => {
            return check(
                "binary.installed",
                DiagnosticStatus::Fail,
                "Managed LimitWise binary metadata is unreadable",
                [evidence("path", "$DATA/bin/limitwise")],
                Some(remediation("Reinstall the LimitWise binary", None)),
            )
        }
    };
    let mut problems = private_file_problems(&metadata, true);
    if !is_executable(&metadata) {
        problems.push("not executable".to_string());
    }
    let output = run_command(
        path,
        &[OsStr::new("--version")],
        context.command_timeout,
        &context.command_environment,
    );
    let version = match output {
        Ok(output) if output.status.success() => parsed_version(&output.stdout),
        _ => None,
    };
    if version.is_none() {
        problems.push("version unavailable".to_string());
    }
    let version_mismatch = version
        .as_deref()
        .map(|value| !value.contains(env!("CARGO_PKG_VERSION")))
        .unwrap_or(false);
    let status = if !problems.is_empty() {
        DiagnosticStatus::Fail
    } else if version_mismatch {
        DiagnosticStatus::Warn
    } else {
        DiagnosticStatus::Pass
    };
    check(
        "binary.installed",
        status,
        match status {
            DiagnosticStatus::Pass => "Managed LimitWise binary is current and private",
            DiagnosticStatus::Warn => "Managed LimitWise binary version differs from this process",
            DiagnosticStatus::Fail => "Managed LimitWise binary is not usable",
        },
        [
            evidence("path", "$DATA/bin/limitwise"),
            evidence("version", version.as_deref().unwrap_or("unavailable")),
            evidence("problems", &list_or_none(&problems)),
        ],
        if status == DiagnosticStatus::Pass {
            None
        } else {
            Some(remediation("Reinstall the managed LimitWise binary", None))
        },
    )
}

fn check_codex(context: &DiagnosticContext) -> Vec<DiagnosticCheck> {
    let version_output = run_codex(context, &["--version"]);
    let executable = match version_output {
        Ok(output) if output.status.success() => match parsed_version(&output.stdout) {
            Some(version) => check(
                "codex.executable",
                DiagnosticStatus::Pass,
                "Codex executable is available",
                [
                    evidence("path", "<configured Codex executable>"),
                    evidence("version", &version),
                ],
                None,
            ),
            None => check(
                "codex.executable",
                DiagnosticStatus::Fail,
                "Codex returned malformed version output",
                [evidence("path", "<configured Codex executable>")],
                Some(remediation(
                    "Verify the Codex CLI installation",
                    Some("codex --version"),
                )),
            ),
        },
        Ok(output) => check(
            "codex.executable",
            DiagnosticStatus::Fail,
            "Codex version command failed",
            [evidence("exit_status", &status_label(output.status))],
            Some(remediation(
                "Verify the Codex CLI installation",
                Some("codex --version"),
            )),
        ),
        Err(CommandFailure::TimedOut) => check(
            "codex.executable",
            DiagnosticStatus::Fail,
            "Codex version command timed out",
            [],
            Some(remediation(
                "Verify the Codex CLI installation",
                Some("codex --version"),
            )),
        ),
        Err(CommandFailure::Start) => check(
            "codex.executable",
            DiagnosticStatus::Fail,
            "Codex executable was not found or could not start",
            [evidence("path", "<configured Codex executable>")],
            Some(remediation("Install Codex and ensure it is on PATH", None)),
        ),
        Err(CommandFailure::Wait) => check(
            "codex.executable",
            DiagnosticStatus::Fail,
            "Codex version command could not be observed",
            [],
            Some(remediation("Verify the Codex CLI installation", None)),
        ),
    };

    let login = match run_codex(context, &["login", "status"]) {
        Ok(output) if output.status.success() => check(
            "codex.login",
            DiagnosticStatus::Pass,
            "Codex reports an active login",
            [],
            None,
        ),
        Ok(output) => check(
            "codex.login",
            DiagnosticStatus::Fail,
            "Codex is not signed in",
            [evidence("exit_status", &status_label(output.status))],
            Some(remediation("Sign in to Codex", Some("codex login"))),
        ),
        Err(CommandFailure::TimedOut) => check(
            "codex.login",
            DiagnosticStatus::Fail,
            "Codex login status timed out",
            [],
            Some(remediation(
                "Check Codex login manually",
                Some("codex login status"),
            )),
        ),
        Err(_) => check(
            "codex.login",
            DiagnosticStatus::Fail,
            "Codex login status is unavailable",
            [],
            Some(remediation(
                "Install and sign in to Codex",
                Some("codex login"),
            )),
        ),
    };

    let usage = UsageClient::for_diagnostics(
        context.codex.clone(),
        context.quota_timeout,
        context.command_environment.clone(),
    );
    let quota = match usage.fetch() {
        Ok(snapshot) => match snapshot.five_hour {
            Some(five_hour) => check(
                "codex.quota",
                DiagnosticStatus::Pass,
                "Required quota telemetry is available and unambiguous",
                [
                    evidence("adapter", &snapshot.adapter),
                    evidence(
                        "five_hour_window_minutes",
                        &five_hour.duration_minutes.to_string(),
                    ),
                    evidence(
                        "weekly_window_minutes",
                        &snapshot.weekly.duration_minutes.to_string(),
                    ),
                ],
                None,
            ),
            None => check(
                "codex.quota",
                DiagnosticStatus::Warn,
                "Five-hour quota telemetry is unavailable; weekly or token budgets remain usable",
                [
                    evidence("adapter", &snapshot.adapter),
                    evidence(
                        "weekly_window_minutes",
                        &snapshot.weekly.duration_minutes.to_string(),
                    ),
                ],
                None,
            ),
        },
        Err(error) => check(
            "codex.quota",
            DiagnosticStatus::Fail,
            "Required quota telemetry is unavailable",
            [evidence("category", &quota_error_category(&error))],
            Some(remediation(
                "Confirm Codex is signed in and quota telemetry is available",
                Some("limitwise usage"),
            )),
        ),
    };
    vec![executable, login, quota]
}

fn check_plugin(context: &DiagnosticContext) -> DiagnosticCheck {
    match run_codex(
        context,
        &["plugin", "list", "--marketplace", "limitwise", "--json"],
    ) {
        Ok(output) if output.status.success() => {
            if plugin_inventory_has_enabled_limitwise(&output.stdout) {
                check(
                    "plugin.installed",
                    DiagnosticStatus::Pass,
                    "LimitWise plugin is registered with Codex",
                    [evidence("plugin", "limitwise@limitwise")],
                    None,
                )
            } else {
                check(
                    "plugin.installed",
                    DiagnosticStatus::Fail,
                    "LimitWise plugin is not registered with Codex",
                    [],
                    Some(remediation(
                        "Install the LimitWise plugin",
                        Some("codex plugin add limitwise@limitwise"),
                    )),
                )
            }
        }
        Ok(output) => check(
            "plugin.installed",
            DiagnosticStatus::Fail,
            "Codex plugin inventory failed",
            [evidence("exit_status", &status_label(output.status))],
            Some(remediation(
                "Inspect the Codex plugin inventory",
                Some("codex plugin list"),
            )),
        ),
        Err(CommandFailure::TimedOut) => check(
            "plugin.installed",
            DiagnosticStatus::Fail,
            "Codex plugin inventory timed out",
            [],
            Some(remediation(
                "Inspect the Codex plugin inventory",
                Some("codex plugin list"),
            )),
        ),
        Err(_) => check(
            "plugin.installed",
            DiagnosticStatus::Fail,
            "Codex plugin inventory is unavailable",
            [],
            Some(remediation("Install Codex and the LimitWise plugin", None)),
        ),
    }
}

fn plugin_inventory_has_enabled_limitwise(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output)
        .ok()
        .and_then(|inventory| inventory.get("installed")?.as_array().cloned())
        .is_some_and(|plugins| {
            plugins.iter().any(|plugin| {
                plugin.get("pluginId").and_then(|value| value.as_str())
                    == Some("limitwise@limitwise")
                    && plugin.get("installed").and_then(|value| value.as_bool()) == Some(true)
                    && plugin.get("enabled").and_then(|value| value.as_bool()) == Some(true)
            })
        })
}

fn check_mcp_configuration(context: &DiagnosticContext) -> DiagnosticCheck {
    match run_codex(context, &["mcp", "list"]) {
        Ok(output) if output.status.success() => {
            if output.stdout.to_ascii_lowercase().contains("limitwise") {
                check(
                    "plugin.mcp_configuration",
                    DiagnosticStatus::Pass,
                    "LimitWise MCP server is registered with Codex",
                    [evidence("server", "limitwise")],
                    None,
                )
            } else {
                check(
                    "plugin.mcp_configuration",
                    DiagnosticStatus::Fail,
                    "LimitWise MCP server is not registered with Codex",
                    [],
                    Some(remediation(
                        "Refresh the LimitWise plugin registration",
                        Some("codex plugin add limitwise@limitwise"),
                    )),
                )
            }
        }
        Ok(output) => check(
            "plugin.mcp_configuration",
            DiagnosticStatus::Fail,
            "Codex MCP inventory failed",
            [evidence("exit_status", &status_label(output.status))],
            Some(remediation(
                "Inspect the Codex MCP inventory",
                Some("codex mcp list"),
            )),
        ),
        Err(CommandFailure::TimedOut) => check(
            "plugin.mcp_configuration",
            DiagnosticStatus::Fail,
            "Codex MCP inventory timed out",
            [],
            Some(remediation(
                "Inspect the Codex MCP inventory",
                Some("codex mcp list"),
            )),
        ),
        Err(_) => check(
            "plugin.mcp_configuration",
            DiagnosticStatus::Fail,
            "Codex MCP inventory is unavailable",
            [],
            Some(remediation("Install Codex and the LimitWise plugin", None)),
        ),
    }
}

fn check_storage_paths(context: &DiagnosticContext) -> DiagnosticCheck {
    let mut missing = Vec::new();
    let mut problems = Vec::new();
    inspect_private_directory(&context.paths.data_dir, "data", &mut missing, &mut problems);
    inspect_private_directory(&context.paths.logs_dir, "logs", &mut missing, &mut problems);
    if context.paths.database.exists() {
        match fs::metadata(&context.paths.database) {
            Ok(metadata) => problems.extend(
                private_file_problems(&metadata, false)
                    .into_iter()
                    .map(|problem| format!("database {problem}")),
            ),
            Err(_) => problems.push("database metadata unreadable".to_string()),
        }
    } else {
        missing.push("database".to_string());
    }
    for (label, path) in [
        (
            "database WAL",
            sibling_with_suffix(&context.paths.database, "-wal"),
        ),
        (
            "database shared memory",
            sibling_with_suffix(&context.paths.database, "-shm"),
        ),
        (
            "daemon stdout log",
            context.paths.logs_dir.join("daemon.stdout.log"),
        ),
        (
            "daemon stderr log",
            context.paths.logs_dir.join("daemon.stderr.log"),
        ),
    ] {
        if let Ok(metadata) = fs::metadata(path) {
            problems.extend(
                private_file_problems(&metadata, false)
                    .into_iter()
                    .map(|problem| format!("{label} {problem}")),
            );
        }
    }
    let status = if !problems.is_empty() {
        DiagnosticStatus::Fail
    } else if !missing.is_empty() {
        DiagnosticStatus::Warn
    } else {
        DiagnosticStatus::Pass
    };
    check(
        "storage.paths",
        status,
        match status {
            DiagnosticStatus::Pass => "Data, log, and database paths are private",
            DiagnosticStatus::Warn => "Some local state does not exist yet",
            DiagnosticStatus::Fail => "Local state permissions or ownership are unsafe",
        },
        [
            evidence("data_path", "$DATA"),
            evidence("missing", &list_or_none(&missing)),
            evidence("problems", &list_or_none(&problems)),
        ],
        if status == DiagnosticStatus::Fail {
            Some(remediation(
                "Restrict LimitWise directories to 0700 and files to 0600",
                None,
            ))
        } else {
            None
        },
    )
}

fn check_database(context: &DiagnosticContext) -> DiagnosticCheck {
    if !context.paths.database.exists() {
        return check(
            "storage.database",
            DiagnosticStatus::Warn,
            "No LimitWise database exists yet",
            [evidence("path", "$DATA/limitwise.sqlite3")],
            None,
        );
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let database_uri = immutable_sqlite_uri(&context.paths.database);
    let connection = match Connection::open_with_flags(database_uri, flags) {
        Ok(connection) => connection,
        Err(_) => {
            return check(
                "storage.database",
                DiagnosticStatus::Fail,
                "LimitWise database cannot be opened read-only",
                [evidence("path", "$DATA/limitwise.sqlite3")],
                Some(remediation(
                    "Restore a readable database copy before running LimitWise",
                    None,
                )),
            )
        }
    };
    if connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map(|value| value != "ok")
        .unwrap_or(true)
    {
        return check(
            "storage.database",
            DiagnosticStatus::Fail,
            "SQLite integrity check failed",
            [evidence("path", "$DATA/limitwise.sqlite3")],
            Some(remediation(
                "Restore the database from a known-good copy",
                None,
            )),
        );
    }
    let required: BTreeMap<&str, &[&str]> = BTreeMap::from([
        (
            "batches",
            &["id", "idempotency_key", "cap_percent", "created_at"][..],
        ),
        ("runs", &["id", "task_id", "started_at", "status"][..]),
        (
            "tasks",
            &[
                "id",
                "batch_id",
                "title",
                "prompt",
                "success_criteria",
                "cwd",
                "run_at",
                "timezone",
                "difficulty",
                "model",
                "effort",
                "status",
                "created_at",
                "updated_at",
            ][..],
        ),
    ]);
    let optional: BTreeMap<&str, &[&str]> = BTreeMap::from([
        (
            "batches",
            &[
                "cap_basis",
                "budget_mode",
                "token_cap",
                "consumed_tokens",
                "window_reset_at",
                "baseline_weekly_used_percent",
                "allowance_points",
                "consumed_points",
                "five_hour_cap_percent",
                "five_hour_window_reset_at",
                "baseline_five_hour_used_percent",
                "five_hour_allowance_points",
                "five_hour_consumed_points",
            ][..],
        ),
        (
            "runs",
            &[
                "finished_at",
                "usage_before_json",
                "usage_after_json",
                "session_id",
                "transcript_path",
                "tokens_used",
                "token_usage_state",
                "error",
            ][..],
        ),
        (
            "tasks",
            &[
                "last_error",
                "position",
                "depends_on_task_id",
                "dependency_type",
            ][..],
        ),
    ]);
    let mut missing_required = Vec::new();
    let mut missing_optional = Vec::new();
    for (table, columns) in &required {
        match table_columns(&connection, table) {
            Ok(actual) => {
                for column in *columns {
                    if !actual.contains(*column) {
                        missing_required.push(format!("{table}.{column}"));
                    }
                }
                if let Some(expected) = optional.get(*table) {
                    for column in *expected {
                        if !actual.contains(*column) {
                            missing_optional.push(format!("{table}.{column}"));
                        }
                    }
                }
            }
            Err(_) => missing_required.push(format!("{table}.*")),
        }
    }
    let foreign_key_violations = connection
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut statement| {
            let mut rows = statement.query([])?;
            Ok(rows.next()?.is_some())
        })
        .unwrap_or(true);
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|_| "unavailable".to_string());
    let wal = sibling_with_suffix(&context.paths.database, "-wal");
    let wal_bytes = fs::metadata(wal)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let journal_warning = journal_mode != "wal";
    let status = if !missing_required.is_empty() || foreign_key_violations {
        DiagnosticStatus::Fail
    } else if !missing_optional.is_empty() || journal_warning {
        DiagnosticStatus::Warn
    } else {
        DiagnosticStatus::Pass
    };
    check(
        "storage.database",
        status,
        match status {
            DiagnosticStatus::Pass => "SQLite database is intact and current",
            DiagnosticStatus::Warn => {
                "SQLite database is intact but uses a compatible legacy schema or non-WAL journal"
            }
            DiagnosticStatus::Fail => "SQLite database schema or foreign keys are invalid",
        },
        [
            evidence("integrity", "ok"),
            evidence("journal_mode", &journal_mode),
            evidence("wal_bytes", &wal_bytes.to_string()),
            evidence("missing_required", &list_or_none(&missing_required)),
            evidence("missing_optional", &list_or_none(&missing_optional)),
            evidence(
                "foreign_keys",
                if foreign_key_violations {
                    "violations"
                } else {
                    "ok"
                },
            ),
        ],
        if status == DiagnosticStatus::Fail {
            Some(remediation(
                "Back up the database and repair or restore it before scheduling work",
                None,
            ))
        } else if status == DiagnosticStatus::Warn {
            Some(remediation(
                "A normal LimitWise write path will apply additive migrations when needed",
                None,
            ))
        } else {
            None
        },
    )
}

fn check_service(context: &DiagnosticContext) -> DiagnosticCheck {
    match context.os.as_str() {
        "linux" => {
            let definition = context.home.join(".config/systemd/user/limitwise.service");
            check_service_definition(
                context,
                &definition,
                "systemd",
                &context.systemctl,
                &["--user", "is-active", "limitwise.service"],
            )
        }
        "macos" => {
            let definition = context
                .home
                .join("Library/LaunchAgents/io.openai.limitwise.plist");
            let service = format!("gui/{}/io.openai.limitwise", current_uid());
            check_service_definition(
                context,
                &definition,
                "launchd",
                &context.launchctl,
                &["print", &service],
            )
        }
        _ => check(
            "service.state",
            DiagnosticStatus::Warn,
            "No service inspector is available for this platform",
            [],
            None,
        ),
    }
}

fn check_service_definition(
    context: &DiagnosticContext,
    definition: &Path,
    manager: &str,
    manager_binary: &Path,
    arguments: &[&str],
) -> DiagnosticCheck {
    let contents = match fs::read_to_string(definition) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return check(
                "service.state",
                DiagnosticStatus::Warn,
                "Optional background service is not installed",
                [evidence("manager", manager), evidence("state", "absent")],
                Some(remediation(
                    "Install the service only if scheduled execution is needed",
                    Some("limitwise setup"),
                )),
            )
        }
        Err(_) => {
            return check(
                "service.state",
                DiagnosticStatus::Fail,
                "Installed service definition is unreadable",
                [
                    evidence("manager", manager),
                    evidence("state", "unreadable"),
                ],
                Some(remediation(
                    "Reinstall the LimitWise service",
                    Some("limitwise setup"),
                )),
            )
        }
    };
    let expected_binary = context.paths.installed_binary.to_string_lossy();
    let escaped_systemd = expected_binary.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped_xml = expected_binary
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let binary_matches = [
        expected_binary.as_ref(),
        escaped_systemd.as_str(),
        escaped_xml.as_str(),
    ]
    .iter()
    .any(|candidate| contents.contains(*candidate));
    if !binary_matches || !contents.contains("daemon") {
        return check(
            "service.state",
            DiagnosticStatus::Fail,
            "Installed service definition points at stale or invalid execution settings",
            [
                evidence("manager", manager),
                evidence("state", "definition_mismatch"),
            ],
            Some(remediation(
                "Reinstall the LimitWise service",
                Some("limitwise setup"),
            )),
        );
    }
    let os_arguments = arguments
        .iter()
        .map(|argument| OsStr::new(*argument))
        .collect::<Vec<_>>();
    match run_command(
        manager_binary,
        &os_arguments,
        context.command_timeout,
        &context.command_environment,
    ) {
        Ok(output) if output.status.success() => check(
            "service.state",
            DiagnosticStatus::Pass,
            "Installed background service is active",
            [evidence("manager", manager), evidence("state", "active")],
            None,
        ),
        Ok(output) if service_query_is_blocked(&output.stderr) => check(
            "service.state",
            DiagnosticStatus::Warn,
            "Background service state cannot be verified in this execution context",
            [
                evidence("manager", manager),
                evidence("state", "unavailable"),
                evidence("exit_status", &status_label(output.status)),
            ],
            Some(remediation(
                "Run doctor directly in a user terminal to verify the service",
                Some("limitwise doctor"),
            )),
        ),
        Ok(output) => check(
            "service.state",
            DiagnosticStatus::Fail,
            "Installed background service is stopped or failed",
            [
                evidence("manager", manager),
                evidence("state", "stopped"),
                evidence("exit_status", &status_label(output.status)),
            ],
            Some(remediation(
                "Inspect the service, then reinstall it if the definition is stale",
                None,
            )),
        ),
        Err(CommandFailure::TimedOut) => check(
            "service.state",
            DiagnosticStatus::Fail,
            "Installed background service query timed out",
            [evidence("manager", manager), evidence("state", "unknown")],
            Some(remediation("Inspect the service manager directly", None)),
        ),
        Err(_) => check(
            "service.state",
            DiagnosticStatus::Fail,
            "Installed background service cannot be queried",
            [evidence("manager", manager), evidence("state", "unknown")],
            Some(remediation("Inspect the service manager directly", None)),
        ),
    }
}

fn service_query_is_blocked(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "failed to connect to bus",
        "operation not permitted",
        "permission denied",
        "access denied",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

fn check_daemon_log(context: &DiagnosticContext) -> DiagnosticCheck {
    let path = context.paths.logs_dir.join("daemon.stderr.log");
    let contents = match read_tail(&path, MAX_LOG_BYTES) {
        Ok(Some(contents)) => contents,
        Ok(None) => {
            return check(
                "logs.daemon_errors",
                DiagnosticStatus::Pass,
                "No daemon stderr log exists",
                [evidence("path", "$DATA/logs/daemon.stderr.log")],
                None,
            )
        }
        Err(_) => {
            return check(
                "logs.daemon_errors",
                DiagnosticStatus::Fail,
                "Daemon stderr log is unreadable",
                [evidence("path", "$DATA/logs/daemon.stderr.log")],
                Some(remediation(
                    "Restore private read access to the log file",
                    None,
                )),
            )
        }
    };
    let categories = summarize_log_categories(&contents);
    check(
        "logs.daemon_errors",
        if categories.is_empty() {
            DiagnosticStatus::Pass
        } else {
            DiagnosticStatus::Warn
        },
        if categories.is_empty() {
            "No recognized errors appear in the bounded daemon log tail"
        } else {
            "Recent daemon errors were summarized without raw log content"
        },
        [
            evidence("path", "$DATA/logs/daemon.stderr.log"),
            evidence("bytes_inspected", &contents.len().to_string()),
            evidence(
                "categories",
                &categories.into_iter().collect::<Vec<_>>().join(","),
            ),
        ],
        None,
    )
}

fn table_columns(connection: &Connection, table: &str) -> Result<BTreeSet<String>, String> {
    let sql = format!("PRAGMA table_info({table})");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    if columns.is_empty() {
        Err("table missing".to_string())
    } else {
        Ok(columns)
    }
}

fn inspect_private_directory(
    path: &Path,
    label: &str,
    missing: &mut Vec<String>,
    problems: &mut Vec<String>,
) {
    match fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                problems.push(format!("{label} path is not a directory"));
            }
            problems.extend(
                private_directory_problems(&metadata)
                    .into_iter()
                    .map(|problem| format!("{label} {problem}")),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            missing.push(label.to_string())
        }
        Err(_) => problems.push(format!("{label} metadata unreadable")),
    }
}

#[cfg(unix)]
fn private_directory_problems(metadata: &fs::Metadata) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut problems = Vec::new();
    if metadata.mode() & 0o777 != 0o700 {
        problems.push(format!(
            "mode {:04o} is not the required 0700",
            metadata.mode() & 0o7777
        ));
    }
    if metadata.uid() != current_uid() {
        problems.push("owner differs from the current user".to_string());
    }
    problems
}

#[cfg(not(unix))]
fn private_directory_problems(_metadata: &fs::Metadata) -> Vec<String> {
    Vec::new()
}

#[cfg(unix)]
fn private_file_problems(metadata: &fs::Metadata, allow_execute: bool) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut problems = Vec::new();
    let required = if allow_execute { 0o700 } else { 0o600 };
    if !metadata.is_file() {
        problems.push("is not a regular file".to_string());
    }
    if metadata.mode() & 0o777 != required {
        problems.push(format!(
            "mode {:04o} is not the required {required:04o}",
            metadata.mode() & 0o7777
        ));
    }
    if metadata.uid() != current_uid() {
        problems.push("owner differs from the current user".to_string());
    }
    problems
}

#[cfg(not(unix))]
fn private_file_problems(_metadata: &fs::Metadata, _allow_execute: bool) -> Vec<String> {
    Vec::new()
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}

#[cfg(unix)]
fn owner_label(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() == current_uid() {
        "current_user".to_string()
    } else if metadata.uid() == 0 {
        "root".to_string()
    } else {
        "other".to_string()
    }
}

#[cfg(not(unix))]
fn owner_label(_metadata: &fs::Metadata) -> String {
    "unavailable".to_string()
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandFailure {
    Start,
    TimedOut,
    Wait,
}

fn run_codex(
    context: &DiagnosticContext,
    arguments: &[&str],
) -> Result<CommandOutput, CommandFailure> {
    let arguments = arguments
        .iter()
        .map(|argument| OsStr::new(*argument))
        .collect::<Vec<_>>();
    run_command(
        &context.codex,
        &arguments,
        context.command_timeout,
        &context.command_environment,
    )
}

fn run_command(
    binary: &Path,
    arguments: &[&OsStr],
    timeout: Duration,
    environment: &[(OsString, OsString)],
) -> Result<CommandOutput, CommandFailure> {
    let mut child = Command::new(binary)
        .args(arguments)
        .envs(environment.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| CommandFailure::Start)?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|_| CommandFailure::Wait)?;
                return Ok(CommandOutput {
                    status: output.status,
                    stdout: bounded_utf8(&output.stdout),
                    stderr: bounded_utf8(&output.stderr),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CommandFailure::TimedOut);
            }
            Err(_) => return Err(CommandFailure::Wait),
        }
    }
}

fn bounded_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_COMMAND_OUTPUT)]).into_owned()
}

fn parsed_version(output: &str) -> Option<String> {
    let line = output.lines().find(|line| !line.trim().is_empty())?.trim();
    if line.len() > 200 || !line.chars().any(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(redact(line))
}

fn status_label(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

fn quota_error_category(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("timed out") {
        "timeout"
    } else if lower.contains("ambiguous") {
        "ambiguous"
    } else if lower.contains("missing") || lower.contains("no result") {
        "missing"
    } else if lower.contains("start") {
        "unavailable"
    } else {
        "invalid"
    }
    .to_string()
}

fn read_tail(path: &Path, maximum: u64) -> Result<Option<String>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("log unavailable".to_string()),
    };
    let length = file
        .metadata()
        .map_err(|_| "log metadata unavailable".to_string())?
        .len();
    let start = length.saturating_sub(maximum);
    file.seek(SeekFrom::Start(start))
        .map_err(|_| "log seek failed".to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| "log read failed".to_string())?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn summarize_log_categories(contents: &str) -> BTreeSet<&'static str> {
    let mut categories = BTreeSet::new();
    for line in contents.lines().rev().take(200) {
        let lower = line.to_ascii_lowercase();
        if !(lower.contains("error")
            || lower.contains("fail")
            || lower.contains("denied")
            || lower.contains("timeout"))
        {
            continue;
        }
        if lower.contains("login") || lower.contains("auth") || lower.contains("credential") {
            categories.insert("authentication");
        } else if lower.contains("quota") || lower.contains("rate limit") {
            categories.insert("quota");
        } else if lower.contains("sqlite") || lower.contains("database") {
            categories.insert("database");
        } else if lower.contains("permission") || lower.contains("denied") {
            categories.insert("permission");
        } else if lower.contains("codex") {
            categories.insert("codex_process");
        } else {
            categories.insert("other");
        }
    }
    categories
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn immutable_sqlite_uri(path: &Path) -> String {
    use std::fmt::Write as _;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    #[cfg(unix)]
    let bytes = path.as_os_str().as_bytes().to_vec();
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().as_bytes().to_vec();

    let mut uri = String::from("file:");
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-' | b'~') {
            uri.push(char::from(byte));
        } else {
            write!(uri, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    uri
}

fn redacted_path(path: &Path, home: &Path) -> String {
    path.strip_prefix(home)
        .map(|relative| format!("$HOME/{}", relative.display()))
        .unwrap_or_else(|_| path.display().to_string())
}

fn list_or_none(values: &[impl AsRef<str>]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values
            .iter()
            .map(|value| redact(value.as_ref()))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn check<const N: usize>(
    id: &str,
    status: DiagnosticStatus,
    summary: &str,
    evidence_items: [DiagnosticEvidence; N],
    remediation_value: Option<DiagnosticRemediation>,
) -> DiagnosticCheck {
    DiagnosticCheck {
        id: id.to_string(),
        status,
        summary: redact(summary),
        evidence: evidence_items.into_iter().collect(),
        remediation: remediation_value,
    }
}

fn evidence(label: &str, value: &str) -> DiagnosticEvidence {
    DiagnosticEvidence {
        label: label.to_string(),
        value: redact(value),
    }
}

fn remediation(summary: &str, command: Option<&str>) -> DiagnosticRemediation {
    DiagnosticRemediation {
        summary: redact(summary),
        command: command.map(redact),
    }
}

fn redact(value: &str) -> String {
    let compact = value.replace(['\r', '\n'], " ");
    let lower = compact.to_ascii_lowercase();
    if lower.contains("prompt:")
        || lower.contains("prompt=")
        || lower.contains("transcript:")
        || lower.contains("transcript=")
    {
        return "[REDACTED]".to_string();
    }
    let mut words = Vec::new();
    let mut redact_next = false;
    for word in compact.split_whitespace() {
        let lowered = word.to_ascii_lowercase();
        if redact_next {
            if lowered == "bearer" {
                words.push(word.to_string());
            } else {
                words.push("[REDACTED]".to_string());
                redact_next = false;
            }
        } else if matches!(lowered.as_str(), "bearer" | "authorization:") {
            words.push(word.to_string());
            redact_next = true;
        } else if ["token=", "password=", "secret=", "api_key=", "apikey="]
            .iter()
            .any(|prefix| lowered.starts_with(*prefix))
            || lowered.starts_with("sk-")
        {
            let key = word.split_once('=').map(|(key, _)| key).unwrap_or("secret");
            words.push(format!("{key}=[REDACTED]"));
        } else {
            words.push(word.to_string());
        }
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_check(id: &str, status: DiagnosticStatus) -> DiagnosticCheck {
        check(id, status, "summary", [], None)
    }

    fn sample_report(status: DiagnosticStatus) -> DiagnosticReport {
        DiagnosticReport::new(
            1_700_000_000,
            DiagnosticPlatform {
                os: "linux".to_string(),
                architecture: "x86_64".to_string(),
            },
            vec![sample_check("sample.check", status)],
        )
    }

    #[test]
    fn diagnostic_contract_serialization_snapshot() {
        let report = DiagnosticReport::new(
            1_700_000_000,
            DiagnosticPlatform {
                os: "linux".to_string(),
                architecture: "x86_64".to_string(),
            },
            vec![check(
                "codex.login",
                DiagnosticStatus::Fail,
                "Codex is not signed in",
                [evidence("exit_status", "1")],
                Some(remediation("Sign in to Codex", Some("codex login"))),
            )],
        );
        assert_eq!(
            serde_json::to_string_pretty(&report).unwrap(),
            r#"{
  "schema_version": "1",
  "overall": "fail",
  "generated_at": 1700000000,
  "platform": {
    "os": "linux",
    "architecture": "x86_64"
  },
  "checks": [
    {
      "id": "codex.login",
      "status": "fail",
      "summary": "Codex is not signed in",
      "evidence": [
        {
          "label": "exit_status",
          "value": "1"
        }
      ],
      "remediation": {
        "summary": "Sign in to Codex",
        "command": "codex login"
      }
    }
  ]
}"#
        );
    }

    #[test]
    fn aggregate_uses_fail_then_warn_then_pass() {
        assert_eq!(aggregate_status(&[]), DiagnosticStatus::Pass);
        assert_eq!(
            aggregate_status(&[sample_check("warn", DiagnosticStatus::Warn)]),
            DiagnosticStatus::Warn
        );
        assert_eq!(
            aggregate_status(&[
                sample_check("warn", DiagnosticStatus::Warn),
                sample_check("fail", DiagnosticStatus::Fail),
            ]),
            DiagnosticStatus::Fail
        );
    }

    #[test]
    fn redacts_secrets_prompts_transcripts_and_bearer_tokens() {
        assert_eq!(
            redact("Authorization: Bearer secret-value"),
            "Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            redact("token=abc password=def sk-secret"),
            "token=[REDACTED] password=[REDACTED] secret=[REDACTED]"
        );
        assert_eq!(redact("prompt: do private work"), "[REDACTED]");
        assert_eq!(redact("transcript=/private/content"), "[REDACTED]");
    }

    #[test]
    fn plugin_inventory_requires_exact_enabled_install() {
        assert!(plugin_inventory_has_enabled_limitwise(
            r#"{"installed":[{"pluginId":"limitwise@limitwise","installed":true,"enabled":true}]}"#
        ));
        assert!(!plugin_inventory_has_enabled_limitwise(
            r#"{"installed":[{"pluginId":"limitwise@personal","installed":false,"enabled":false}]}"#
        ));
        assert!(!plugin_inventory_has_enabled_limitwise(
            r#"{"installed":[{"pluginId":"limitwise@limitwise","installed":true,"enabled":false}]}"#
        ));
    }

    #[test]
    fn human_output_golden_and_exit_codes() {
        let mut human = Vec::new();
        let code = run_cli_with(&[], &mut human, || {
            Ok(sample_report(DiagnosticStatus::Warn))
        })
        .unwrap();
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(human).unwrap(),
            "LimitWise doctor: WARN\nSchema: 1\nGenerated: 1700000000\nPlatform: linux/x86_64\nWARN sample.check: summary\n"
        );
        let mut failed = Vec::new();
        assert_eq!(
            run_cli_with(&[], &mut failed, || Ok(sample_report(
                DiagnosticStatus::Fail
            )))
            .unwrap(),
            1
        );
        let mut passed = Vec::new();
        assert_eq!(
            run_cli_with(&[], &mut passed, || Ok(sample_report(
                DiagnosticStatus::Pass
            )))
            .unwrap(),
            0
        );
        assert!(run_cli_with(&["--bad".to_string()], &mut Vec::new(), || {
            Ok(sample_report(DiagnosticStatus::Pass))
        })
        .is_err());
        assert!(
            run_cli_with(&[], &mut Vec::new(), || Err("execution failed".to_string())).is_err()
        );
    }

    #[test]
    fn json_output_is_only_the_report() {
        let mut output = Vec::new();
        let report = sample_report(DiagnosticStatus::Pass);
        assert_eq!(
            run_cli_with(&["--json".to_string()], &mut output, || Ok(report.clone())).unwrap(),
            0
        );
        assert_eq!(
            serde_json::from_slice::<DiagnosticReport>(&output).unwrap(),
            report
        );
    }

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "limitwise-doctor-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn fixture_context(root: &Path, scenario: &str) -> DiagnosticContext {
        let data = root.join("data");
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-codex.sh");
        DiagnosticContext {
            paths: Paths {
                database: data.join("limitwise.sqlite3"),
                logs_dir: data.join("logs"),
                installed_binary: data.join("bin/limitwise"),
                data_dir: data,
            },
            home: root.join("home"),
            current_exe: fixture.clone(),
            codex: fixture,
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            generated_at: 1_700_000_000,
            command_timeout: Duration::from_millis(200),
            quota_timeout: Duration::from_millis(200),
            command_environment: vec![(
                OsString::from("LIMITWISE_DOCTOR_SCENARIO"),
                OsString::from(scenario),
            )],
            systemctl: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/bin/systemctl"),
            launchctl: PathBuf::from("launchctl"),
        }
    }

    fn find_check<'a>(report: &'a DiagnosticReport, id: &str) -> &'a DiagnosticCheck {
        report.checks.iter().find(|check| check.id == id).unwrap()
    }

    #[test]
    fn fake_process_fixtures_cover_required_codex_failures() {
        let root = temporary_root("commands");
        fs::create_dir_all(&root).unwrap();
        for (scenario, check_id) in [
            ("success", "codex.executable"),
            ("logged_out", "codex.login"),
            ("malformed_version", "codex.executable"),
            ("plugin_missing", "plugin.installed"),
            ("mcp_missing", "plugin.mcp_configuration"),
            ("quota_timeout", "codex.quota"),
            ("quota_ambiguous", "codex.quota"),
        ] {
            let report = snapshot_with(&fixture_context(&root, scenario)).unwrap();
            let status = find_check(&report, check_id).status;
            if scenario == "success" {
                assert_eq!(status, DiagnosticStatus::Pass);
            } else {
                assert_eq!(status, DiagnosticStatus::Fail, "scenario {scenario}");
            }
        }
        let mut missing = fixture_context(&root, "success");
        missing.codex = root.join("missing-codex");
        let report = snapshot_with(&missing).unwrap();
        assert_eq!(
            find_check(&report, "codex.executable").status,
            DiagnosticStatus::Fail
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn linux_and_arm_macos_reports_use_equivalent_check_ids() {
        let root = temporary_root("platforms");
        fs::create_dir_all(&root).unwrap();
        let linux = snapshot_with(&fixture_context(&root, "success")).unwrap();
        let mut mac_context = fixture_context(&root, "success");
        mac_context.os = "macos".to_string();
        mac_context.architecture = "aarch64".to_string();
        let mac = snapshot_with(&mac_context).unwrap();
        assert_eq!(
            linux
                .checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            mac.checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            find_check(&linux, "platform.supported").status,
            DiagnosticStatus::Pass
        );
        assert_eq!(
            find_check(&mac, "platform.supported").status,
            DiagnosticStatus::Warn
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn create_legacy_database(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE batches (id TEXT PRIMARY KEY,idempotency_key TEXT,cap_percent REAL,created_at INTEGER);
                 CREATE TABLE tasks (id TEXT PRIMARY KEY,batch_id TEXT,title TEXT,prompt TEXT,success_criteria TEXT,cwd TEXT,run_at INTEGER,timezone TEXT,difficulty TEXT,model TEXT,effort TEXT,status TEXT,created_at INTEGER,updated_at INTEGER);
                 CREATE TABLE runs (id TEXT PRIMARY KEY,task_id TEXT,started_at INTEGER,status TEXT);",
            )
            .unwrap();
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}

    #[derive(Debug, PartialEq, Eq)]
    struct FileState {
        path: PathBuf,
        length: u64,
        mode: u32,
        modified_nanos: u128,
        changed: (i64, i64),
        contents: Vec<u8>,
    }

    #[cfg(unix)]
    fn metadata_mode_and_change(metadata: &fs::Metadata) -> (u32, (i64, i64)) {
        use std::os::unix::fs::MetadataExt;
        (metadata.mode(), (metadata.ctime(), metadata.ctime_nsec()))
    }

    #[cfg(not(unix))]
    fn metadata_mode_and_change(_metadata: &fs::Metadata) -> (u32, (i64, i64)) {
        (0, (0, 0))
    }

    fn tree_state(root: &Path) -> Vec<FileState> {
        fn visit(root: &Path, path: &Path, output: &mut Vec<FileState>) {
            let mut entries = fs::read_dir(path)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                let metadata = fs::metadata(&path).unwrap();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                let (mode, changed) = metadata_mode_and_change(&metadata);
                let modified_nanos = metadata
                    .modified()
                    .unwrap()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let contents = if metadata.is_file() {
                    fs::read(&path).unwrap()
                } else {
                    Vec::new()
                };
                output.push(FileState {
                    path: relative,
                    length: metadata.len(),
                    mode,
                    modified_nanos,
                    changed,
                    contents,
                });
                if metadata.is_dir() {
                    visit(root, &path, output);
                }
            }
        }
        let mut output = Vec::new();
        visit(root, root, &mut output);
        output
    }

    #[test]
    fn diagnostics_do_not_mutate_filesystem_or_database() {
        let root = temporary_root("nonmutation");
        let context = fixture_context(&root, "success");
        fs::create_dir_all(&context.paths.logs_dir).unwrap();
        fs::create_dir_all(context.paths.installed_binary.parent().unwrap()).unwrap();
        set_mode(&context.paths.data_dir, 0o700);
        set_mode(&context.paths.logs_dir, 0o700);
        create_legacy_database(&context.paths.database);
        set_mode(&context.paths.database, 0o600);
        let before = tree_state(&root);
        let report = snapshot_with(&context).unwrap();
        let after = tree_state(&root);
        assert_eq!(
            find_check(&report, "storage.database").status,
            DiagnosticStatus::Warn
        );
        assert_eq!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_fixtures_cover_absent_legacy_corrupt_and_wrong_modes() {
        let root = temporary_root("storage");
        let context = fixture_context(&root, "success");
        fs::create_dir_all(&context.paths.logs_dir).unwrap();
        set_mode(&context.paths.data_dir, 0o700);
        set_mode(&context.paths.logs_dir, 0o700);
        assert_eq!(check_database(&context).status, DiagnosticStatus::Warn);
        create_legacy_database(&context.paths.database);
        set_mode(&context.paths.database, 0o600);
        assert_eq!(check_database(&context).status, DiagnosticStatus::Warn);
        fs::write(&context.paths.database, b"not sqlite").unwrap();
        assert_eq!(check_database(&context).status, DiagnosticStatus::Fail);
        fs::remove_file(&context.paths.database).unwrap();
        create_legacy_database(&context.paths.database);
        set_mode(&context.paths.data_dir, 0o755);
        assert_eq!(check_storage_paths(&context).status, DiagnosticStatus::Fail);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn service_fixtures_cover_stopped_and_stale_definition() {
        let root = temporary_root("service");
        let mut context = fixture_context(&root, "service_stopped");
        let unit = context.home.join(".config/systemd/user/limitwise.service");
        fs::create_dir_all(unit.parent().unwrap()).unwrap();
        fs::write(
            &unit,
            format!(
                "ExecStart={} daemon\n",
                context.paths.installed_binary.display()
            ),
        )
        .unwrap();
        assert_eq!(check_service(&context).status, DiagnosticStatus::Fail);
        context.command_environment = vec![(
            OsString::from("LIMITWISE_DOCTOR_SCENARIO"),
            OsString::from("service_unavailable"),
        )];
        let unavailable = check_service(&context);
        assert_eq!(unavailable.status, DiagnosticStatus::Warn);
        assert!(unavailable
            .evidence
            .iter()
            .any(|item| item.value == "unavailable"));
        context.command_environment = vec![];
        fs::write(&unit, "ExecStart=/stale/limitwise daemon\n").unwrap();
        let stale = check_service(&context);
        assert_eq!(stale.status, DiagnosticStatus::Fail);
        assert!(stale
            .evidence
            .iter()
            .any(|item| item.value == "definition_mismatch"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn noisy_log_is_bounded_categorized_and_redacted() {
        let root = temporary_root("log");
        let context = fixture_context(&root, "success");
        fs::create_dir_all(&context.paths.logs_dir).unwrap();
        fs::write(
            context.paths.logs_dir.join("daemon.stderr.log"),
            "authentication failed bearer super-secret\nquota error prompt: private work\n",
        )
        .unwrap();
        let result = check_daemon_log(&context);
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(result.status, DiagnosticStatus::Warn);
        assert!(serialized.contains("authentication,quota"));
        assert!(!serialized.contains("super-secret"));
        assert!(!serialized.contains("private work"));
        fs::remove_dir_all(root).unwrap();
    }
}
