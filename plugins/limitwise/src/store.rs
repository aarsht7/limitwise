use crate::config::{set_private_file, Paths};
use crate::model::{route, validate_route, Difficulty, PermissionProfile};
use crate::transcript::token_usage;
use crate::usage::UsageSnapshot;
use chrono::{DateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);
pub const TASK_PAGE_SIZE: i64 = 20;
pub const BATCH_PAGE_SIZE: i64 = 20;
pub const BATCH_EDIT_LEASE_SECONDS: i64 = 60;

#[cfg(test)]
pub const TERMINAL_STATUSES: &[&str] = &[
    "completed",
    "failed",
    "quota_skipped",
    "quota_interrupted",
    "missed",
    "blocked",
    "cancelled",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraft {
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub success_criteria: String,
    pub cwd: String,
    #[serde(default)]
    pub run_at: Option<String>,
    #[serde(default)]
    pub after_previous: bool,
    #[serde(default)]
    pub continue_from_task_id: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    pub difficulty: Difficulty,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub permission_profile: PermissionProfile,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEditTaskInput {
    #[serde(default)]
    pub task_id: Option<String>,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub success_criteria: String,
    pub cwd: String,
    #[serde(default)]
    pub run_at: Option<String>,
    pub timezone: String,
    pub difficulty: Difficulty,
    pub model: String,
    pub effort: String,
    #[serde(default)]
    pub permission_profile: PermissionProfile,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEditInput {
    pub edit_session_id: String,
    #[serde(default)]
    pub networked_confirmed: bool,
    pub tasks: Vec<BatchEditTaskInput>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEditSessionInput {
    pub edit_session_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleBatchInput {
    pub idempotency_key: String,
    #[serde(default)]
    pub budget_mode: Option<BudgetMode>,
    #[serde(default)]
    pub weekly_cap_percent: Option<f64>,
    #[serde(default)]
    pub token_cap: Option<i64>,
    #[serde(default)]
    pub cap_percent: Option<f64>,
    #[serde(default)]
    pub five_hour_cap_percent: Option<f64>,
    #[serde(default)]
    pub networked_confirmed: bool,
    pub tasks: Vec<TaskDraft>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetMode {
    Percentage,
    Tokens,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    Retry,
    QuotaResume,
}

impl fmt::Display for AttemptKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Retry => "retry",
            Self::QuotaResume => "quota_resume",
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryTaskOptions {
    pub budget_mode: BudgetMode,
    #[serde(default)]
    pub weekly_cap_percent: Option<f64>,
    #[serde(default)]
    pub token_cap: Option<i64>,
    #[serde(default)]
    pub five_hour_cap_percent: Option<f64>,
    #[serde(default)]
    pub run_at: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub permission_profile: Option<PermissionProfile>,
    #[serde(default)]
    pub networked_confirmed: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedRetryTaskInput {
    pub idempotency_key: String,
    pub confirmed: bool,
    #[serde(flatten)]
    pub options: RetryTaskOptions,
}

impl fmt::Display for BudgetMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Percentage => "percentage",
            Self::Tokens => "tokens",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TaskUpdate {
    pub title: Option<String>,
    pub prompt: Option<String>,
    pub success_criteria: Option<String>,
    pub cwd: Option<String>,
    pub run_at: Option<String>,
    pub timezone: Option<String>,
    pub difficulty: Option<Difficulty>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_profile: Option<PermissionProfile>,
    #[serde(default)]
    pub networked_confirmed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Batch {
    pub id: String,
    pub idempotency_key: String,
    pub budget_mode: String,
    pub weekly_cap_percent: Option<f64>,
    pub token_cap: Option<i64>,
    pub consumed_tokens: i64,
    pub cap_percent: f64,
    pub cap_basis: String,
    pub created_at: i64,
    pub window_reset_at: Option<i64>,
    pub baseline_weekly_used_percent: Option<f64>,
    pub allowance_points: f64,
    pub consumed_points: f64,
    pub five_hour_cap_percent: Option<f64>,
    pub five_hour_window_reset_at: Option<i64>,
    pub baseline_five_hour_used_percent: Option<f64>,
    pub five_hour_allowance_points: f64,
    pub five_hour_consumed_points: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Task {
    pub id: String,
    pub batch_id: String,
    pub title: String,
    pub prompt: String,
    pub success_criteria: String,
    pub cwd: String,
    pub run_at: i64,
    pub run_at_iso: String,
    pub position: i64,
    pub depends_on_task_id: Option<String>,
    pub dependency_type: String,
    pub source_task_id: Option<String>,
    pub attempt_kind: Option<String>,
    pub attempt_number: i64,
    pub timezone: String,
    pub difficulty: String,
    pub model: String,
    pub effort: String,
    pub permission_profile: PermissionProfile,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskPage {
    pub items: Vec<Task>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub total_pages: i64,
    pub sort: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchGroup {
    pub batch: Batch,
    pub tasks: Vec<Task>,
    pub editable: bool,
    pub editing: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchPage {
    pub items: Vec<BatchGroup>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub total_pages: i64,
    pub sort: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchEditSession {
    pub edit_session_id: String,
    pub expires_at: i64,
    pub batch: Batch,
    pub tasks: Vec<Task>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArchiveTaskResult {
    pub task_id: String,
    pub archived_at: i64,
    pub status: String,
    pub preserved_runs: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskSort {
    Newest,
    Oldest,
    RunAtNewest,
    RunAtOldest,
    TitleAscending,
    TitleDescending,
}

impl FromStr for TaskSort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "newest" => Ok(Self::Newest),
            "oldest" => Ok(Self::Oldest),
            "run_at_newest" => Ok(Self::RunAtNewest),
            "run_at_oldest" => Ok(Self::RunAtOldest),
            "title_ascending" => Ok(Self::TitleAscending),
            "title_descending" => Ok(Self::TitleDescending),
            _ => Err("sort must be newest, oldest, run_at_newest, run_at_oldest, title_ascending, or title_descending".to_string()),
        }
    }
}

impl fmt::Display for TaskSort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::RunAtNewest => "run_at_newest",
            Self::RunAtOldest => "run_at_oldest",
            Self::TitleAscending => "title_ascending",
            Self::TitleDescending => "title_descending",
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ScheduleBatchResult {
    pub batch: Batch,
    pub tasks: Vec<Task>,
    pub idempotent_replay: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub usage_before_json: Option<String>,
    pub usage_after_json: Option<String>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub tokens_used: Option<i64>,
    pub token_usage_state: String,
    pub error: Option<String>,
}

pub struct RunFinish<'a> {
    pub status: &'a str,
    pub usage_json: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub transcript: Option<&'a str>,
    pub tokens_used: Option<i64>,
    pub error: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub(crate) struct ContinuationContext {
    pub predecessor_task_id: String,
    pub predecessor_title: String,
    pub predecessor_prompt: String,
    pub predecessor_success_criteria: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub transcript_excerpt: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskStatus {
    pub task: Task,
    pub batch: Batch,
    pub runs: Vec<RunRecord>,
    pub lineage: Vec<Task>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttemptBudgetPreview {
    pub budget_mode: String,
    pub weekly_cap_percent: Option<f64>,
    pub token_cap: Option<i64>,
    pub five_hour_cap_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttemptTaskPreview {
    pub title: String,
    pub prompt: String,
    pub success_criteria: String,
    pub cwd: String,
    pub timezone: String,
    pub difficulty: String,
    pub model: String,
    pub effort: String,
    pub permission_profile: PermissionProfile,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttemptPreview {
    pub source_task_id: String,
    pub attempt_kind: String,
    pub action_label: String,
    pub attempt_number: i64,
    pub run_at: i64,
    pub run_at_iso: String,
    pub provider_reset_at: Option<i64>,
    pub resume_mode: String,
    pub session_available: bool,
    pub task: AttemptTaskPreview,
    pub budget: AttemptBudgetPreview,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttemptCreateResult {
    pub batch: Batch,
    pub task: Task,
    pub idempotent_replay: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsagePeriodStats {
    pub since: i64,
    pub run_count: i64,
    pub tokens_used: i64,
    pub tokens_unavailable_runs: i64,
    pub status_counts: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DailyUsageStats {
    pub date: String,
    pub run_count: i64,
    pub tokens_used: i64,
    pub tokens_unavailable_runs: i64,
    pub status_counts: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndividualRunUsage {
    pub run_id: String,
    pub task_id: String,
    pub task_title: String,
    pub status: String,
    pub started_at: i64,
    pub started_at_iso: String,
    pub finished_at: Option<i64>,
    pub finished_at_iso: Option<String>,
    pub budget_mode: String,
    pub model: String,
    pub effort: String,
    pub permission_profile: PermissionProfile,
    pub tokens_used: Option<i64>,
    pub token_usage_state: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskUsageStats {
    pub generated_at: i64,
    pub timezone: String,
    pub last_year: UsagePeriodStats,
    pub last_month: UsagePeriodStats,
    pub last_week: UsagePeriodStats,
    pub daily_last_7_days: Vec<DailyUsageStats>,
    pub individual_runs_last_7_days: Vec<IndividualRunUsage>,
}

#[derive(Clone, Debug)]
pub(crate) struct HistoricalUsageSample {
    pub difficulty: String,
    pub model: String,
    pub effort: String,
    pub tokens_used: i64,
    pub usage_before_json: Option<String>,
    pub usage_after_json: Option<String>,
}

#[derive(Clone, Debug)]
struct StoredUsageRun {
    run_id: String,
    task_id: String,
    task_title: String,
    status: String,
    started_at: i64,
    finished_at: Option<i64>,
    budget_mode: String,
    model: String,
    effort: String,
    permission_profile: PermissionProfile,
    tokens_used: Option<i64>,
    token_usage_state: String,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open() -> Result<Self, String> {
        let paths = Paths::discover()?;
        paths.ensure()?;
        let connection = Connection::open(&paths.database).map_err(|e| e.to_string())?;
        let store = Self::from_connection(connection)?;
        set_private_file(&paths.database)?;
        Ok(store)
    }

    fn from_connection(connection: Connection) -> Result<Self, String> {
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS batches (
                   id TEXT PRIMARY KEY,
                   idempotency_key TEXT NOT NULL UNIQUE,
                   cap_percent REAL NOT NULL,
                   cap_basis TEXT NOT NULL DEFAULT 'remaining_percent',
                   budget_mode TEXT NOT NULL DEFAULT 'percentage',
                   token_cap INTEGER,
                   consumed_tokens INTEGER NOT NULL DEFAULT 0,
                   created_at INTEGER NOT NULL,
                   window_reset_at INTEGER,
                   baseline_weekly_used_percent REAL,
                   allowance_points REAL NOT NULL DEFAULT 0,
                   consumed_points REAL NOT NULL DEFAULT 0,
                   five_hour_cap_percent REAL,
                   five_hour_window_reset_at INTEGER,
                   baseline_five_hour_used_percent REAL,
                   five_hour_allowance_points REAL NOT NULL DEFAULT 0,
                   five_hour_consumed_points REAL NOT NULL DEFAULT 0,
                   edit_session_id TEXT,
                   edit_heartbeat_at INTEGER
                 );
                 CREATE TABLE IF NOT EXISTS tasks (
                   id TEXT PRIMARY KEY,
                   batch_id TEXT NOT NULL REFERENCES batches(id),
                   title TEXT NOT NULL,
                   prompt TEXT NOT NULL,
                   success_criteria TEXT NOT NULL,
                   cwd TEXT NOT NULL,
                   run_at INTEGER NOT NULL,
                   timezone TEXT NOT NULL,
                   difficulty TEXT NOT NULL,
                   model TEXT NOT NULL,
                   effort TEXT NOT NULL,
                   status TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL,
                   last_error TEXT,
                   position INTEGER NOT NULL DEFAULT 0,
                   depends_on_task_id TEXT REFERENCES tasks(id),
                   dependency_type TEXT NOT NULL DEFAULT 'success',
                   source_task_id TEXT REFERENCES tasks(id),
                   attempt_kind TEXT,
                   attempt_number INTEGER NOT NULL DEFAULT 1,
                   archived_at INTEGER,
                   permission_profile TEXT NOT NULL DEFAULT 'restricted'
                     CHECK(permission_profile IN ('restricted','networked'))
                 );
                 CREATE TABLE IF NOT EXISTS runs (
                   id TEXT PRIMARY KEY,
                   task_id TEXT NOT NULL REFERENCES tasks(id),
                   started_at INTEGER NOT NULL,
                   finished_at INTEGER,
                   status TEXT NOT NULL,
                   usage_before_json TEXT,
                   usage_after_json TEXT,
                   session_id TEXT,
                   transcript_path TEXT,
                   tokens_used INTEGER,
                   token_usage_state TEXT NOT NULL DEFAULT 'pending',
                   error TEXT
                 );
                 CREATE TABLE IF NOT EXISTS task_stop_requests (
                   task_id TEXT PRIMARY KEY REFERENCES tasks(id),
                   requested_at INTEGER NOT NULL
                 );",
            )
            .map_err(|e| e.to_string())?;
        add_column_if_missing(
            &connection,
            "batches",
            "cap_basis",
            "TEXT NOT NULL DEFAULT 'remaining_percent'",
        )?;
        add_column_if_missing(
            &connection,
            "batches",
            "budget_mode",
            "TEXT NOT NULL DEFAULT 'percentage'",
        )?;
        add_column_if_missing(&connection, "batches", "token_cap", "INTEGER")?;
        add_column_if_missing(
            &connection,
            "batches",
            "consumed_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(&connection, "batches", "five_hour_cap_percent", "REAL")?;
        add_column_if_missing(
            &connection,
            "batches",
            "five_hour_window_reset_at",
            "INTEGER",
        )?;
        add_column_if_missing(
            &connection,
            "batches",
            "baseline_five_hour_used_percent",
            "REAL",
        )?;
        add_column_if_missing(
            &connection,
            "batches",
            "five_hour_allowance_points",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &connection,
            "batches",
            "five_hour_consumed_points",
            "REAL NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(&connection, "batches", "edit_session_id", "TEXT")?;
        add_column_if_missing(&connection, "batches", "edit_heartbeat_at", "INTEGER")?;
        add_column_if_missing(
            &connection,
            "tasks",
            "position",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &connection,
            "tasks",
            "depends_on_task_id",
            "TEXT REFERENCES tasks(id)",
        )?;
        add_column_if_missing(
            &connection,
            "tasks",
            "dependency_type",
            "TEXT NOT NULL DEFAULT 'success'",
        )?;
        add_column_if_missing(
            &connection,
            "tasks",
            "source_task_id",
            "TEXT REFERENCES tasks(id)",
        )?;
        add_column_if_missing(&connection, "tasks", "attempt_kind", "TEXT")?;
        add_column_if_missing(
            &connection,
            "tasks",
            "attempt_number",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        add_column_if_missing(&connection, "tasks", "archived_at", "INTEGER")?;
        add_column_if_missing(
            &connection,
            "tasks",
            "permission_profile",
            "TEXT NOT NULL DEFAULT 'restricted' CHECK(permission_profile IN ('restricted','networked'))",
        )?;
        connection
            .execute(
                "UPDATE tasks SET attempt_number=1 WHERE attempt_number IS NULL OR attempt_number<1",
                [],
            )
            .map_err(|e| e.to_string())?;
        add_column_if_missing(&connection, "runs", "tokens_used", "INTEGER")?;
        add_column_if_missing(
            &connection,
            "runs",
            "token_usage_state",
            "TEXT NOT NULL DEFAULT 'pending'",
        )?;
        backfill_run_token_usage(&connection)?;
        connection
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS tasks_due_idx ON tasks(status, run_at);
                 CREATE INDEX IF NOT EXISTS tasks_dependency_idx ON tasks(depends_on_task_id, status);
                 CREATE INDEX IF NOT EXISTS tasks_source_idx ON tasks(source_task_id, attempt_number);
                 CREATE INDEX IF NOT EXISTS tasks_created_idx ON tasks(created_at, id);
                 CREATE INDEX IF NOT EXISTS runs_started_idx ON runs(started_at);",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self { connection })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, String> {
        Self::from_connection(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    pub fn schedule_batch(
        &mut self,
        input: ScheduleBatchInput,
    ) -> Result<ScheduleBatchResult, String> {
        validate_batch_input(&input)?;
        for task in &input.tasks {
            require_networked_confirmation(task.permission_profile, input.networked_confirmed)?;
        }
        if let Some(batch) = self.batch_by_idempotency(&input.idempotency_key)? {
            let tasks = self.tasks_for_batch(&batch.id)?;
            return Ok(ScheduleBatchResult {
                batch,
                tasks,
                idempotent_replay: true,
            });
        }
        let budget = requested_budget(&input)?;
        let normalized = normalize_drafts(&input.tasks, |predecessor_id, cwd| {
            self.continuation_reset_at(predecessor_id, cwd)
        })?;
        let task_ids: Vec<String> = input.tasks.iter().map(|_| new_id("task")).collect();
        let now = now_epoch();
        let batch_id = new_id("batch");
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO batches
                 (id,idempotency_key,cap_percent,cap_basis,budget_mode,token_cap,created_at,five_hour_cap_percent)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    batch_id,
                    input.idempotency_key,
                    budget.cap_percent,
                    budget.cap_basis,
                    budget.mode.to_string(),
                    budget.token_cap,
                    now,
                    input.five_hour_cap_percent,
                ],
            )
            .map_err(|e| e.to_string())?;
        for (position, (draft, normalized)) in input.tasks.into_iter().zip(normalized).enumerate() {
            let (dependency, dependency_type) = if draft.after_previous {
                (Some(task_ids[position - 1].as_str()), "success")
            } else if let Some(predecessor) = draft.continue_from_task_id.as_deref() {
                (Some(predecessor), "quota_reset")
            } else {
                (None, "success")
            };
            transaction
                .execute(
                    "INSERT INTO tasks
                     (id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,position,depends_on_task_id,dependency_type,permission_profile)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'scheduled',?12,?12,?13,?14,?15,?16)",
                    params![
                        task_ids[position], batch_id, draft.title.trim(), draft.prompt.trim(),
                        draft.success_criteria.trim(), draft.cwd, normalized.run_at,
                        normalized.timezone, draft.difficulty.to_string(), normalized.model,
                        normalized.effort, now, position as i64, dependency, dependency_type,
                        draft.permission_profile.to_string()
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        let batch = self
            .batch(&batch_id)?
            .ok_or_else(|| "created batch disappeared".to_string())?;
        let tasks = self.tasks_for_batch(&batch_id)?;
        Ok(ScheduleBatchResult {
            batch,
            tasks,
            idempotent_replay: false,
        })
    }

    /// Validate a complete schedule using the same rules as persistence without
    /// creating a batch or task. HTTP uses this before showing confirmation.
    pub fn validate_schedule_batch(&self, input: &ScheduleBatchInput) -> Result<(), String> {
        validate_batch_input(input)?;
        normalize_drafts(&input.tasks, |predecessor_id, cwd| {
            self.continuation_reset_at(predecessor_id, cwd)
        })?;
        Ok(())
    }

    pub fn preview_retry_task(
        &self,
        source_task_id: &str,
        options: &RetryTaskOptions,
    ) -> Result<AttemptPreview, String> {
        let source = self
            .task(source_task_id)?
            .ok_or_else(|| "task not found".to_string())?;
        let quota_metadata = if matches!(
            source.status.as_str(),
            "quota_interrupted" | "quota_skipped"
        ) {
            self.quota_resume_metadata(source_task_id, &source.cwd).ok()
        } else {
            None
        };
        let attempt_kind = if quota_metadata.is_some() {
            AttemptKind::QuotaResume
        } else if matches!(
            source.status.as_str(),
            "quota_interrupted" | "quota_skipped"
        ) {
            AttemptKind::Retry
        } else {
            attempt_kind_for_status(&source.status)?
        };
        let budget = requested_retry_budget(options)?;
        validate_five_hour_cap(options.five_hour_cap_percent)?;

        let timezone = options
            .timezone
            .clone()
            .unwrap_or_else(|| source.timezone.clone());
        validate_timezone(&timezone)?;
        let model = options
            .model
            .clone()
            .unwrap_or_else(|| source.model.clone());
        let effort = options
            .effort
            .clone()
            .unwrap_or_else(|| source.effort.clone());
        validate_route(&model, &effort)?;

        let (run_at, provider_reset_at, session_available, resume_mode) = match attempt_kind {
            AttemptKind::Retry => {
                let value = options
                    .run_at
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "run_at is required for retry".to_string())?;
                let (run_at, explicit_offset) = parse_future_time(value)?;
                validate_timezone_at(&timezone, run_at, explicit_offset)?;
                (run_at, None, false, "fresh_session")
            }
            AttemptKind::QuotaResume => {
                if options.run_at.is_some() {
                    return Err(
                        "run_at is derived from provider reset for quota_resume".to_string()
                    );
                }
                let (reset_at, session_available) = quota_metadata
                    .expect("quota resume is selected only with validated reset metadata");
                (
                    reset_at,
                    Some(reset_at),
                    session_available,
                    if session_available {
                        "session_resume_or_context_fallback"
                    } else {
                        "context_fallback"
                    },
                )
            }
        };

        Ok(AttemptPreview {
            source_task_id: source.id,
            attempt_kind: attempt_kind.to_string(),
            action_label: match attempt_kind {
                AttemptKind::Retry => "Retry as new run",
                AttemptKind::QuotaResume => "Continue after quota reset",
            }
            .to_string(),
            attempt_number: source.attempt_number.max(1) + 1,
            run_at,
            run_at_iso: timestamp_in_timezone(run_at, &timezone),
            provider_reset_at,
            resume_mode: resume_mode.to_string(),
            session_available,
            task: AttemptTaskPreview {
                title: source.title,
                prompt: source.prompt,
                success_criteria: source.success_criteria,
                cwd: source.cwd,
                timezone,
                difficulty: source.difficulty,
                model,
                effort,
                permission_profile: options
                    .permission_profile
                    .unwrap_or(source.permission_profile),
            },
            budget: AttemptBudgetPreview {
                budget_mode: budget.mode.to_string(),
                weekly_cap_percent: (budget.mode == BudgetMode::Percentage)
                    .then_some(budget.cap_percent),
                token_cap: budget.token_cap,
                five_hour_cap_percent: options.five_hour_cap_percent,
            },
        })
    }

    pub fn create_retry_task(
        &mut self,
        source_task_id: &str,
        input: ConfirmedRetryTaskInput,
    ) -> Result<AttemptCreateResult, String> {
        if !input.confirmed {
            return Err("explicit confirmation is required".to_string());
        }
        if input.idempotency_key.trim().is_empty() {
            return Err("idempotency_key is required".to_string());
        }
        if let Some(replay) = self.attempt_by_idempotency(&input.idempotency_key, source_task_id)? {
            return Ok(replay);
        }

        let preview = self.preview_retry_task(source_task_id, &input.options)?;
        require_networked_confirmation(
            preview.task.permission_profile,
            input.options.networked_confirmed,
        )?;
        let budget = requested_retry_budget(&input.options)?;
        let batch_id = new_id("batch");
        let task_id = new_id("task");
        let now = now_epoch();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let existing_batch = transaction
            .query_row(
                "SELECT id FROM batches WHERE idempotency_key=?1",
                params![input.idempotency_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if existing_batch.is_some() {
            transaction.commit().map_err(|e| e.to_string())?;
            return self
                .attempt_by_idempotency(&input.idempotency_key, source_task_id)?
                .ok_or_else(|| "idempotent attempt disappeared".to_string());
        }

        transaction
            .execute(
                "INSERT INTO batches
                 (id,idempotency_key,cap_percent,cap_basis,budget_mode,token_cap,created_at,five_hour_cap_percent)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    batch_id,
                    input.idempotency_key,
                    budget.cap_percent,
                    budget.cap_basis,
                    budget.mode.to_string(),
                    budget.token_cap,
                    now,
                    input.options.five_hour_cap_percent,
                ],
            )
            .map_err(|e| e.to_string())?;
        let quota_resume = preview.attempt_kind == AttemptKind::QuotaResume.to_string();
        let dependency = quota_resume.then_some(source_task_id);
        let dependency_type = if quota_resume {
            "quota_reset"
        } else {
            "success"
        };
        transaction
            .execute(
                "INSERT INTO tasks
                 (id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,position,depends_on_task_id,dependency_type,source_task_id,attempt_kind,attempt_number,permission_profile)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'scheduled',?12,?12,0,?13,?14,?15,?16,?17,?18)",
                params![
                    task_id,
                    batch_id,
                    preview.task.title,
                    preview.task.prompt,
                    preview.task.success_criteria,
                    preview.task.cwd,
                    preview.run_at,
                    preview.task.timezone,
                    preview.task.difficulty,
                    preview.task.model,
                    preview.task.effort,
                    now,
                    dependency,
                    dependency_type,
                    source_task_id,
                    preview.attempt_kind,
                    preview.attempt_number,
                    preview.task.permission_profile.to_string(),
                ],
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;

        let batch = self
            .batch(&batch_id)?
            .ok_or_else(|| "created attempt batch disappeared".to_string())?;
        let task = self
            .task(&task_id)?
            .ok_or_else(|| "created attempt task disappeared".to_string())?;
        Ok(AttemptCreateResult {
            batch,
            task,
            idempotent_replay: false,
        })
    }

    pub fn list_tasks(&self, status: Option<&str>) -> Result<Vec<Task>, String> {
        let mut sql = "SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id,dependency_type,source_task_id,attempt_kind,attempt_number,permission_profile FROM tasks WHERE archived_at IS NULL".to_string();
        if status.is_some() {
            sql.push_str(" AND status=?1");
        }
        sql.push_str(" ORDER BY run_at ASC, position ASC");
        let mut statement = self.connection.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = if let Some(value) = status {
            statement
                .query_map(params![value], row_to_task)
                .map_err(|e| e.to_string())?
        } else {
            statement
                .query_map([], row_to_task)
                .map_err(|e| e.to_string())?
        };
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn list_task_page(
        &self,
        status: Option<&str>,
        sort: TaskSort,
        requested_page: i64,
    ) -> Result<TaskPage, String> {
        if requested_page < 1 {
            return Err("page must be at least 1".to_string());
        }
        if let Some(status) = status {
            if !is_known_task_status(status) {
                return Err("status is not a known task status".to_string());
            }
        }
        let total = match status {
            Some(status) => self
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE archived_at IS NULL AND status=?1",
                    params![status],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?,
            None => self
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE archived_at IS NULL",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?,
        };
        let total_pages = ((total + TASK_PAGE_SIZE - 1) / TASK_PAGE_SIZE).max(1);
        let page = requested_page.min(total_pages);
        let offset = (page - 1) * TASK_PAGE_SIZE;
        let order = match sort {
            TaskSort::Newest => "created_at DESC, id DESC",
            TaskSort::Oldest => "created_at ASC, id ASC",
            TaskSort::RunAtNewest => "run_at DESC, position DESC, id DESC",
            TaskSort::RunAtOldest => "run_at ASC, position ASC, id ASC",
            TaskSort::TitleAscending => "title COLLATE NOCASE ASC, created_at DESC, id DESC",
            TaskSort::TitleDescending => "title COLLATE NOCASE DESC, created_at DESC, id DESC",
        };
        let columns = "id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id,dependency_type,source_task_id,attempt_kind,attempt_number,permission_profile";
        let sql = if status.is_some() {
            format!(
                "SELECT {columns} FROM tasks WHERE archived_at IS NULL AND status=?1 ORDER BY {order} LIMIT ?2 OFFSET ?3"
            )
        } else {
            format!("SELECT {columns} FROM tasks WHERE archived_at IS NULL ORDER BY {order} LIMIT ?1 OFFSET ?2")
        };
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| error.to_string())?;
        let items = if let Some(status) = status {
            statement
                .query_map(params![status, TASK_PAGE_SIZE, offset], row_to_task)
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        } else {
            statement
                .query_map(params![TASK_PAGE_SIZE, offset], row_to_task)
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        Ok(TaskPage {
            items,
            page,
            page_size: TASK_PAGE_SIZE,
            total,
            total_pages,
            sort: sort.to_string(),
        })
    }

    pub fn list_batch_page(
        &self,
        status: Option<&str>,
        sort: TaskSort,
        requested_page: i64,
    ) -> Result<BatchPage, String> {
        if requested_page < 1 {
            return Err("page must be at least 1".to_string());
        }
        if let Some(status) = status {
            if !is_known_task_status(status) {
                return Err("status is not a known task status".to_string());
            }
        }
        let mut grouped = BTreeMap::<String, Vec<Task>>::new();
        for task in self.list_tasks(None)? {
            grouped.entry(task.batch_id.clone()).or_default().push(task);
        }
        let now = now_epoch();
        let mut items = Vec::with_capacity(grouped.len());
        for (batch_id, mut tasks) in grouped {
            tasks.sort_by_key(|task| task.position);
            if status.is_some_and(|value| !tasks.iter().any(|task| task.status == value)) {
                continue;
            }
            let batch = self
                .batch(&batch_id)?
                .ok_or_else(|| "task batch not found".to_string())?;
            items.push(BatchGroup {
                editable: batch_tasks_are_editable(&tasks),
                editing: self.batch_edit_is_active(&batch_id, now)?,
                batch,
                tasks,
            });
        }
        items.sort_by(|left, right| {
            let left_task = &left.tasks[0];
            let right_task = &right.tasks[0];
            match sort {
                TaskSort::Newest => right
                    .batch
                    .created_at
                    .cmp(&left.batch.created_at)
                    .then_with(|| right.batch.id.cmp(&left.batch.id)),
                TaskSort::Oldest => left
                    .batch
                    .created_at
                    .cmp(&right.batch.created_at)
                    .then_with(|| left.batch.id.cmp(&right.batch.id)),
                TaskSort::RunAtNewest => right_task
                    .run_at
                    .cmp(&left_task.run_at)
                    .then_with(|| right.batch.id.cmp(&left.batch.id)),
                TaskSort::RunAtOldest => left_task
                    .run_at
                    .cmp(&right_task.run_at)
                    .then_with(|| left.batch.id.cmp(&right.batch.id)),
                TaskSort::TitleAscending => left_task
                    .title
                    .to_lowercase()
                    .cmp(&right_task.title.to_lowercase())
                    .then_with(|| left.batch.id.cmp(&right.batch.id)),
                TaskSort::TitleDescending => right_task
                    .title
                    .to_lowercase()
                    .cmp(&left_task.title.to_lowercase())
                    .then_with(|| right.batch.id.cmp(&left.batch.id)),
            }
        });
        let total = items.len() as i64;
        let total_pages = ((total + BATCH_PAGE_SIZE - 1) / BATCH_PAGE_SIZE).max(1);
        let page = requested_page.min(total_pages);
        let start = ((page - 1) * BATCH_PAGE_SIZE) as usize;
        let items = items
            .into_iter()
            .skip(start)
            .take(BATCH_PAGE_SIZE as usize)
            .collect();
        Ok(BatchPage {
            items,
            page,
            page_size: BATCH_PAGE_SIZE,
            total,
            total_pages,
            sort: sort.to_string(),
        })
    }

    pub fn begin_batch_edit(&mut self, batch_id: &str) -> Result<BatchEditSession, String> {
        let now = now_epoch();
        let session_id = new_id("batch-edit");
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let tasks = query_tasks_for_batch(&transaction, batch_id, false)?;
        if tasks.is_empty() {
            return Err("batch not found".to_string());
        }
        if !batch_tasks_are_editable(&tasks) {
            return Err("the batch can only be edited before its first task starts".to_string());
        }
        let changed = transaction
            .execute(
                "UPDATE batches
                 SET edit_session_id=?2,edit_heartbeat_at=?3
                 WHERE id=?1
                   AND (edit_session_id IS NULL OR edit_heartbeat_at IS NULL OR edit_heartbeat_at<?4)",
                params![batch_id, session_id, now, now - BATCH_EDIT_LEASE_SECONDS],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("the batch is already being edited".to_string());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(BatchEditSession {
            edit_session_id: session_id,
            expires_at: now + BATCH_EDIT_LEASE_SECONDS,
            batch: self
                .batch(batch_id)?
                .ok_or_else(|| "batch not found".to_string())?,
            tasks,
        })
    }

    pub fn heartbeat_batch_edit(
        &self,
        batch_id: &str,
        edit_session_id: &str,
    ) -> Result<i64, String> {
        let now = now_epoch();
        let changed = self
            .connection
            .execute(
                "UPDATE batches
                 SET edit_heartbeat_at=?3
                 WHERE id=?1 AND edit_session_id=?2 AND edit_heartbeat_at>=?4",
                params![
                    batch_id,
                    edit_session_id,
                    now,
                    now - BATCH_EDIT_LEASE_SECONDS
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("the batch edit session expired; reopen the editor".to_string());
        }
        Ok(now + BATCH_EDIT_LEASE_SECONDS)
    }

    pub fn cancel_batch_edit(&self, batch_id: &str, edit_session_id: &str) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE batches
                 SET edit_session_id=NULL,edit_heartbeat_at=NULL
                 WHERE id=?1 AND edit_session_id=?2",
                params![batch_id, edit_session_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("the batch edit session is not active".to_string());
        }
        Ok(())
    }

    pub fn update_batch(
        &mut self,
        batch_id: &str,
        input: BatchEditInput,
    ) -> Result<BatchGroup, String> {
        if input.edit_session_id.trim().is_empty() {
            return Err("edit_session_id is required".to_string());
        }
        if input.tasks.is_empty() {
            return Err("a batch must contain at least one task".to_string());
        }
        let mut seen_task_ids = HashSet::new();
        let drafts = input
            .tasks
            .iter()
            .enumerate()
            .map(|(position, task)| {
                if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
                    return Err("every task needs a title and prompt".to_string());
                }
                if task
                    .task_id
                    .as_ref()
                    .is_some_and(|id| id.trim().is_empty() || !seen_task_ids.insert(id.clone()))
                {
                    return Err("task_id values must be non-empty and unique".to_string());
                }
                require_networked_confirmation(task.permission_profile, input.networked_confirmed)?;
                Ok(TaskDraft {
                    title: task.title.clone(),
                    prompt: task.prompt.clone(),
                    success_criteria: task.success_criteria.clone(),
                    cwd: task.cwd.clone(),
                    run_at: (position == 0).then(|| task.run_at.clone()).flatten(),
                    after_previous: position != 0,
                    continue_from_task_id: None,
                    timezone: Some(task.timezone.clone()),
                    difficulty: task.difficulty,
                    model: Some(task.model.clone()),
                    effort: Some(task.effort.clone()),
                    permission_profile: task.permission_profile,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if drafts[0].run_at.is_none() {
            return Err("the first task requires run_at".to_string());
        }
        let normalized = normalize_drafts(&drafts, |_, _| {
            Err("batch editing does not create quota continuations".to_string())
        })
        .map_err(|error| {
            if error == "run_at must be in the future" {
                "the batch start time has passed; choose a future time before saving changes"
                    .to_string()
            } else {
                error
            }
        })?;

        let now = now_epoch();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let active_session: Option<String> = transaction
            .query_row(
                "SELECT edit_session_id FROM batches
                 WHERE id=?1 AND edit_session_id=?2 AND edit_heartbeat_at>=?3",
                params![
                    batch_id,
                    input.edit_session_id,
                    now - BATCH_EDIT_LEASE_SECONDS
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if active_session.is_none() {
            return Err("the batch edit session expired; reopen the editor".to_string());
        }
        let existing = query_tasks_for_batch(&transaction, batch_id, false)?;
        if !batch_tasks_are_editable(&existing) {
            return Err("the batch can only be edited before its first task starts".to_string());
        }
        let existing_by_id = existing
            .iter()
            .map(|task| (task.id.clone(), task))
            .collect::<BTreeMap<_, _>>();
        for task_id in &seen_task_ids {
            if !existing_by_id.contains_key(task_id) {
                return Err(format!(
                    "task '{task_id}' does not belong to this editable batch"
                ));
            }
        }

        let task_ids = input
            .tasks
            .iter()
            .map(|task| task.task_id.clone().unwrap_or_else(|| new_id("task")))
            .collect::<Vec<_>>();
        for (position, ((task, draft), normalized)) in input
            .tasks
            .iter()
            .zip(drafts.iter())
            .zip(normalized.iter())
            .enumerate()
        {
            let dependency = (position != 0).then(|| task_ids[position - 1].as_str());
            if task.task_id.is_some() {
                transaction
                    .execute(
                        "UPDATE tasks
                         SET title=?2,prompt=?3,success_criteria=?4,cwd=?5,run_at=?6,status='scheduled',
                             timezone=?7,difficulty=?8,model=?9,effort=?10,
                             permission_profile=?11,position=?12,depends_on_task_id=?13,
                             dependency_type='success',updated_at=?14,last_error=NULL
                         WHERE id=?1 AND batch_id=?15 AND status IN ('scheduled','cancelled')
                           AND archived_at IS NULL",
                        params![
                            task_ids[position],
                            draft.title.trim(),
                            draft.prompt.trim(),
                            draft.success_criteria.trim(),
                            draft.cwd,
                            normalized.run_at,
                            normalized.timezone,
                            draft.difficulty.to_string(),
                            normalized.model,
                            normalized.effort,
                            draft.permission_profile.to_string(),
                            position as i64,
                            dependency,
                            now,
                            batch_id,
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            } else {
                transaction
                    .execute(
                        "INSERT INTO tasks
                         (id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,
                          difficulty,model,effort,status,created_at,updated_at,position,
                          depends_on_task_id,dependency_type,permission_profile)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'scheduled',?12,?12,?13,?14,'success',?15)",
                        params![
                            task_ids[position],
                            batch_id,
                            draft.title.trim(),
                            draft.prompt.trim(),
                            draft.success_criteria.trim(),
                            draft.cwd,
                            normalized.run_at,
                            normalized.timezone,
                            draft.difficulty.to_string(),
                            normalized.model,
                            normalized.effort,
                            now,
                            position as i64,
                            dependency,
                            draft.permission_profile.to_string(),
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        for task in existing
            .iter()
            .filter(|task| !seen_task_ids.contains(&task.id))
        {
            transaction
                .execute(
                    "UPDATE tasks
                     SET status='cancelled',archived_at=?2,updated_at=?2,
                         last_error='Removed while editing batch'
                     WHERE id=?1 AND status IN ('scheduled','cancelled') AND archived_at IS NULL",
                    params![task.id, now],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute(
                "UPDATE batches SET edit_session_id=NULL,edit_heartbeat_at=NULL
                 WHERE id=?1 AND edit_session_id=?2",
                params![batch_id, input.edit_session_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;

        let tasks = query_tasks_for_batch(&self.connection, batch_id, false)?;
        Ok(BatchGroup {
            editable: batch_tasks_are_editable(&tasks),
            editing: false,
            batch: self
                .batch(batch_id)?
                .ok_or_else(|| "batch not found".to_string())?,
            tasks,
        })
    }

    pub fn task(&self, task_id: &str) -> Result<Option<Task>, String> {
        self.connection
            .query_row(
                "SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id,dependency_type,source_task_id,attempt_kind,attempt_number,permission_profile FROM tasks WHERE id=?1",
                params![task_id], row_to_task,
            )
            .optional().map_err(|e| e.to_string())
    }

    pub fn task_status(&self, task_id: &str) -> Result<Option<TaskStatus>, String> {
        let task = match self.task(task_id)? {
            Some(task) => task,
            None => return Ok(None),
        };
        let batch = self
            .batch(&task.batch_id)?
            .ok_or_else(|| "task batch not found".to_string())?;
        let mut statement = self.connection.prepare(
            "SELECT id,task_id,started_at,finished_at,status,usage_before_json,usage_after_json,session_id,transcript_path,tokens_used,token_usage_state,error FROM runs WHERE task_id=?1 ORDER BY started_at ASC",
        ).map_err(|e| e.to_string())?;
        let runs = statement
            .query_map(params![task_id], |row| {
                Ok(RunRecord {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    started_at: row.get(2)?,
                    finished_at: row.get(3)?,
                    status: row.get(4)?,
                    usage_before_json: row.get(5)?,
                    usage_after_json: row.get(6)?,
                    session_id: row.get(7)?,
                    transcript_path: row.get(8)?,
                    tokens_used: row.get(9)?,
                    token_usage_state: row.get(10)?,
                    error: row.get(11)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let lineage = self.task_lineage(task_id)?;
        Ok(Some(TaskStatus {
            task,
            batch,
            runs,
            lineage,
        }))
    }

    pub fn task_lineage(&self, task_id: &str) -> Result<Vec<Task>, String> {
        let mut root = self
            .task(task_id)?
            .ok_or_else(|| "task not found".to_string())?;
        let mut seen = HashSet::new();
        while let Some(source_task_id) = root.source_task_id.clone() {
            if !seen.insert(root.id.clone()) {
                return Err("attempt lineage contains a cycle".to_string());
            }
            root = self
                .task(&source_task_id)?
                .ok_or_else(|| "attempt lineage source task not found".to_string())?;
        }
        let mut statement = self.connection.prepare(
            "WITH RECURSIVE attempt_lineage AS (
               SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id,dependency_type,source_task_id,attempt_kind,attempt_number,permission_profile
               FROM tasks WHERE id=?1
               UNION
               SELECT child.id,child.batch_id,child.title,child.prompt,child.success_criteria,child.cwd,child.run_at,child.timezone,child.difficulty,child.model,child.effort,child.status,child.created_at,child.updated_at,child.last_error,child.position,child.depends_on_task_id,child.dependency_type,child.source_task_id,child.attempt_kind,child.attempt_number,child.permission_profile
               FROM tasks child JOIN attempt_lineage parent ON child.source_task_id=parent.id
             )
             SELECT * FROM attempt_lineage ORDER BY attempt_number ASC, created_at ASC, id ASC",
        ).map_err(|e| e.to_string())?;
        let lineage = statement
            .query_map(params![root.id], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(lineage)
    }

    pub fn task_usage_stats(&self, now: i64, timezone: &str) -> Result<TaskUsageStats, String> {
        const DAY_SECONDS: i64 = 86_400;
        const WEEK_SECONDS: i64 = 7 * DAY_SECONDS;
        const MONTH_SECONDS: i64 = 30 * DAY_SECONDS;
        const YEAR_SECONDS: i64 = 365 * DAY_SECONDS;

        let timezone = timezone
            .parse::<Tz>()
            .map_err(|_| format!("invalid IANA timezone '{timezone}'"))?;
        let year_since = now.saturating_sub(YEAR_SECONDS);
        let month_since = now.saturating_sub(MONTH_SECONDS);
        let week_since = now.saturating_sub(WEEK_SECONDS);
        let mut statement = self
            .connection
            .prepare(
                "SELECT run.id,run.task_id,task.title,run.status,run.started_at,run.finished_at,
                    batch.budget_mode,task.model,task.effort,run.tokens_used,run.token_usage_state,
                    task.permission_profile
             FROM runs run
             JOIN tasks task ON task.id=run.task_id
             JOIN batches batch ON batch.id=task.batch_id
             WHERE run.started_at>=?1
             ORDER BY run.started_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let runs = statement
            .query_map(params![year_since], |row| {
                Ok(StoredUsageRun {
                    run_id: row.get(0)?,
                    task_id: row.get(1)?,
                    task_title: row.get(2)?,
                    status: row.get(3)?,
                    started_at: row.get(4)?,
                    finished_at: row.get(5)?,
                    budget_mode: row.get(6)?,
                    model: row.get(7)?,
                    effort: row.get(8)?,
                    tokens_used: row.get(9)?,
                    token_usage_state: row.get(10)?,
                    permission_profile: permission_profile_from_row(row, 11)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let now_utc = DateTime::<Utc>::from_timestamp(now, 0)
            .ok_or_else(|| "current time is outside the supported timestamp range".to_string())?;
        let today = now_utc.with_timezone(&timezone).date_naive();
        let first_day = today - chrono::Duration::days(6);
        let mut daily_last_7_days = (0..7)
            .map(|offset| DailyUsageStats {
                date: (first_day + chrono::Duration::days(offset)).to_string(),
                run_count: 0,
                tokens_used: 0,
                tokens_unavailable_runs: 0,
                status_counts: BTreeMap::new(),
            })
            .collect::<Vec<_>>();
        let mut individual_runs_last_7_days = Vec::new();
        for run in runs.iter().filter(|run| run.started_at >= week_since) {
            let Some(started_utc) = DateTime::<Utc>::from_timestamp(run.started_at, 0) else {
                continue;
            };
            let started_local = started_utc.with_timezone(&timezone);
            let day_offset = (started_local.date_naive() - first_day).num_days();
            if let Some(day) = usize::try_from(day_offset)
                .ok()
                .filter(|offset| *offset < daily_last_7_days.len())
                .and_then(|offset| daily_last_7_days.get_mut(offset))
            {
                add_usage(day, run);
            }
            individual_runs_last_7_days.push(IndividualRunUsage {
                run_id: run.run_id.clone(),
                task_id: run.task_id.clone(),
                task_title: run.task_title.clone(),
                status: run.status.clone(),
                started_at: run.started_at,
                started_at_iso: started_local.to_rfc3339(),
                finished_at: run.finished_at,
                finished_at_iso: run.finished_at.and_then(|timestamp| {
                    DateTime::<Utc>::from_timestamp(timestamp, 0)
                        .map(|value| value.with_timezone(&timezone).to_rfc3339())
                }),
                budget_mode: run.budget_mode.clone(),
                model: run.model.clone(),
                effort: run.effort.clone(),
                permission_profile: run.permission_profile,
                tokens_used: run.tokens_used,
                token_usage_state: run.token_usage_state.clone(),
            });
        }

        Ok(TaskUsageStats {
            generated_at: now,
            timezone: timezone.to_string(),
            last_year: summarize_usage(&runs, year_since),
            last_month: summarize_usage(&runs, month_since),
            last_week: summarize_usage(&runs, week_since),
            daily_last_7_days,
            individual_runs_last_7_days,
        })
    }

    pub(crate) fn prediction_samples(
        &self,
        since: i64,
    ) -> Result<Vec<HistoricalUsageSample>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT task.difficulty,task.model,task.effort,run.tokens_used,
                        run.usage_before_json,run.usage_after_json
                 FROM runs run
                 JOIN tasks task ON task.id=run.task_id
                 WHERE run.status='completed'
                   AND run.token_usage_state='reported'
                   AND run.tokens_used>0
                   AND run.started_at>=?1
                 ORDER BY run.started_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![since], |row| {
                Ok(HistoricalUsageSample {
                    difficulty: row.get(0)?,
                    model: row.get(1)?,
                    effort: row.get(2)?,
                    tokens_used: row.get(3)?,
                    usage_before_json: row.get(4)?,
                    usage_after_json: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn due_tasks(&self, now: i64) -> Result<Vec<Task>, String> {
        let mut statement = self.connection.prepare(
            "SELECT task.id,task.batch_id,task.title,task.prompt,task.success_criteria,task.cwd,
                    task.run_at,task.timezone,task.difficulty,task.model,task.effort,task.status,
                    task.created_at,task.updated_at,task.last_error,task.position,task.depends_on_task_id,
                    task.dependency_type,task.source_task_id,task.attempt_kind,task.attempt_number,
                    task.permission_profile
             FROM tasks task
             JOIN batches batch ON batch.id=task.batch_id
             LEFT JOIN tasks prerequisite ON prerequisite.id=task.depends_on_task_id
             WHERE task.archived_at IS NULL
               AND task.status='scheduled' AND task.run_at<=?1
               AND (batch.edit_session_id IS NULL OR batch.edit_heartbeat_at IS NULL
                    OR batch.edit_heartbeat_at<?2)
               AND (task.depends_on_task_id IS NULL
                    OR (task.dependency_type='success' AND prerequisite.status='completed')
                    OR (task.dependency_type='quota_reset'
                        AND prerequisite.status IN ('quota_interrupted','quota_skipped')))
             ORDER BY task.run_at ASC, task.position ASC",
        ).map_err(|e| e.to_string())?;
        let tasks = statement
            .query_map(params![now, now - BATCH_EDIT_LEASE_SECONDS], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(tasks)
    }

    pub fn failed_dependency_tasks(&self) -> Result<Vec<Task>, String> {
        let mut statement = self.connection.prepare(
            "SELECT task.id,task.batch_id,task.title,task.prompt,task.success_criteria,task.cwd,
                    task.run_at,task.timezone,task.difficulty,task.model,task.effort,task.status,
                    task.created_at,task.updated_at,task.last_error,task.position,task.depends_on_task_id,
                    task.dependency_type,task.source_task_id,task.attempt_kind,task.attempt_number,
                    task.permission_profile
             FROM tasks task
             JOIN batches batch ON batch.id=task.batch_id
             JOIN tasks prerequisite ON prerequisite.id=task.depends_on_task_id
             WHERE task.archived_at IS NULL
               AND task.status='scheduled'
               AND (batch.edit_session_id IS NULL OR batch.edit_heartbeat_at IS NULL
                    OR batch.edit_heartbeat_at<?1)
               AND task.dependency_type='success'
               AND prerequisite.status NOT IN ('scheduled','running','completed')
             ORDER BY task.run_at ASC, task.position ASC",
        ).map_err(|e| e.to_string())?;
        let tasks = statement
            .query_map(params![now_epoch() - BATCH_EDIT_LEASE_SECONDS], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(tasks)
    }

    pub fn dependency_completed_at(&self, task: &Task) -> Result<Option<i64>, String> {
        let Some(dependency_id) = task.depends_on_task_id.as_deref() else {
            return Ok(None);
        };
        if task.dependency_type != "success" {
            return Ok(None);
        }
        self.connection
            .query_row(
                "SELECT updated_at FROM tasks WHERE id=?1 AND status='completed'",
                params![dependency_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn claim_task(&self, task_id: &str) -> Result<bool, String> {
        let now = now_epoch();
        let changed = self
            .connection
            .execute(
                "UPDATE tasks SET status='running',updated_at=?2
             WHERE id=?1 AND status='scheduled' AND archived_at IS NULL
               AND EXISTS (
                 SELECT 1 FROM batches batch
                 WHERE batch.id=tasks.batch_id
                   AND (batch.edit_session_id IS NULL OR batch.edit_heartbeat_at IS NULL
                        OR batch.edit_heartbeat_at<?3)
               )",
                params![task_id, now, now - BATCH_EDIT_LEASE_SECONDS],
            )
            .map_err(|e| e.to_string())?;
        Ok(changed == 1)
    }

    pub fn update_task(&mut self, task_id: &str, update: TaskUpdate) -> Result<Task, String> {
        let task = self
            .task(task_id)?
            .ok_or_else(|| "task not found".to_string())?;
        if task.status != "scheduled" {
            return Err("only scheduled tasks can be updated".to_string());
        }
        if self.batch_edit_is_active(&task.batch_id, now_epoch())? {
            return Err("the task belongs to a batch that is currently being edited".to_string());
        }
        let difficulty = update
            .difficulty
            .unwrap_or(Difficulty::from_str(&task.difficulty)?);
        let routed = route(difficulty);
        let model = update.model.unwrap_or_else(|| {
            if update.difficulty.is_some() {
                routed.model
            } else {
                task.model.clone()
            }
        });
        let effort = update.effort.unwrap_or_else(|| {
            if update.difficulty.is_some() {
                routed.effort
            } else {
                task.effort.clone()
            }
        });
        validate_route(&model, &effort)?;
        let permission_profile = update.permission_profile.unwrap_or(task.permission_profile);
        require_networked_confirmation(permission_profile, update.networked_confirmed)?;
        let timezone = update.timezone.unwrap_or(task.timezone.clone());
        validate_timezone(&timezone)?;
        let run_at = if let Some(value) = update.run_at {
            if task.depends_on_task_id.is_some() {
                return Err(
                    "run_at cannot be changed for a task chained after another task".to_string(),
                );
            }
            let (timestamp, explicit_offset) = parse_future_time(&value)?;
            validate_timezone_at(&timezone, timestamp, explicit_offset)?;
            timestamp
        } else {
            task.run_at
        };
        let cwd = update.cwd.unwrap_or(task.cwd.clone());
        if task.dependency_type == "quota_reset" && cwd != task.cwd {
            return Err("a continuation task must stay in the predecessor worktree".to_string());
        }
        validate_cwd(&cwd)?;
        let changed = self.connection.execute(
            "UPDATE tasks SET title=?2,prompt=?3,success_criteria=?4,cwd=?5,run_at=?6,timezone=?7,difficulty=?8,model=?9,effort=?10,permission_profile=?11,updated_at=?12 WHERE id=?1 AND status='scheduled'",
            params![task_id, update.title.unwrap_or(task.title), update.prompt.unwrap_or(task.prompt),
                update.success_criteria.unwrap_or(task.success_criteria), cwd, run_at, timezone,
                difficulty.to_string(), model, effort, permission_profile.to_string(), now_epoch()],
        ).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("only scheduled tasks can be updated".to_string());
        }
        self.task(task_id)?
            .ok_or_else(|| "updated task disappeared".to_string())
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<Task, String> {
        let task = self
            .task(task_id)?
            .ok_or_else(|| "task not found".to_string())?;
        if task.status != "scheduled" {
            return Err("only scheduled tasks can be cancelled".to_string());
        }
        if self.batch_edit_is_active(&task.batch_id, now_epoch())? {
            return Err("the task belongs to a batch that is currently being edited".to_string());
        }
        self.set_status(task_id, "cancelled", None)?;
        self.task(task_id)?
            .ok_or_else(|| "cancelled task disappeared".to_string())
    }

    pub fn request_task_stop(&self, task_id: &str) -> Result<Task, String> {
        let task = self
            .task(task_id)?
            .ok_or_else(|| "task not found".to_string())?;
        if task.status != "running" {
            return Err("only a running task can be stopped".to_string());
        }
        let inserted = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO task_stop_requests (task_id,requested_at)
                 SELECT id,?2 FROM tasks WHERE id=?1 AND status='running'",
                params![task_id, now_epoch()],
            )
            .map_err(|error| error.to_string())?;
        if inserted == 0 && !self.task_stop_requested(task_id)? {
            return Err("task is no longer running".to_string());
        }
        self.task(task_id)?
            .ok_or_else(|| "running task disappeared".to_string())
    }

    pub fn archive_task(&self, task_id: &str) -> Result<ArchiveTaskResult, String> {
        let state = self
            .connection
            .query_row(
                "SELECT status,archived_at,batch_id FROM tasks WHERE id=?1",
                params![task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "task not found".to_string())?;
        if state.0 == "running" {
            return Err("a running task must be stopped before it can be removed".to_string());
        }
        if state.1.is_none() && self.batch_edit_is_active(&state.2, now_epoch())? {
            return Err("the task belongs to a batch that is currently being edited".to_string());
        }
        let archived_at = state.1.unwrap_or_else(now_epoch);
        if state.1.is_none() {
            let changed = self
                .connection
                .execute(
                    "UPDATE tasks
                     SET status=CASE WHEN status='scheduled' THEN 'cancelled' ELSE status END,
                         archived_at=?2,updated_at=?2
                     WHERE id=?1 AND archived_at IS NULL AND status!='running'",
                    params![task_id, archived_at],
                )
                .map_err(|error| error.to_string())?;
            if changed != 1 {
                return Err(
                    "task changed while it was being removed; refresh and retry".to_string()
                );
            }
        }
        let status = self
            .connection
            .query_row(
                "SELECT status FROM tasks WHERE id=?1",
                params![task_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let preserved_runs = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE task_id=?1",
                params![task_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Ok(ArchiveTaskResult {
            task_id: task_id.to_string(),
            archived_at,
            status,
            preserved_runs,
        })
    }

    pub fn task_stop_requested(&self, task_id: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_stop_requests WHERE task_id=?1)",
                params![task_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub fn clear_task_stop_request(&self, task_id: &str) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM task_stop_requests WHERE task_id=?1",
                params![task_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn set_status(
        &self,
        task_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE tasks SET status=?2,last_error=?3,updated_at=?4 WHERE id=?1",
                params![task_id, status, error, now_epoch()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn defer_continuation(
        &self,
        task_id: &str,
        run_at: i64,
        reason: &str,
    ) -> Result<(), String> {
        if run_at <= now_epoch() {
            return Err("next 5-hour quota reset must be in the future".to_string());
        }
        let changed = self
            .connection
            .execute(
                "UPDATE tasks SET status='scheduled',run_at=?2,last_error=?3,updated_at=?4
                 WHERE id=?1 AND status='running' AND dependency_type='quota_reset'",
                params![task_id, run_at, reason, now_epoch()],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("only a running quota-reset continuation can be deferred".to_string());
        }
        Ok(())
    }

    fn continuation_reset_at(&self, predecessor_id: &str, cwd: &str) -> Result<i64, String> {
        let predecessor = self
            .task(predecessor_id)?
            .ok_or_else(|| format!("continuation predecessor task not found: {predecessor_id}"))?;
        if !matches!(
            predecessor.status.as_str(),
            "quota_interrupted" | "quota_skipped"
        ) {
            return Err(format!(
                "continuation predecessor must be quota_interrupted or quota_skipped, not {}",
                predecessor.status
            ));
        }
        if predecessor.cwd != cwd {
            return Err("continuation task must use the predecessor worktree".to_string());
        }
        self.validate_existing_dependency_chain(predecessor_id)?;
        let usage_json = self
            .connection
            .query_row(
                "SELECT COALESCE(usage_after_json,usage_before_json) FROM runs
                 WHERE task_id=?1 ORDER BY started_at DESC LIMIT 1",
                params![predecessor_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten()
            .ok_or_else(|| "continuation predecessor has no quota snapshot".to_string())?;
        let snapshot: UsageSnapshot = serde_json::from_str(&usage_json)
            .map_err(|_| "continuation predecessor has no valid quota snapshot".to_string())?;
        Ok(snapshot
            .five_hour
            .ok_or_else(|| "continuation predecessor has no 5-hour quota snapshot".to_string())?
            .resets_at
            .max(now_epoch()))
    }

    fn quota_resume_metadata(
        &self,
        predecessor_id: &str,
        cwd: &str,
    ) -> Result<(i64, bool), String> {
        let predecessor = self
            .task(predecessor_id)?
            .ok_or_else(|| "task not found".to_string())?;
        if !matches!(
            predecessor.status.as_str(),
            "quota_interrupted" | "quota_skipped"
        ) {
            return Err(format!(
                "quota_resume source must be quota_interrupted or quota_skipped, not {}",
                predecessor.status
            ));
        }
        if predecessor.cwd != cwd {
            return Err("quota_resume task must use the source worktree".to_string());
        }
        self.validate_existing_dependency_chain(predecessor_id)?;
        let (usage_json, session_id) = self
            .connection
            .query_row(
                "SELECT COALESCE(usage_after_json,usage_before_json),session_id FROM runs
                 WHERE task_id=?1 ORDER BY started_at DESC LIMIT 1",
                params![predecessor_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "quota_resume source has no run metadata".to_string())?;
        let usage_json =
            usage_json.ok_or_else(|| "quota_resume source has no quota snapshot".to_string())?;
        let snapshot: UsageSnapshot = serde_json::from_str(&usage_json)
            .map_err(|_| "quota_resume source has no valid quota snapshot".to_string())?;
        let five_hour = snapshot
            .five_hour
            .ok_or_else(|| "quota_resume source has no 5-hour quota snapshot".to_string())?;
        if five_hour.resets_at <= 0 {
            return Err("quota_resume source has stale quota reset metadata".to_string());
        }
        Ok((
            five_hour.resets_at,
            session_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()),
        ))
    }

    fn validate_existing_dependency_chain(&self, task_id: &str) -> Result<(), String> {
        let mut current = Some(task_id.to_string());
        let mut seen = HashSet::new();
        while let Some(id) = current {
            if !seen.insert(id.clone()) {
                return Err("continuation predecessor has an invalid dependency cycle".to_string());
            }
            current = self.task(&id)?.and_then(|task| task.depends_on_task_id);
        }
        Ok(())
    }

    pub(crate) fn continuation_context(
        &self,
        task: &Task,
    ) -> Result<Option<ContinuationContext>, String> {
        if task.dependency_type != "quota_reset" {
            return Ok(None);
        }
        let predecessor_id = task
            .depends_on_task_id
            .as_deref()
            .ok_or_else(|| "quota-reset continuation has no predecessor".to_string())?;
        let predecessor = self
            .task(predecessor_id)?
            .ok_or_else(|| "continuation predecessor not found".to_string())?;
        let run = self
            .connection
            .query_row(
                "SELECT session_id,transcript_path FROM runs
                 WHERE task_id=?1 ORDER BY started_at DESC LIMIT 1",
                params![predecessor_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (session_id, transcript_path) = run.unwrap_or((None, None));
        let transcript_excerpt = transcript_path.as_deref().and_then(read_transcript_excerpt);
        Ok(Some(ContinuationContext {
            predecessor_task_id: predecessor.id,
            predecessor_title: predecessor.title,
            predecessor_prompt: predecessor.prompt,
            predecessor_success_criteria: predecessor.success_criteria,
            cwd: predecessor.cwd,
            session_id: session_id.filter(|value| !value.trim().is_empty()),
            transcript_path,
            transcript_excerpt,
        }))
    }

    pub fn batch(&self, batch_id: &str) -> Result<Option<Batch>, String> {
        self.connection.query_row(
            "SELECT id,idempotency_key,budget_mode,token_cap,consumed_tokens,cap_percent,cap_basis,created_at,window_reset_at,baseline_weekly_used_percent,allowance_points,consumed_points,five_hour_cap_percent,five_hour_window_reset_at,baseline_five_hour_used_percent,five_hour_allowance_points,five_hour_consumed_points FROM batches WHERE id=?1",
            params![batch_id], row_to_batch,
        ).optional().map_err(|e| e.to_string())
    }

    pub fn ensure_budget_window(
        &self,
        batch_id: &str,
        weekly_used: f64,
        reset_at: i64,
    ) -> Result<Batch, String> {
        let current = self
            .batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())?;
        if current.budget_mode != "percentage" {
            return Ok(current);
        }
        if current.window_reset_at != Some(reset_at) {
            let remaining = (100.0 - weekly_used).max(0.0);
            let allowance = if current.cap_basis == "total_weekly_percent" {
                remaining.min(current.cap_percent)
            } else {
                (remaining * current.cap_percent / 100.0).max(0.0)
            };
            self.connection.execute(
                "UPDATE batches SET window_reset_at=?2,baseline_weekly_used_percent=?3,allowance_points=?4,consumed_points=0 WHERE id=?1",
                params![batch_id, reset_at, weekly_used, allowance],
            ).map_err(|e| e.to_string())?;
        }
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn ensure_five_hour_window(
        &self,
        batch_id: &str,
        five_hour_used: f64,
        reset_at: i64,
    ) -> Result<Batch, String> {
        let current = self
            .batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())?;
        let Some(cap) = current.five_hour_cap_percent else {
            return Ok(current);
        };
        if current.five_hour_window_reset_at != Some(reset_at) {
            let available_before_reserve =
                (100.0 - crate::config::FIVE_HOUR_RESERVE_PERCENT - five_hour_used).max(0.0);
            let allowance = cap.min(available_before_reserve);
            self.connection
                .execute(
                    "UPDATE batches
                     SET five_hour_window_reset_at=?2,baseline_five_hour_used_percent=?3,
                         five_hour_allowance_points=?4,five_hour_consumed_points=0
                     WHERE id=?1",
                    params![batch_id, reset_at, five_hour_used, allowance],
                )
                .map_err(|e| e.to_string())?;
        }
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn add_five_hour_consumption(&self, batch_id: &str, delta: f64) -> Result<Batch, String> {
        self.connection
            .execute(
                "UPDATE batches
                 SET five_hour_consumed_points=MIN(100.0, five_hour_consumed_points + ?2)
                 WHERE id=?1 AND five_hour_cap_percent IS NOT NULL",
                params![batch_id, delta.max(0.0)],
            )
            .map_err(|e| e.to_string())?;
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn reconcile_five_hour_consumption(
        &self,
        batch_id: &str,
        five_hour_used: f64,
    ) -> Result<Batch, String> {
        self.connection
            .execute(
                "UPDATE batches
                 SET five_hour_consumed_points=MAX(
                     five_hour_consumed_points,
                     MAX(0, ?2 - baseline_five_hour_used_percent)
                 )
                 WHERE id=?1 AND five_hour_cap_percent IS NOT NULL
                   AND baseline_five_hour_used_percent IS NOT NULL",
                params![batch_id, five_hour_used],
            )
            .map_err(|e| e.to_string())?;
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn add_consumption(&self, batch_id: &str, delta: f64) -> Result<Batch, String> {
        self.connection
            .execute(
                "UPDATE batches SET consumed_points=MIN(100.0, consumed_points + ?2)
                 WHERE id=?1 AND budget_mode='percentage'",
                params![batch_id, delta.max(0.0)],
            )
            .map_err(|e| e.to_string())?;
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn add_token_consumption(&self, batch_id: &str, tokens: i64) -> Result<Batch, String> {
        self.connection
            .execute(
                "UPDATE batches SET consumed_tokens=consumed_tokens + ?2
                 WHERE id=?1 AND budget_mode='tokens'",
                params![batch_id, tokens.max(0)],
            )
            .map_err(|e| e.to_string())?;
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn exhaust_token_budget(&self, batch_id: &str) -> Result<Batch, String> {
        self.connection
            .execute(
                "UPDATE batches SET consumed_tokens=COALESCE(token_cap, consumed_tokens)
                 WHERE id=?1 AND budget_mode='tokens'",
                params![batch_id],
            )
            .map_err(|e| e.to_string())?;
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn reconcile_consumption(&self, batch_id: &str, weekly_used: f64) -> Result<Batch, String> {
        self.connection.execute(
            "UPDATE batches SET consumed_points=MAX(consumed_points, MAX(0, ?2 - baseline_weekly_used_percent))
             WHERE id=?1 AND budget_mode='percentage' AND baseline_weekly_used_percent IS NOT NULL",
            params![batch_id, weekly_used],
        ).map_err(|e| e.to_string())?;
        self.batch(batch_id)?
            .ok_or_else(|| "batch not found".to_string())
    }

    pub fn start_run(&self, task_id: &str, usage_json: &str) -> Result<String, String> {
        let run_id = new_id("run");
        self.connection.execute(
            "INSERT INTO runs (id,task_id,started_at,status,usage_before_json) VALUES (?1,?2,?3,'running',?4)",
            params![run_id, task_id, now_epoch(), usage_json],
        ).map_err(|e| e.to_string())?;
        Ok(run_id)
    }

    pub fn finish_run(&self, run_id: &str, finish: RunFinish<'_>) -> Result<(), String> {
        let token_usage_state = match (finish.tokens_used, finish.transcript) {
            (Some(_), Some(_)) => "reported",
            (Some(_), None) => "not_launched",
            (None, _) => "unavailable",
        };
        self.connection.execute(
            "UPDATE runs SET finished_at=?2,status=?3,usage_after_json=?4,session_id=?5,transcript_path=?6,tokens_used=?7,token_usage_state=?8,error=?9 WHERE id=?1",
            params![run_id, now_epoch(), finish.status, finish.usage_json, finish.session_id,
                finish.transcript, finish.tokens_used, token_usage_state, finish.error],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn batch_by_idempotency(&self, key: &str) -> Result<Option<Batch>, String> {
        self.connection.query_row(
            "SELECT id,idempotency_key,budget_mode,token_cap,consumed_tokens,cap_percent,cap_basis,created_at,window_reset_at,baseline_weekly_used_percent,allowance_points,consumed_points,five_hour_cap_percent,five_hour_window_reset_at,baseline_five_hour_used_percent,five_hour_allowance_points,five_hour_consumed_points FROM batches WHERE idempotency_key=?1",
            params![key], row_to_batch,
        ).optional().map_err(|e| e.to_string())
    }

    fn attempt_by_idempotency(
        &self,
        key: &str,
        source_task_id: &str,
    ) -> Result<Option<AttemptCreateResult>, String> {
        let Some(batch) = self.batch_by_idempotency(key)? else {
            return Ok(None);
        };
        let tasks = self.tasks_for_batch(&batch.id)?;
        if tasks.len() != 1
            || tasks[0].source_task_id.as_deref() != Some(source_task_id)
            || tasks[0].attempt_kind.is_none()
        {
            return Err("idempotency_key is already used by another operation".to_string());
        }
        Ok(Some(AttemptCreateResult {
            batch,
            task: tasks.into_iter().next().expect("length checked"),
            idempotent_replay: true,
        }))
    }

    fn tasks_for_batch(&self, batch_id: &str) -> Result<Vec<Task>, String> {
        query_tasks_for_batch(&self.connection, batch_id, true)
    }

    fn batch_edit_is_active(&self, batch_id: &str, now: i64) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM batches
                   WHERE id=?1 AND edit_session_id IS NOT NULL
                     AND edit_heartbeat_at>=?2
                 )",
                params![batch_id, now - BATCH_EDIT_LEASE_SECONDS],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }
}

fn query_tasks_for_batch(
    connection: &Connection,
    batch_id: &str,
    include_archived: bool,
) -> Result<Vec<Task>, String> {
    let archived = if include_archived {
        ""
    } else {
        " AND archived_at IS NULL"
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id,dependency_type,source_task_id,attempt_kind,attempt_number,permission_profile FROM tasks WHERE batch_id=?1{archived} ORDER BY position ASC, id ASC"
        ))
        .map_err(|error| error.to_string())?;
    let tasks = statement
        .query_map(params![batch_id], row_to_task)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(tasks)
}

fn batch_tasks_are_editable(tasks: &[Task]) -> bool {
    tasks.first().is_some_and(|task| {
        task.status == "scheduled"
            && task.depends_on_task_id.is_none()
            && task.source_task_id.is_none()
            && task.attempt_kind.is_none()
    }) && tasks.iter().all(|task| {
        matches!(task.status.as_str(), "scheduled" | "cancelled")
            && task.dependency_type == "success"
            && task.source_task_id.is_none()
            && task.attempt_kind.is_none()
    })
}

fn validate_batch_input(input: &ScheduleBatchInput) -> Result<(), String> {
    if input.idempotency_key.trim().is_empty() {
        return Err("idempotency_key is required".to_string());
    }
    requested_budget(input)?;
    validate_five_hour_cap(input.five_hour_cap_percent)?;
    if input.tasks.is_empty() {
        return Err("at least one task is required".to_string());
    }
    for task in &input.tasks {
        if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
            return Err("every task needs a title and prompt".to_string());
        }
    }
    validate_draft_triggers(&input.tasks)?;
    Ok(())
}

fn require_networked_confirmation(
    permission_profile: PermissionProfile,
    networked_confirmed: bool,
) -> Result<(), String> {
    if permission_profile == PermissionProfile::Networked && !networked_confirmed {
        return Err(
            "networked permission_profile requires separate explicit networked_confirmed=true acknowledgement"
                .to_string(),
        );
    }
    Ok(())
}

fn attempt_kind_for_status(status: &str) -> Result<AttemptKind, String> {
    match status {
        "quota_interrupted" | "quota_skipped" => Ok(AttemptKind::QuotaResume),
        "failed" | "blocked" | "missed" | "cancelled" => Ok(AttemptKind::Retry),
        "completed" | "scheduled" | "running" => Err(format!(
            "task status '{status}' is not eligible for retry or resume"
        )),
        _ => Err(format!("unknown task status '{status}'")),
    }
}

fn is_known_task_status(status: &str) -> bool {
    matches!(
        status,
        "scheduled"
            | "running"
            | "completed"
            | "failed"
            | "blocked"
            | "missed"
            | "cancelled"
            | "quota_skipped"
            | "quota_interrupted"
    )
}

fn requested_retry_budget(options: &RetryTaskOptions) -> Result<RequestedBudget, String> {
    requested_budget(&ScheduleBatchInput {
        idempotency_key: "retry-preview".to_string(),
        budget_mode: Some(options.budget_mode),
        weekly_cap_percent: options.weekly_cap_percent,
        token_cap: options.token_cap,
        cap_percent: None,
        five_hour_cap_percent: options.five_hour_cap_percent,
        networked_confirmed: false,
        tasks: Vec::new(),
    })
}

fn validate_five_hour_cap(cap: Option<f64>) -> Result<(), String> {
    if let Some(cap) = cap {
        if !cap.is_finite() || cap <= 0.0 || cap > 100.0 {
            return Err("five_hour_cap_percent must be greater than 0 and at most 100".to_string());
        }
    }
    Ok(())
}

struct RequestedBudget {
    mode: BudgetMode,
    cap_percent: f64,
    cap_basis: &'static str,
    token_cap: Option<i64>,
}

fn requested_budget(input: &ScheduleBatchInput) -> Result<RequestedBudget, String> {
    let mode = input.budget_mode.unwrap_or_else(|| {
        if input.token_cap.is_some() {
            BudgetMode::Tokens
        } else {
            BudgetMode::Percentage
        }
    });
    match mode {
        BudgetMode::Percentage => {
            if input.token_cap.is_some() {
                return Err("token_cap is only valid with budget_mode=tokens".to_string());
            }
            let (value, basis) = match (input.weekly_cap_percent, input.cap_percent) {
                (Some(value), None) => (value, "total_weekly_percent"),
                (None, Some(value)) if input.budget_mode.is_none() => {
                    (value, "remaining_percent")
                }
                (Some(_), Some(_)) => {
                    return Err(
                        "provide weekly_cap_percent, not both weekly_cap_percent and legacy cap_percent"
                            .to_string(),
                    )
                }
                (None, Some(_)) => {
                    return Err("legacy cap_percent cannot be combined with budget_mode".to_string())
                }
                (None, None) => return Err("weekly_cap_percent is required".to_string()),
            };
            if !value.is_finite() || value <= 0.0 || value > 100.0 {
                return Err(format!(
                    "{} must be greater than 0 and at most 100",
                    if basis == "total_weekly_percent" {
                        "weekly_cap_percent"
                    } else {
                        "cap_percent"
                    }
                ));
            }
            Ok(RequestedBudget {
                mode,
                cap_percent: value,
                cap_basis: basis,
                token_cap: None,
            })
        }
        BudgetMode::Tokens => {
            if input.weekly_cap_percent.is_some() || input.cap_percent.is_some() {
                return Err("percentage caps are not valid with budget_mode=tokens".to_string());
            }
            let token_cap = input
                .token_cap
                .ok_or_else(|| "token_cap is required".to_string())?;
            if !(1..=1_000_000_000).contains(&token_cap) {
                return Err("token_cap must be from 1 to 1000000000".to_string());
            }
            Ok(RequestedBudget {
                mode,
                cap_percent: 0.0,
                cap_basis: "total_tokens",
                token_cap: Some(token_cap),
            })
        }
    }
}

struct NormalizedDraft {
    run_at: i64,
    timezone: String,
    model: String,
    effort: String,
}

fn validate_draft_triggers(drafts: &[TaskDraft]) -> Result<(), String> {
    for (index, draft) in drafts.iter().enumerate() {
        let continuation = draft
            .continue_from_task_id
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        if draft.continue_from_task_id.is_some() && continuation.is_none() {
            return Err("continue_from_task_id cannot be empty".to_string());
        }
        let trigger_count = usize::from(draft.run_at.is_some())
            + usize::from(draft.after_previous)
            + usize::from(continuation.is_some());
        if trigger_count != 1 {
            return Err(
                "each task must use exactly one of run_at, after_previous, or continue_from_task_id"
                    .to_string(),
            );
        }
        if draft.after_previous && index == 0 {
            return Err("the first task cannot use after_previous".to_string());
        }
    }
    Ok(())
}

fn normalize_drafts<F>(
    drafts: &[TaskDraft],
    continuation_reset_at: F,
) -> Result<Vec<NormalizedDraft>, String>
where
    F: Fn(&str, &str) -> Result<i64, String>,
{
    validate_draft_triggers(drafts)?;
    let mut normalized: Vec<NormalizedDraft> = Vec::with_capacity(drafts.len());
    for (index, draft) in drafts.iter().enumerate() {
        validate_cwd(&draft.cwd)?;
        let timezone = draft
            .timezone
            .clone()
            .unwrap_or_else(crate::config::system_timezone);
        validate_timezone(&timezone)?;
        let run_at = if draft.after_previous {
            normalized[index - 1].run_at
        } else if let Some(predecessor_id) = draft.continue_from_task_id.as_deref() {
            continuation_reset_at(predecessor_id, &draft.cwd)?
        } else {
            let value = draft
                .run_at
                .as_deref()
                .ok_or_else(|| "run_at is required unless after_previous=true".to_string())?;
            let (run_at, explicit_offset) = parse_future_time(value)?;
            validate_timezone_at(&timezone, run_at, explicit_offset)?;
            run_at
        };
        let routed = route(draft.difficulty);
        let model = draft.model.clone().unwrap_or(routed.model);
        let effort = draft.effort.clone().unwrap_or(routed.effort);
        validate_route(&model, &effort)?;
        normalized.push(NormalizedDraft {
            run_at,
            timezone,
            model,
            effort,
        });
    }
    Ok(normalized)
}

fn parse_future_time(value: &str) -> Result<(i64, i32), String> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| "run_at must be RFC3339 with an explicit UTC offset".to_string())?;
    if parsed.timestamp() <= now_epoch() {
        return Err("run_at must be in the future".to_string());
    }
    Ok((parsed.timestamp(), parsed.offset().local_minus_utc()))
}

fn validate_timezone(value: &str) -> Result<(), String> {
    value
        .parse::<Tz>()
        .map(|_| ())
        .map_err(|_| format!("invalid IANA timezone '{value}'"))
}

fn validate_timezone_at(value: &str, timestamp: i64, explicit_offset: i32) -> Result<(), String> {
    let timezone = value
        .parse::<Tz>()
        .map_err(|_| format!("invalid IANA timezone '{value}'"))?;
    let utc = Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .ok_or_else(|| "run_at is outside the supported timestamp range".to_string())?;
    let expected = utc
        .with_timezone(&timezone)
        .offset()
        .fix()
        .local_minus_utc();
    if expected != explicit_offset {
        return Err(format!(
            "run_at UTC offset does not match timezone '{value}' at that instant (possible DST ambiguity)"
        ));
    }
    Ok(())
}

fn validate_cwd(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err("cwd must be an absolute path".to_string());
    }
    if !path.is_dir() {
        return Err(format!("cwd does not exist or is not a directory: {value}"));
    }
    Ok(())
}

fn row_to_batch(row: &Row<'_>) -> rusqlite::Result<Batch> {
    let budget_mode: String = row.get(2)?;
    let cap_percent: f64 = row.get(5)?;
    Ok(Batch {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        budget_mode: budget_mode.clone(),
        weekly_cap_percent: (budget_mode == "percentage").then_some(cap_percent),
        token_cap: row.get(3)?,
        consumed_tokens: row.get(4)?,
        cap_percent,
        cap_basis: row.get(6)?,
        created_at: row.get(7)?,
        window_reset_at: row.get(8)?,
        baseline_weekly_used_percent: row.get(9)?,
        allowance_points: row.get(10)?,
        consumed_points: row.get(11)?,
        five_hour_cap_percent: row.get(12)?,
        five_hour_window_reset_at: row.get(13)?,
        baseline_five_hour_used_percent: row.get(14)?,
        five_hour_allowance_points: row.get(15)?,
        five_hour_consumed_points: row.get(16)?,
    })
}

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let timestamp: i64 = row.get(6)?;
    let timezone: String = row.get(7)?;
    let run_at_iso = timestamp_in_timezone(timestamp, &timezone);
    Ok(Task {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        title: row.get(2)?,
        prompt: row.get(3)?,
        success_criteria: row.get(4)?,
        cwd: row.get(5)?,
        run_at: timestamp,
        run_at_iso,
        position: row.get(15)?,
        depends_on_task_id: row.get(16)?,
        dependency_type: row.get(17)?,
        source_task_id: row.get(18)?,
        attempt_kind: row.get(19)?,
        attempt_number: row.get(20)?,
        timezone,
        difficulty: row.get(8)?,
        model: row.get(9)?,
        effort: row.get(10)?,
        permission_profile: permission_profile_from_row(row, 21)?,
        status: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        last_error: row.get(14)?,
    })
}

fn permission_profile_from_row(row: &Row<'_>, index: usize) -> rusqlite::Result<PermissionProfile> {
    let value: String = row.get(index)?;
    PermissionProfile::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })
}

fn timestamp_in_timezone(timestamp: i64, timezone: &str) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .and_then(|utc| {
            timezone
                .parse::<Tz>()
                .ok()
                .map(|tz| utc.with_timezone(&tz).to_rfc3339())
        })
        .unwrap_or_else(|| timestamp.to_string())
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if !names.iter().any(|name| name == column) {
        connection
            .execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn read_transcript_excerpt(path: &str) -> Option<String> {
    const MAX_BYTES: usize = 16 * 1024;
    let content = fs::read(path).ok()?;
    let start = content.len().saturating_sub(MAX_BYTES);
    Some(String::from_utf8_lossy(&content[start..]).into_owned())
}

fn backfill_run_token_usage(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "UPDATE runs
             SET tokens_used=0,token_usage_state='not_launched'
             WHERE finished_at IS NOT NULL AND tokens_used IS NULL AND transcript_path IS NULL",
            [],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "UPDATE runs
             SET token_usage_state=CASE WHEN transcript_path IS NULL THEN 'not_launched' ELSE 'reported' END
             WHERE tokens_used IS NOT NULL",
            [],
        )
        .map_err(|e| e.to_string())?;

    let candidates = {
        let mut statement = connection
            .prepare(
                "SELECT id,transcript_path FROM runs
                 WHERE finished_at IS NOT NULL AND tokens_used IS NULL AND transcript_path IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let mapped = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for (run_id, transcript_path) in candidates {
        let Ok(content) = std::fs::read_to_string(transcript_path) else {
            continue;
        };
        let Some(tokens) = token_usage(&content) else {
            continue;
        };
        connection
            .execute(
                "UPDATE runs SET tokens_used=?2,token_usage_state='reported' WHERE id=?1",
                params![run_id, tokens],
            )
            .map_err(|e| e.to_string())?;
    }
    connection
        .execute(
            "UPDATE runs SET token_usage_state='unavailable'
             WHERE finished_at IS NOT NULL AND tokens_used IS NULL",
            [],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn summarize_usage(runs: &[StoredUsageRun], since: i64) -> UsagePeriodStats {
    let mut stats = UsagePeriodStats {
        since,
        run_count: 0,
        tokens_used: 0,
        tokens_unavailable_runs: 0,
        status_counts: BTreeMap::new(),
    };
    for run in runs.iter().filter(|run| run.started_at >= since) {
        stats.run_count += 1;
        if let Some(tokens) = run.tokens_used {
            stats.tokens_used = stats.tokens_used.saturating_add(tokens);
        } else {
            stats.tokens_unavailable_runs += 1;
        }
        *stats.status_counts.entry(run.status.clone()).or_default() += 1;
    }
    stats
}

fn add_usage(day: &mut DailyUsageStats, run: &StoredUsageRun) {
    day.run_count += 1;
    if let Some(tokens) = run.tokens_used {
        day.tokens_used = day.tokens_used.saturating_add(tokens);
    } else {
        day.tokens_unavailable_runs += 1;
    }
    *day.status_counts.entry(run.status.clone()).or_default() += 1;
}

fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{nanos:x}-{:x}-{counter:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::RateWindow;
    use chrono::Duration;
    #[test]
    fn rejects_bad_caps() {
        let input = ScheduleBatchInput {
            idempotency_key: "key".into(),
            budget_mode: Some(BudgetMode::Percentage),
            weekly_cap_percent: Some(0.0),
            token_cap: None,
            cap_percent: None,
            five_hour_cap_percent: None,
            networked_confirmed: false,
            tasks: vec![],
        };
        assert!(validate_batch_input(&input).is_err());

        let mut input = sample_input("bad-five-hour-cap", 1.0);
        input.five_hour_cap_percent = Some(0.0);
        assert!(validate_batch_input(&input).is_err());
    }
    #[test]
    fn terminal_statuses_are_stable() {
        assert!(TERMINAL_STATUSES.contains(&"quota_interrupted"));
        assert!(!TERMINAL_STATUSES.contains(&"running"));
    }

    #[test]
    fn schedule_validation_reuses_rules_without_persisting() {
        let store = Store::in_memory().unwrap();
        let input = sample_input("validation-only", 1.0);
        store.validate_schedule_batch(&input).unwrap();
        assert!(store
            .batch_by_idempotency("validation-only")
            .unwrap()
            .is_none());
        assert!(store.list_tasks(None).unwrap().is_empty());
    }

    #[test]
    fn old_client_payload_defaults_every_task_to_restricted() {
        let run_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
        let input: ScheduleBatchInput = serde_json::from_value(serde_json::json!({
            "idempotency_key": "old-client-profile-default",
            "budget_mode": "percentage",
            "weekly_cap_percent": 1,
            "tasks": [{
                "title": "legacy client",
                "prompt": "safe work",
                "cwd": "/tmp",
                "run_at": run_at,
                "timezone": "UTC",
                "difficulty": "simple"
            }]
        }))
        .unwrap();
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        assert_eq!(
            created.tasks[0].permission_profile,
            PermissionProfile::Restricted
        );
    }

    #[test]
    fn networked_writes_require_separate_confirmation_and_remain_race_safe() {
        let mut store = Store::in_memory().unwrap();
        let mut input = sample_input("networked-confirmation", 1.0);
        input.tasks[0].permission_profile = PermissionProfile::Networked;
        assert!(store
            .schedule_batch(input.clone())
            .unwrap_err()
            .contains("networked_confirmed=true"));
        assert!(store.list_tasks(None).unwrap().is_empty());
        input.networked_confirmed = true;
        let task = store.schedule_batch(input).unwrap().tasks.remove(0);
        assert_eq!(task.permission_profile, PermissionProfile::Networked);

        let error = store
            .update_task(
                &task.id,
                TaskUpdate {
                    permission_profile: Some(PermissionProfile::Networked),
                    ..TaskUpdate::default()
                },
            )
            .unwrap_err();
        assert!(error.contains("networked_confirmed=true"));
        store
            .update_task(
                &task.id,
                TaskUpdate {
                    permission_profile: Some(PermissionProfile::Restricted),
                    ..TaskUpdate::default()
                },
            )
            .unwrap();
        assert!(store.claim_task(&task.id).unwrap());
        assert!(store
            .update_task(
                &task.id,
                TaskUpdate {
                    permission_profile: Some(PermissionProfile::Networked),
                    networked_confirmed: true,
                    ..TaskUpdate::default()
                },
            )
            .unwrap_err()
            .contains("only scheduled tasks"));
    }

    #[test]
    fn retry_inherits_profile_and_confirmed_change_is_explicit() {
        let mut store = Store::in_memory().unwrap();
        let mut input = sample_input("networked-retry-source", 1.0);
        input.tasks[0].permission_profile = PermissionProfile::Networked;
        input.networked_confirmed = true;
        let source = store.schedule_batch(input).unwrap().tasks.remove(0);
        store
            .set_status(&source.id, "failed", Some("fixture"))
            .unwrap();

        let inherited = retry_options(true);
        let preview = store.preview_retry_task(&source.id, &inherited).unwrap();
        assert_eq!(
            preview.task.permission_profile,
            PermissionProfile::Networked
        );
        let error = store
            .create_retry_task(
                &source.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "networked-retry-unconfirmed".into(),
                    confirmed: true,
                    options: inherited.clone(),
                },
            )
            .unwrap_err();
        assert!(error.contains("networked_confirmed=true"));

        let mut restricted = inherited;
        restricted.permission_profile = Some(PermissionProfile::Restricted);
        let created = store
            .create_retry_task(
                &source.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "networked-retry-changed".into(),
                    confirmed: true,
                    options: restricted,
                },
            )
            .unwrap();
        assert_eq!(
            created.task.permission_profile,
            PermissionProfile::Restricted
        );
    }

    #[test]
    fn rejects_dst_offset_mismatch() {
        let summer = DateTime::parse_from_rfc3339("2030-07-01T12:00:00+02:00").unwrap();
        assert!(validate_timezone_at("Europe/Paris", summer.timestamp(), 7200).is_ok());
        assert!(validate_timezone_at("Europe/Paris", summer.timestamp(), 3600).is_err());
    }

    #[test]
    fn task_timestamp_is_returned_in_its_local_timezone() {
        let mut input = sample_input("local-time", 1.0);
        input.tasks[0].run_at = Some("2030-07-01T12:00:00+02:00".into());
        input.tasks[0].timezone = Some("Europe/Paris".into());
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        assert_eq!(created.tasks[0].run_at_iso, "2030-07-01T12:00:00+02:00");
    }

    #[test]
    fn task_pages_are_twenty_rows_and_default_to_newest_first() {
        let mut input = sample_input("paged-tasks", 5.0);
        let first = input.tasks.remove(0);
        input.tasks = (0..25)
            .map(|index| TaskDraft {
                title: format!("Task {index:02}"),
                prompt: first.prompt.clone(),
                success_criteria: first.success_criteria.clone(),
                cwd: first.cwd.clone(),
                run_at: (index == 0).then(|| first.run_at.clone().unwrap()),
                after_previous: index != 0,
                continue_from_task_id: None,
                timezone: first.timezone.clone(),
                difficulty: first.difficulty,
                model: None,
                effort: None,
                permission_profile: PermissionProfile::Restricted,
            })
            .collect();
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        for (index, task) in created.tasks.iter().enumerate() {
            store
                .connection
                .execute(
                    "UPDATE tasks SET created_at=?2 WHERE id=?1",
                    params![task.id, 1_000 + index as i64],
                )
                .unwrap();
        }

        let first_page = store.list_task_page(None, TaskSort::Newest, 1).unwrap();
        assert_eq!(first_page.items.len(), 20);
        assert_eq!(first_page.total, 25);
        assert_eq!(first_page.total_pages, 2);
        assert_eq!(first_page.items[0].title, "Task 24");
        let second_page = store.list_task_page(None, TaskSort::Newest, 2).unwrap();
        assert_eq!(second_page.items.len(), 5);
        assert_eq!(second_page.items[0].title, "Task 04");
        let alphabetical = store
            .list_task_page(None, TaskSort::TitleAscending, 1)
            .unwrap();
        assert_eq!(alphabetical.items[0].title, "Task 00");
        assert!(store.list_task_page(None, TaskSort::Newest, 0).is_err());
    }

    #[test]
    fn dashboard_pages_group_chained_tasks_by_batch() {
        let mut input = sample_input("grouped-dashboard", 5.0);
        let first = input.tasks[0].clone();
        input.tasks.push(TaskDraft {
            title: "second".into(),
            prompt: "continue the work".into(),
            success_criteria: "chain complete".into(),
            cwd: first.cwd,
            run_at: None,
            after_previous: true,
            continue_from_task_id: None,
            timezone: first.timezone,
            difficulty: Difficulty::Simple,
            model: None,
            effort: None,
            permission_profile: PermissionProfile::Restricted,
        });
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();

        let page = store.list_batch_page(None, TaskSort::Newest, 1).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].batch.id, created.batch.id);
        assert_eq!(page.items[0].tasks.len(), 2);
        assert!(page.items[0].editable);
    }

    #[test]
    fn active_batch_edit_blocks_claim_and_atomically_replaces_the_chain() {
        let mut input = sample_input("edit-batch", 5.0);
        let first_draft = input.tasks[0].clone();
        input.tasks.push(TaskDraft {
            title: "removed task".into(),
            prompt: "remove me".into(),
            success_criteria: "not retained".into(),
            cwd: first_draft.cwd,
            run_at: None,
            after_previous: true,
            continue_from_task_id: None,
            timezone: first_draft.timezone,
            difficulty: Difficulty::Simple,
            model: None,
            effort: None,
            permission_profile: PermissionProfile::Restricted,
        });
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        let session = store.begin_batch_edit(&created.batch.id).unwrap();
        assert!(!store.claim_task(&created.tasks[0].id).unwrap());

        let first = BatchEditTaskInput {
            task_id: Some(created.tasks[0].id.clone()),
            title: "edited first".into(),
            prompt: created.tasks[0].prompt.clone(),
            success_criteria: created.tasks[0].success_criteria.clone(),
            cwd: created.tasks[0].cwd.clone(),
            run_at: Some((Utc::now() + Duration::hours(4)).to_rfc3339()),
            timezone: "UTC".into(),
            difficulty: Difficulty::Standard,
            model: "gpt-6-sol".into(),
            effort: "medium".into(),
            permission_profile: PermissionProfile::Restricted,
        };
        let added = BatchEditTaskInput {
            task_id: None,
            title: "new second".into(),
            prompt: "finish the edited chain".into(),
            success_criteria: "done".into(),
            cwd: "/tmp".into(),
            run_at: None,
            timezone: "UTC".into(),
            difficulty: Difficulty::Simple,
            model: "gpt-5.6-luna".into(),
            effort: "low".into(),
            permission_profile: PermissionProfile::Restricted,
        };
        let updated = store
            .update_batch(
                &created.batch.id,
                BatchEditInput {
                    edit_session_id: session.edit_session_id,
                    networked_confirmed: false,
                    tasks: vec![first, added],
                },
            )
            .unwrap();

        assert_eq!(updated.tasks.len(), 2);
        assert_eq!(updated.tasks[0].id, created.tasks[0].id);
        assert_eq!(updated.tasks[0].title, "edited first");
        assert_eq!(updated.tasks[1].title, "new second");
        assert_eq!(
            updated.tasks[1].depends_on_task_id.as_deref(),
            Some(updated.tasks[0].id.as_str())
        );
        let removed = store.task(&created.tasks[1].id).unwrap().unwrap();
        assert_eq!(removed.status, "cancelled");
        assert!(store.claim_task(&updated.tasks[0].id).unwrap());
    }

    #[test]
    fn passed_start_time_must_change_before_batch_edit_can_save() {
        let mut store = Store::in_memory().unwrap();
        let created = store
            .schedule_batch(sample_input("edit-passed-time", 1.0))
            .unwrap();
        let session = store.begin_batch_edit(&created.batch.id).unwrap();
        let task = &created.tasks[0];
        let error = store
            .update_batch(
                &created.batch.id,
                BatchEditInput {
                    edit_session_id: session.edit_session_id.clone(),
                    networked_confirmed: false,
                    tasks: vec![BatchEditTaskInput {
                        task_id: Some(task.id.clone()),
                        title: task.title.clone(),
                        prompt: task.prompt.clone(),
                        success_criteria: task.success_criteria.clone(),
                        cwd: task.cwd.clone(),
                        run_at: Some((Utc::now() - Duration::minutes(1)).to_rfc3339()),
                        timezone: "UTC".into(),
                        difficulty: Difficulty::Standard,
                        model: "gpt-6-sol".into(),
                        effort: "medium".into(),
                        permission_profile: PermissionProfile::Restricted,
                    }],
                },
            )
            .unwrap_err();
        assert_eq!(
            error,
            "the batch start time has passed; choose a future time before saving changes"
        );
        assert!(!store.claim_task(&task.id).unwrap());
        store
            .cancel_batch_edit(&created.batch.id, &session.edit_session_id)
            .unwrap();
        assert!(store.claim_task(&task.id).unwrap());
    }

    #[test]
    fn archive_hides_task_but_preserves_run_usage_metadata() {
        let mut store = Store::in_memory().unwrap();
        let task = store
            .schedule_batch(sample_input("archive-completed", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        let now = now_epoch();
        store
            .connection
            .execute(
                "INSERT INTO runs
                 (id,task_id,started_at,finished_at,status,tokens_used,token_usage_state)
                 VALUES ('run-archive',?1,?2,?2,'completed',321,'reported')",
                params![task.id, now],
            )
            .unwrap();
        store.set_status(&task.id, "completed", None).unwrap();

        let archived = store.archive_task(&task.id).unwrap();
        assert_eq!(archived.status, "completed");
        assert_eq!(archived.preserved_runs, 1);
        assert!(store.list_tasks(None).unwrap().is_empty());
        assert_eq!(
            store
                .list_task_page(None, TaskSort::Newest, 1)
                .unwrap()
                .total,
            0
        );
        assert_eq!(store.task_status(&task.id).unwrap().unwrap().runs.len(), 1);
        assert_eq!(
            store
                .task_usage_stats(now, "UTC")
                .unwrap()
                .last_week
                .tokens_used,
            321
        );

        let replay = store.archive_task(&task.id).unwrap();
        assert_eq!(replay.archived_at, archived.archived_at);
        assert_eq!(replay.preserved_runs, 1);
    }

    #[test]
    fn archive_cancels_pending_task_and_rejects_running_task() {
        let mut store = Store::in_memory().unwrap();
        let pending = store
            .schedule_batch(sample_input("archive-pending", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        assert_eq!(store.archive_task(&pending.id).unwrap().status, "cancelled");
        assert!(!store.claim_task(&pending.id).unwrap());

        let running = store
            .schedule_batch(sample_input("archive-running", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        assert!(store.claim_task(&running.id).unwrap());
        assert!(store
            .archive_task(&running.id)
            .unwrap_err()
            .contains("must be stopped"));
    }

    #[test]
    fn running_task_stop_request_is_idempotent_and_clearable() {
        let mut store = Store::in_memory().unwrap();
        let task = store
            .schedule_batch(sample_input("stop-running", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        assert!(store.request_task_stop(&task.id).is_err());
        assert!(store.claim_task(&task.id).unwrap());
        assert_eq!(store.request_task_stop(&task.id).unwrap().status, "running");
        assert_eq!(store.request_task_stop(&task.id).unwrap().status, "running");
        assert!(store.task_stop_requested(&task.id).unwrap());
        store.clear_task_stop_request(&task.id).unwrap();
        assert!(!store.task_stop_requested(&task.id).unwrap());
    }

    fn sample_input(key: &str, cap_percent: f64) -> ScheduleBatchInput {
        let run_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
        ScheduleBatchInput {
            idempotency_key: key.into(),
            budget_mode: Some(BudgetMode::Percentage),
            weekly_cap_percent: Some(cap_percent),
            token_cap: None,
            cap_percent: None,
            five_hour_cap_percent: None,
            networked_confirmed: false,
            tasks: vec![TaskDraft {
                title: "test".into(),
                prompt: "make a harmless change".into(),
                success_criteria: "tests pass".into(),
                cwd: "/tmp".into(),
                run_at: Some(run_at),
                after_previous: false,
                continue_from_task_id: None,
                timezone: Some("UTC".into()),
                difficulty: Difficulty::Standard,
                model: None,
                effort: None,
                permission_profile: PermissionProfile::Restricted,
            }],
        }
    }

    fn retry_options(with_run_at: bool) -> RetryTaskOptions {
        RetryTaskOptions {
            budget_mode: BudgetMode::Percentage,
            weekly_cap_percent: Some(2.0),
            token_cap: None,
            five_hour_cap_percent: Some(4.0),
            run_at: with_run_at.then(|| (Utc::now() + Duration::hours(3)).to_rfc3339()),
            timezone: Some("UTC".into()),
            model: Some("gpt-5.6-luna".into()),
            effort: Some("low".into()),
            permission_profile: None,
            networked_confirmed: false,
        }
    }

    #[test]
    fn retry_eligibility_covers_every_source_status() {
        for status in ["quota_interrupted", "quota_skipped"] {
            assert_eq!(
                attempt_kind_for_status(status).unwrap(),
                AttemptKind::QuotaResume
            );
        }
        for status in ["failed", "blocked", "missed", "cancelled"] {
            assert_eq!(attempt_kind_for_status(status).unwrap(), AttemptKind::Retry);
        }
        for status in ["completed", "scheduled", "running"] {
            assert!(attempt_kind_for_status(status).is_err(), "{status}");
        }
    }

    #[test]
    fn retry_preview_is_read_only_and_complete() {
        let mut store = Store::in_memory().unwrap();
        let source = store
            .schedule_batch(sample_input("retry-preview-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        store
            .set_status(&source.id, "failed", Some("fixture"))
            .unwrap();
        let before = (
            store
                .connection
                .query_row("SELECT COUNT(*) FROM batches", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            store
                .connection
                .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                .unwrap(),
        );
        let preview = store
            .preview_retry_task(&source.id, &retry_options(true))
            .unwrap();
        let after = (
            store
                .connection
                .query_row("SELECT COUNT(*) FROM batches", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            store
                .connection
                .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
                .unwrap(),
        );
        assert_eq!(before, after);
        assert_eq!(preview.attempt_kind, "retry");
        assert_eq!(preview.attempt_number, 2);
        assert_eq!(preview.resume_mode, "fresh_session");
        assert_eq!(preview.task.prompt, source.prompt);
        assert_eq!(preview.budget.weekly_cap_percent, Some(2.0));
    }

    #[test]
    fn retry_creation_is_atomic_idempotent_and_preserves_source_history() {
        let mut store = Store::in_memory().unwrap();
        let source = store
            .schedule_batch(sample_input("retry-create-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        store
            .set_status(&source.id, "failed", Some("fixture failure"))
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO runs (id,task_id,started_at,finished_at,status,error) VALUES ('source-run',?1,1,2,'failed','fixture failure')",
                params![source.id],
            )
            .unwrap();
        let before = store.task_status(&source.id).unwrap().unwrap();
        let source_task_before = serde_json::to_string(&before.task).unwrap();
        let source_runs_before = serde_json::to_string(&before.runs).unwrap();
        let first = store
            .create_retry_task(
                &source.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "retry-create-key".into(),
                    confirmed: true,
                    options: retry_options(true),
                },
            )
            .unwrap();
        let replay = store
            .create_retry_task(
                &source.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "retry-create-key".into(),
                    confirmed: true,
                    options: retry_options(true),
                },
            )
            .unwrap();
        assert!(!first.idempotent_replay);
        assert!(replay.idempotent_replay);
        assert_eq!(first.task.id, replay.task.id);
        assert_eq!(first.batch.id, replay.batch.id);
        assert_eq!(first.batch.weekly_cap_percent, Some(2.0));
        assert_eq!(
            first.task.source_task_id.as_deref(),
            Some(source.id.as_str())
        );
        assert_eq!(first.task.attempt_kind.as_deref(), Some("retry"));
        assert_eq!(first.task.attempt_number, 2);
        assert_eq!(first.task.depends_on_task_id, None);
        assert_eq!(first.task.dependency_type, "success");
        let after = store.task_status(&source.id).unwrap().unwrap();
        assert_eq!(
            serde_json::to_string(&after.task).unwrap(),
            source_task_before
        );
        assert_eq!(
            serde_json::to_string(&after.runs).unwrap(),
            source_runs_before
        );
        assert_eq!(after.lineage.len(), 2);
    }

    #[test]
    fn retry_confirmation_requires_true_and_key_owned_by_attempt() {
        let mut store = Store::in_memory().unwrap();
        let source = store
            .schedule_batch(sample_input("retry-confirm-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        store
            .set_status(&source.id, "failed", Some("fixture"))
            .unwrap();
        let error = store
            .create_retry_task(
                &source.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "retry-unconfirmed".into(),
                    confirmed: false,
                    options: retry_options(true),
                },
            )
            .unwrap_err();
        assert!(error.contains("explicit confirmation"));
        let collision = store
            .create_retry_task(
                &source.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "retry-confirm-source".into(),
                    confirmed: true,
                    options: retry_options(true),
                },
            )
            .unwrap_err();
        assert!(collision.contains("already used by another operation"));
    }

    #[test]
    fn retry_idempotency_and_lineage_survive_reopen() {
        let path = std::env::temp_dir().join(new_id("limitwise-c04-reopen"));
        let (source_id, attempt_id) = {
            let mut store = Store::from_connection(Connection::open(&path).unwrap()).unwrap();
            let source = store
                .schedule_batch(sample_input("retry-reopen-source", 1.0))
                .unwrap()
                .tasks
                .remove(0);
            store
                .set_status(&source.id, "failed", Some("fixture"))
                .unwrap();
            let created = store
                .create_retry_task(
                    &source.id,
                    ConfirmedRetryTaskInput {
                        idempotency_key: "retry-reopen-key".into(),
                        confirmed: true,
                        options: retry_options(true),
                    },
                )
                .unwrap();
            (source.id, created.task.id)
        };
        let mut reopened = Store::from_connection(Connection::open(&path).unwrap()).unwrap();
        let replay = reopened
            .create_retry_task(
                &source_id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "retry-reopen-key".into(),
                    confirmed: true,
                    options: retry_options(true),
                },
            )
            .unwrap();
        assert!(replay.idempotent_replay);
        assert_eq!(replay.task.id, attempt_id);
        assert_eq!(reopened.task_lineage(&source_id).unwrap().len(), 2);
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn retry_transaction_rolls_back_partial_creation_before_reopen() {
        let path = std::env::temp_dir().join(new_id("limitwise-c04-crash"));
        let source_id = {
            let mut store = Store::from_connection(Connection::open(&path).unwrap()).unwrap();
            let source = store
                .schedule_batch(sample_input("retry-crash-source", 1.0))
                .unwrap()
                .tasks
                .remove(0);
            store
                .set_status(&source.id, "failed", Some("fixture"))
                .unwrap();
            {
                let transaction = store
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO batches (id,idempotency_key,cap_percent,cap_basis,budget_mode,created_at) VALUES ('partial-batch','retry-crash-key',1,'total_weekly_percent','percentage',1)",
                        [],
                    )
                    .unwrap();
            }
            source.id
        };
        let mut reopened = Store::from_connection(Connection::open(&path).unwrap()).unwrap();
        assert!(reopened
            .batch_by_idempotency("retry-crash-key")
            .unwrap()
            .is_none());
        let created = reopened
            .create_retry_task(
                &source_id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "retry-crash-key".into(),
                    confirmed: true,
                    options: retry_options(true),
                },
            )
            .unwrap();
        assert!(!created.idempotent_replay);
        assert_eq!(
            created.task.source_task_id.as_deref(),
            Some(source_id.as_str())
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn quota_resume_uses_reset_and_session_or_context_fallback_only() {
        let mut store = Store::in_memory().unwrap();
        let with_session = store
            .schedule_batch(sample_input("quota-attempt-session", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        let future_reset = now_epoch() + 600;
        mark_quota_limited(
            &store,
            &with_session.id,
            "quota_interrupted",
            future_reset,
            Some("session-c04"),
            Some("/tmp/c04.jsonl"),
        );
        let preview = store
            .preview_retry_task(&with_session.id, &retry_options(false))
            .unwrap();
        assert_eq!(preview.run_at, future_reset);
        assert_eq!(preview.provider_reset_at, Some(future_reset));
        assert_eq!(preview.resume_mode, "session_resume_or_context_fallback");
        let created = store
            .create_retry_task(
                &with_session.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "quota-attempt-session-key".into(),
                    confirmed: true,
                    options: retry_options(false),
                },
            )
            .unwrap();
        assert_eq!(created.task.attempt_kind.as_deref(), Some("quota_resume"));
        assert_eq!(created.task.dependency_type, "quota_reset");
        assert_eq!(
            created.task.depends_on_task_id.as_deref(),
            Some(with_session.id.as_str())
        );
        assert_eq!(
            store
                .continuation_context(&created.task)
                .unwrap()
                .unwrap()
                .session_id
                .as_deref(),
            Some("session-c04")
        );

        let fallback = store
            .schedule_batch(sample_input("quota-attempt-fallback", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        let passed_reset = now_epoch() - 60;
        mark_quota_limited(
            &store,
            &fallback.id,
            "quota_skipped",
            passed_reset,
            None,
            Some("/tmp/c04-fallback.jsonl"),
        );
        let fallback_preview = store
            .preview_retry_task(&fallback.id, &retry_options(false))
            .unwrap();
        assert_eq!(fallback_preview.run_at, passed_reset);
        assert_eq!(fallback_preview.provider_reset_at, Some(passed_reset));
        assert_eq!(fallback_preview.resume_mode, "context_fallback");
        let fallback_created = store
            .create_retry_task(
                &fallback.id,
                ConfirmedRetryTaskInput {
                    idempotency_key: "quota-attempt-fallback-key".into(),
                    confirmed: true,
                    options: retry_options(false),
                },
            )
            .unwrap();
        assert_eq!(fallback_created.task.run_at, fallback_preview.run_at);
    }

    #[test]
    fn quota_outcomes_without_valid_reset_metadata_allow_only_fresh_retry() {
        let mut store = Store::in_memory().unwrap();
        let source = store
            .schedule_batch(sample_input("quota-attempt-missing", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        store
            .set_status(&source.id, "quota_skipped", Some("quota"))
            .unwrap();
        assert!(store
            .preview_retry_task(&source.id, &retry_options(false))
            .unwrap_err()
            .contains("run_at is required for retry"));
        let missing_fallback = store
            .preview_retry_task(&source.id, &retry_options(true))
            .unwrap();
        assert_eq!(missing_fallback.attempt_kind, "retry");
        assert_eq!(missing_fallback.resume_mode, "fresh_session");

        mark_quota_limited(&store, &source.id, "quota_skipped", 0, None, None);
        assert!(store
            .preview_retry_task(&source.id, &retry_options(false))
            .unwrap_err()
            .contains("run_at is required for retry"));
        assert_eq!(
            store
                .preview_retry_task(&source.id, &retry_options(true))
                .unwrap()
                .attempt_kind,
            "retry"
        );
    }

    #[test]
    fn schedule_is_idempotent_and_routes_defaults() {
        let mut store = Store::in_memory().unwrap();
        let first = store.schedule_batch(sample_input("same", 50.0)).unwrap();
        let second = store.schedule_batch(sample_input("same", 50.0)).unwrap();
        assert!(!first.idempotent_replay);
        assert!(second.idempotent_replay);
        assert_eq!(first.batch.id, second.batch.id);
        assert_eq!(first.tasks[0].model, "gpt-6-sol");
        assert_eq!(first.batch.five_hour_cap_percent, None);
    }

    #[test]
    fn five_hour_cap_is_independent_per_batch_and_resets() {
        let mut store = Store::in_memory().unwrap();
        let mut first_input = sample_input("five-hour-first", 10.0);
        first_input.five_hour_cap_percent = Some(5.0);
        let mut second_input = sample_input("five-hour-second", 10.0);
        second_input.five_hour_cap_percent = Some(5.0);
        let first = store.schedule_batch(first_input).unwrap();
        let second = store.schedule_batch(second_input).unwrap();

        let first_budget = store
            .ensure_five_hour_window(&first.batch.id, 20.0, 100)
            .unwrap();
        let second_budget = store
            .ensure_five_hour_window(&second.batch.id, 20.0, 100)
            .unwrap();
        assert_eq!(first_budget.five_hour_allowance_points, 5.0);
        assert_eq!(second_budget.five_hour_allowance_points, 5.0);

        let consumed = store
            .add_five_hour_consumption(&first.batch.id, 5.0)
            .unwrap();
        assert_eq!(consumed.five_hour_consumed_points, 5.0);
        assert_eq!(
            store
                .batch(&second.batch.id)
                .unwrap()
                .unwrap()
                .five_hour_consumed_points,
            0.0
        );

        let reset = store
            .ensure_five_hour_window(&first.batch.id, 10.0, 200)
            .unwrap();
        assert_eq!(reset.five_hour_allowance_points, 5.0);
        assert_eq!(reset.five_hour_consumed_points, 0.0);
    }

    #[test]
    fn five_hour_cap_is_clamped_by_global_reserve() {
        let mut store = Store::in_memory().unwrap();
        let mut input = sample_input("five-hour-reserve", 10.0);
        input.five_hour_cap_percent = Some(20.0);
        let created = store.schedule_batch(input).unwrap();
        let budget = store
            .ensure_five_hour_window(&created.batch.id, 85.0, 100)
            .unwrap();
        assert_eq!(budget.five_hour_allowance_points, 5.0);
    }

    #[test]
    fn token_budget_is_shared_and_never_resets() {
        let mut input = sample_input("tokens", 1.0);
        input.budget_mode = Some(BudgetMode::Tokens);
        input.weekly_cap_percent = None;
        input.token_cap = Some(50_000);
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        assert_eq!(created.batch.budget_mode, "tokens");
        assert_eq!(created.batch.token_cap, Some(50_000));
        assert_eq!(created.batch.weekly_cap_percent, None);

        let consumed = store
            .add_token_consumption(&created.batch.id, 12_345)
            .unwrap();
        assert_eq!(consumed.consumed_tokens, 12_345);
        let unchanged = store
            .ensure_budget_window(&created.batch.id, 25.0, 200)
            .unwrap();
        assert_eq!(unchanged.consumed_tokens, 12_345);
    }

    #[test]
    fn usage_stats_cover_rolling_windows_daily_totals_and_individual_runs() {
        const NOW: i64 = 2_000_000_000;
        const DAY: i64 = 86_400;
        let mut store = Store::in_memory().unwrap();
        let percentage = store
            .schedule_batch(sample_input("stats-percentage", 1.0))
            .unwrap();
        let mut token_input = sample_input("stats-tokens", 1.0);
        token_input.budget_mode = Some(BudgetMode::Tokens);
        token_input.weekly_cap_percent = None;
        token_input.token_cap = Some(10_000);
        let tokens = store.schedule_batch(token_input).unwrap();

        let rows = [
            (
                "recent-percentage",
                &percentage.tasks[0].id,
                NOW - DAY,
                "completed",
                Some(100),
            ),
            (
                "recent-tokens",
                &tokens.tasks[0].id,
                NOW - 2 * DAY,
                "failed",
                Some(50),
            ),
            (
                "recent-unknown",
                &percentage.tasks[0].id,
                NOW - 3 * DAY,
                "quota_interrupted",
                None,
            ),
            (
                "month",
                &percentage.tasks[0].id,
                NOW - 10 * DAY,
                "completed",
                Some(200),
            ),
            (
                "year",
                &percentage.tasks[0].id,
                NOW - 40 * DAY,
                "completed",
                Some(300),
            ),
            (
                "expired",
                &percentage.tasks[0].id,
                NOW - 366 * DAY,
                "completed",
                Some(400),
            ),
        ];
        for (id, task_id, started_at, status, tokens_used) in rows {
            store
                .connection
                .execute(
                    "INSERT INTO runs (id,task_id,started_at,finished_at,status,tokens_used)
                 VALUES (?1,?2,?3,?3,?4,?5)",
                    params![id, task_id, started_at, status, tokens_used],
                )
                .unwrap();
        }

        let stats = store.task_usage_stats(NOW, "UTC").unwrap();
        assert_eq!(stats.last_year.run_count, 5);
        assert_eq!(stats.last_year.tokens_used, 650);
        assert_eq!(stats.last_month.run_count, 4);
        assert_eq!(stats.last_month.tokens_used, 350);
        assert_eq!(stats.last_week.run_count, 3);
        assert_eq!(stats.last_week.tokens_used, 150);
        assert_eq!(stats.last_week.tokens_unavailable_runs, 1);
        assert_eq!(stats.daily_last_7_days.len(), 7);
        assert_eq!(
            stats
                .daily_last_7_days
                .iter()
                .map(|day| day.tokens_used)
                .sum::<i64>(),
            150
        );
        assert_eq!(stats.individual_runs_last_7_days.len(), 3);
        assert!(stats
            .individual_runs_last_7_days
            .iter()
            .any(|run| run.budget_mode == "percentage" && run.tokens_used == Some(100)));
        assert!(stats
            .individual_runs_last_7_days
            .iter()
            .any(|run| run.budget_mode == "tokens" && run.tokens_used == Some(50)));
    }

    #[test]
    fn token_migration_backfills_transcripts_and_zero_token_non_launches() {
        let mut store = Store::in_memory().unwrap();
        let task = store
            .schedule_batch(sample_input("token-migration", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        let transcript = std::env::temp_dir().join(new_id("limitwise-transcript"));
        std::fs::write(
            &transcript,
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":40,\"output_tokens\":2}}\n",
        )
        .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO runs (id,task_id,started_at,finished_at,status,transcript_path)
             VALUES ('reported',?1,1,2,'completed',?2),('not-launched',?1,1,2,'missed',NULL)",
                params![task.id, transcript.to_string_lossy()],
            )
            .unwrap();

        backfill_run_token_usage(&store.connection).unwrap();
        let reported: (Option<i64>, String) = store
            .connection
            .query_row(
                "SELECT tokens_used,token_usage_state FROM runs WHERE id='reported'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let not_launched: (Option<i64>, String) = store
            .connection
            .query_row(
                "SELECT tokens_used,token_usage_state FROM runs WHERE id='not-launched'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(reported, (Some(42), "reported".into()));
        assert_eq!(not_launched, (Some(0), "not_launched".into()));
        let _ = std::fs::remove_file(transcript);
    }

    #[test]
    fn rejects_mixed_budget_fields() {
        let mut input = sample_input("mixed", 1.0);
        input.token_cap = Some(1000);
        assert!(validate_batch_input(&input).is_err());

        input.budget_mode = Some(BudgetMode::Tokens);
        input.weekly_cap_percent = None;
        assert!(validate_batch_input(&input).is_ok());
    }

    #[test]
    fn batch_budget_restarts_for_each_weekly_reset() {
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(sample_input("budget", 1.0)).unwrap();
        let first = store
            .ensure_budget_window(&created.batch.id, 40.0, 100)
            .unwrap();
        assert_eq!(first.allowance_points, 1.0);
        let consumed = store
            .reconcile_consumption(&created.batch.id, 40.5)
            .unwrap();
        assert_eq!(consumed.consumed_points, 0.5);
        let second = store
            .ensure_budget_window(&created.batch.id, 20.0, 200)
            .unwrap();
        assert_eq!(second.allowance_points, 1.0);
        assert_eq!(second.consumed_points, 0.0);
    }

    #[test]
    fn total_weekly_cap_is_clamped_to_remaining_quota() {
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(sample_input("clamped", 1.0)).unwrap();
        let budget = store
            .ensure_budget_window(&created.batch.id, 99.5, 100)
            .unwrap();
        assert_eq!(budget.allowance_points, 0.5);
    }

    #[test]
    fn legacy_remaining_percent_cap_keeps_its_old_meaning() {
        let mut input = sample_input("legacy", 1.0);
        input.budget_mode = None;
        input.weekly_cap_percent = None;
        input.cap_percent = Some(50.0);
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        let budget = store
            .ensure_budget_window(&created.batch.id, 40.0, 100)
            .unwrap();
        assert_eq!(budget.cap_basis, "remaining_percent");
        assert_eq!(budget.allowance_points, 30.0);
    }

    fn mark_quota_limited(
        store: &Store,
        task_id: &str,
        status: &str,
        reset_at: i64,
        session_id: Option<&str>,
        transcript_path: Option<&str>,
    ) {
        let snapshot = UsageSnapshot {
            adapter: "test".into(),
            captured_at: now_epoch(),
            five_hour: Some(RateWindow {
                used_percent: 90.0,
                remaining_percent: 10.0,
                duration_minutes: 300,
                resets_at: reset_at,
            }),
            weekly: RateWindow {
                used_percent: 20.0,
                remaining_percent: 80.0,
                duration_minutes: 10_080,
                resets_at: reset_at + 10_000,
            },
            warnings: Vec::new(),
        };
        let usage = serde_json::to_string(&snapshot).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO runs
                 (id,task_id,started_at,finished_at,status,usage_before_json,usage_after_json,session_id,transcript_path)
                 VALUES (?1,?2,?3,?3,?4,?5,?5,?6,?7)",
                params![
                    new_id("continuation-run"),
                    task_id,
                    now_epoch(),
                    status,
                    usage,
                    session_id,
                    transcript_path
                ],
            )
            .unwrap();
        store.set_status(task_id, status, Some("quota")).unwrap();
    }

    fn continuation_input(key: &str, predecessor_id: &str) -> ScheduleBatchInput {
        let mut input = sample_input(key, 1.0);
        input.tasks[0].run_at = None;
        input.tasks[0].continue_from_task_id = Some(predecessor_id.to_string());
        input
    }

    #[test]
    fn cross_batch_continuation_uses_reset_and_session_context() {
        let mut store = Store::in_memory().unwrap();
        let predecessor = store
            .schedule_batch(sample_input("continuation-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        let reset_at = now_epoch() + 600;
        mark_quota_limited(
            &store,
            &predecessor.id,
            "quota_interrupted",
            reset_at,
            Some("session-123"),
            Some("/tmp/transcript.jsonl"),
        );

        let successor = store
            .schedule_batch(continuation_input("continuation-target", &predecessor.id))
            .unwrap();
        let task = &successor.tasks[0];
        assert_ne!(successor.batch.id, predecessor.batch_id);
        assert_eq!(task.run_at, reset_at);
        assert_eq!(
            task.depends_on_task_id.as_deref(),
            Some(predecessor.id.as_str())
        );
        assert_eq!(task.dependency_type, "quota_reset");
        let context = store.continuation_context(task).unwrap().unwrap();
        assert_eq!(context.session_id.as_deref(), Some("session-123"));
        assert_eq!(
            context.transcript_path.as_deref(),
            Some("/tmp/transcript.jsonl")
        );
    }

    #[test]
    fn passed_continuation_reset_is_immediately_due() {
        let mut store = Store::in_memory().unwrap();
        let predecessor = store
            .schedule_batch(sample_input("passed-reset-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        mark_quota_limited(
            &store,
            &predecessor.id,
            "quota_skipped",
            now_epoch() - 60,
            None,
            None,
        );
        let successor = store
            .schedule_batch(continuation_input("passed-reset-target", &predecessor.id))
            .unwrap();
        assert!(successor.tasks[0].run_at <= now_epoch());
        assert!(store
            .due_tasks(now_epoch())
            .unwrap()
            .iter()
            .any(|task| task.id == successor.tasks[0].id));
    }

    #[test]
    fn continuation_can_be_deferred_to_next_reset() {
        let mut store = Store::in_memory().unwrap();
        let predecessor = store
            .schedule_batch(sample_input("defer-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        mark_quota_limited(
            &store,
            &predecessor.id,
            "quota_interrupted",
            now_epoch() - 60,
            None,
            None,
        );
        let successor = store
            .schedule_batch(continuation_input("defer-target", &predecessor.id))
            .unwrap();
        let task_id = &successor.tasks[0].id;
        assert!(store.claim_task(task_id).unwrap());
        let next_reset = now_epoch() + 600;
        store
            .defer_continuation(task_id, next_reset, "still reserved")
            .unwrap();
        let deferred = store.task(task_id).unwrap().unwrap();
        assert_eq!(deferred.status, "scheduled");
        assert_eq!(deferred.run_at, next_reset);
        assert_eq!(deferred.last_error.as_deref(), Some("still reserved"));
    }

    #[test]
    fn continuation_rejects_unknown_and_non_quota_predecessors() {
        let mut store = Store::in_memory().unwrap();
        let unknown = continuation_input("unknown-source", "missing");
        assert!(store.schedule_batch(unknown).is_err());

        let predecessor = store
            .schedule_batch(sample_input("failed-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        store
            .set_status(&predecessor.id, "failed", Some("test"))
            .unwrap();
        let error = store
            .schedule_batch(continuation_input("failed-target", &predecessor.id))
            .unwrap_err();
        assert!(error.contains("quota_interrupted or quota_skipped"));
    }

    #[test]
    fn continuation_cannot_move_to_another_worktree() {
        let mut store = Store::in_memory().unwrap();
        let predecessor = store
            .schedule_batch(sample_input("fixed-worktree-source", 1.0))
            .unwrap()
            .tasks
            .remove(0);
        mark_quota_limited(
            &store,
            &predecessor.id,
            "quota_interrupted",
            now_epoch() + 600,
            None,
            None,
        );
        let successor = store
            .schedule_batch(continuation_input("fixed-worktree-target", &predecessor.id))
            .unwrap();
        let update = TaskUpdate {
            cwd: Some("/".into()),
            ..TaskUpdate::default()
        };
        assert!(store.update_task(&successor.tasks[0].id, update).is_err());
    }

    #[test]
    fn chained_task_waits_for_previous_success() {
        let mut input = sample_input("chain", 1.0);
        input.tasks.push(TaskDraft {
            title: "second".into(),
            prompt: "modify the file".into(),
            success_criteria: "file is modified".into(),
            cwd: "/tmp".into(),
            run_at: None,
            after_previous: true,
            continue_from_task_id: None,
            timezone: Some("UTC".into()),
            difficulty: Difficulty::Simple,
            model: None,
            effort: None,
            permission_profile: PermissionProfile::Restricted,
        });
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        assert_eq!(
            created.tasks[1].depends_on_task_id,
            Some(created.tasks[0].id.clone())
        );
        store
            .connection
            .execute(
                "UPDATE tasks SET run_at=?1 WHERE batch_id=?2",
                params![now_epoch() - 1, created.batch.id],
            )
            .unwrap();
        let first_due = store.due_tasks(now_epoch()).unwrap();
        assert_eq!(first_due.len(), 1);
        assert_eq!(first_due[0].id, created.tasks[0].id);
        store
            .set_status(&created.tasks[0].id, "completed", None)
            .unwrap();
        let second_due = store.due_tasks(now_epoch()).unwrap();
        assert_eq!(second_due.len(), 1);
        assert_eq!(second_due[0].id, created.tasks[1].id);
    }

    #[test]
    fn chained_task_is_blocked_when_previous_task_fails() {
        let mut input = sample_input("failed-chain", 1.0);
        input.tasks.push(TaskDraft {
            title: "second".into(),
            prompt: "modify the file".into(),
            success_criteria: String::new(),
            cwd: "/tmp".into(),
            run_at: None,
            after_previous: true,
            continue_from_task_id: None,
            timezone: Some("UTC".into()),
            difficulty: Difficulty::Simple,
            model: None,
            effort: None,
            permission_profile: PermissionProfile::Restricted,
        });
        let mut store = Store::in_memory().unwrap();
        let created = store.schedule_batch(input).unwrap();
        store
            .set_status(&created.tasks[0].id, "failed", Some("test"))
            .unwrap();
        let blocked = store.failed_dependency_tasks().unwrap();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].id, created.tasks[1].id);
    }

    #[test]
    fn rejects_timestamp_on_chained_task() {
        let mut input = sample_input("invalid-chain", 1.0);
        let mut chained = input.tasks[0].clone();
        chained.title = "second".into();
        chained.after_previous = true;
        input.tasks.push(chained);
        assert!(validate_batch_input(&input).is_err());
    }

    #[test]
    fn upgrades_legacy_tables_without_losing_cap_semantics() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE batches (
                   id TEXT PRIMARY KEY,
                   idempotency_key TEXT NOT NULL UNIQUE,
                   cap_percent REAL NOT NULL,
                   created_at INTEGER NOT NULL,
                   window_reset_at INTEGER,
                   baseline_weekly_used_percent REAL,
                   allowance_points REAL NOT NULL DEFAULT 0,
                   consumed_points REAL NOT NULL DEFAULT 0
                 );
                 CREATE TABLE tasks (
                   id TEXT PRIMARY KEY,
                   batch_id TEXT NOT NULL REFERENCES batches(id),
                   title TEXT NOT NULL,
                   prompt TEXT NOT NULL,
                   success_criteria TEXT NOT NULL,
                   cwd TEXT NOT NULL,
                   run_at INTEGER NOT NULL,
                   timezone TEXT NOT NULL,
                   difficulty TEXT NOT NULL,
                   model TEXT NOT NULL,
                   effort TEXT NOT NULL,
                   status TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL,
                   last_error TEXT
                 );",
            )
            .unwrap();
        let store = Store::from_connection(connection).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO batches (id,idempotency_key,cap_percent,created_at) VALUES ('b','k',50,0)",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO tasks (id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at)
                 VALUES ('t','b','legacy','prompt','','/tmp',4102444800,'UTC','simple','gpt-5.6-luna','low','scheduled',0,0)",
                [],
            )
            .unwrap();
        let batch = store.batch("b").unwrap().unwrap();
        assert_eq!(batch.budget_mode, "percentage");
        assert_eq!(batch.cap_basis, "remaining_percent");
        assert_eq!(batch.five_hour_cap_percent, None);
        assert_eq!(batch.five_hour_consumed_points, 0.0);
        let task = store.task("t").unwrap().unwrap();
        assert_eq!(task.position, 0);
        assert_eq!(task.depends_on_task_id, None);
        assert_eq!(task.dependency_type, "success");
        assert_eq!(task.source_task_id, None);
        assert_eq!(task.attempt_kind, None);
        assert_eq!(task.attempt_number, 1);
        assert_eq!(task.permission_profile, PermissionProfile::Restricted);
        assert!(store
            .connection
            .execute(
                "UPDATE tasks SET permission_profile='danger-full-access' WHERE id='t'",
                [],
            )
            .is_err());
    }

    #[test]
    fn attempt_lineage_migration_is_additive_and_repeatable_for_legacy_database() {
        let path = std::env::temp_dir().join(new_id("limitwise-c04-legacy"));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE batches (
                   id TEXT PRIMARY KEY,idempotency_key TEXT NOT NULL UNIQUE,cap_percent REAL NOT NULL,
                   created_at INTEGER NOT NULL,window_reset_at INTEGER,baseline_weekly_used_percent REAL,
                   allowance_points REAL NOT NULL DEFAULT 0,consumed_points REAL NOT NULL DEFAULT 0
                 );
                 CREATE TABLE tasks (
                   id TEXT PRIMARY KEY,batch_id TEXT NOT NULL REFERENCES batches(id),title TEXT NOT NULL,
                   prompt TEXT NOT NULL,success_criteria TEXT NOT NULL,cwd TEXT NOT NULL,run_at INTEGER NOT NULL,
                   timezone TEXT NOT NULL,difficulty TEXT NOT NULL,model TEXT NOT NULL,effort TEXT NOT NULL,
                   status TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,last_error TEXT
                 );
                 CREATE TABLE runs (
                   id TEXT PRIMARY KEY,task_id TEXT NOT NULL REFERENCES tasks(id),started_at INTEGER NOT NULL,
                   finished_at INTEGER,status TEXT NOT NULL,usage_before_json TEXT,usage_after_json TEXT,
                   session_id TEXT,transcript_path TEXT,tokens_used INTEGER,
                   token_usage_state TEXT NOT NULL DEFAULT 'pending',error TEXT
                 );
                 INSERT INTO batches (id,idempotency_key,cap_percent,created_at) VALUES ('legacy-batch','legacy-key',25,7);
                 INSERT INTO tasks (id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error)
                 VALUES ('legacy-task','legacy-batch','Legacy title','Legacy prompt','Legacy done','/tmp',4102444800,'UTC','simple','gpt-5.6-luna','low','failed',8,9,'Legacy error');
                 INSERT INTO runs (id,task_id,started_at,finished_at,status,usage_before_json,usage_after_json,session_id,transcript_path,tokens_used,token_usage_state,error)
                 VALUES ('legacy-run','legacy-task',10,11,'failed','before','after','legacy-session','/tmp/legacy.jsonl',47,'reported','Legacy run error');",
            )
            .unwrap();
        drop(connection);

        for _ in 0..2 {
            let store = Store::from_connection(Connection::open(&path).unwrap()).unwrap();
            let task = store.task("legacy-task").unwrap().unwrap();
            assert_eq!(task.title, "Legacy title");
            assert_eq!(task.prompt, "Legacy prompt");
            assert_eq!(task.success_criteria, "Legacy done");
            assert_eq!(task.cwd, "/tmp");
            assert_eq!(task.run_at, 4_102_444_800);
            assert_eq!(task.timezone, "UTC");
            assert_eq!(task.difficulty, "simple");
            assert_eq!(task.model, "gpt-5.6-luna");
            assert_eq!(task.effort, "low");
            assert_eq!(task.status, "failed");
            assert_eq!(task.created_at, 8);
            assert_eq!(task.updated_at, 9);
            assert_eq!(task.last_error.as_deref(), Some("Legacy error"));
            assert_eq!(task.depends_on_task_id, None);
            assert_eq!(task.dependency_type, "success");
            assert_eq!(task.source_task_id, None);
            assert_eq!(task.attempt_kind, None);
            assert_eq!(task.attempt_number, 1);
            assert_eq!(task.permission_profile, PermissionProfile::Restricted);
            let runs = store.task_status("legacy-task").unwrap().unwrap().runs;
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].id, "legacy-run");
            assert_eq!(runs[0].task_id, "legacy-task");
            assert_eq!(runs[0].started_at, 10);
            assert_eq!(runs[0].finished_at, Some(11));
            assert_eq!(runs[0].status, "failed");
            assert_eq!(runs[0].usage_before_json.as_deref(), Some("before"));
            assert_eq!(runs[0].usage_after_json.as_deref(), Some("after"));
            assert_eq!(runs[0].session_id.as_deref(), Some("legacy-session"));
            assert_eq!(
                runs[0].transcript_path.as_deref(),
                Some("/tmp/legacy.jsonl")
            );
            assert_eq!(runs[0].tokens_used, Some(47));
            assert_eq!(runs[0].token_usage_state, "reported");
            assert_eq!(runs[0].error.as_deref(), Some("Legacy run error"));
        }
        let _ = std::fs::remove_file(path);
    }
}
