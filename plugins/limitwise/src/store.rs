use crate::config::{set_private_file, Paths};
use crate::model::{route, validate_route, Difficulty};
use crate::transcript::token_usage;
use chrono::{DateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    pub timezone: Option<String>,
    pub difficulty: Difficulty,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
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
    pub tasks: Vec<TaskDraft>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetMode {
    Percentage,
    Tokens,
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
    pub timezone: String,
    pub difficulty: String,
    pub model: String,
    pub effort: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_error: Option<String>,
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

#[derive(Clone, Debug, Serialize)]
pub struct TaskStatus {
    pub task: Task,
    pub batch: Batch,
    pub runs: Vec<RunRecord>,
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
                   consumed_points REAL NOT NULL DEFAULT 0
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
                   depends_on_task_id TEXT REFERENCES tasks(id)
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
        if let Some(batch) = self.batch_by_idempotency(&input.idempotency_key)? {
            let tasks = self.tasks_for_batch(&batch.id)?;
            return Ok(ScheduleBatchResult {
                batch,
                tasks,
                idempotent_replay: true,
            });
        }
        let budget = requested_budget(&input)?;
        let normalized = normalize_drafts(&input.tasks)?;
        let task_ids: Vec<String> = input.tasks.iter().map(|_| new_id("task")).collect();
        let now = now_epoch();
        let batch_id = new_id("batch");
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO batches
                 (id,idempotency_key,cap_percent,cap_basis,budget_mode,token_cap,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    batch_id,
                    input.idempotency_key,
                    budget.cap_percent,
                    budget.cap_basis,
                    budget.mode.to_string(),
                    budget.token_cap,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
        for (position, (draft, normalized)) in input
            .tasks
            .into_iter()
            .zip(normalized.into_iter())
            .enumerate()
        {
            let dependency = if normalized.depends_on_previous {
                Some(task_ids[position - 1].as_str())
            } else {
                None
            };
            transaction
                .execute(
                    "INSERT INTO tasks
                     (id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,position,depends_on_task_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'scheduled',?12,?12,?13,?14)",
                    params![
                        task_ids[position], batch_id, draft.title.trim(), draft.prompt.trim(),
                        draft.success_criteria.trim(), draft.cwd, normalized.run_at,
                        normalized.timezone, draft.difficulty.to_string(), normalized.model,
                        normalized.effort, now, position as i64, dependency
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

    pub fn list_tasks(&self, status: Option<&str>) -> Result<Vec<Task>, String> {
        let mut sql = "SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id FROM tasks".to_string();
        if status.is_some() {
            sql.push_str(" WHERE status=?1");
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

    pub fn task(&self, task_id: &str) -> Result<Option<Task>, String> {
        self.connection
            .query_row(
                "SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id FROM tasks WHERE id=?1",
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
        Ok(Some(TaskStatus { task, batch, runs }))
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
                    batch.budget_mode,task.model,task.effort,run.tokens_used,run.token_usage_state
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
                    task.created_at,task.updated_at,task.last_error,task.position,task.depends_on_task_id
             FROM tasks task
             LEFT JOIN tasks prerequisite ON prerequisite.id=task.depends_on_task_id
             WHERE task.status='scheduled' AND task.run_at<=?1
               AND (task.depends_on_task_id IS NULL OR prerequisite.status='completed')
             ORDER BY task.run_at ASC, task.position ASC",
        ).map_err(|e| e.to_string())?;
        let tasks = statement
            .query_map(params![now], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(tasks)
    }

    pub fn failed_dependency_tasks(&self) -> Result<Vec<Task>, String> {
        let mut statement = self.connection.prepare(
            "SELECT task.id,task.batch_id,task.title,task.prompt,task.success_criteria,task.cwd,
                    task.run_at,task.timezone,task.difficulty,task.model,task.effort,task.status,
                    task.created_at,task.updated_at,task.last_error,task.position,task.depends_on_task_id
             FROM tasks task
             JOIN tasks prerequisite ON prerequisite.id=task.depends_on_task_id
             WHERE task.status='scheduled'
               AND prerequisite.status NOT IN ('scheduled','running','completed')
             ORDER BY task.run_at ASC, task.position ASC",
        ).map_err(|e| e.to_string())?;
        let tasks = statement
            .query_map([], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(tasks)
    }

    pub fn dependency_completed_at(&self, task: &Task) -> Result<Option<i64>, String> {
        let Some(dependency_id) = task.depends_on_task_id.as_deref() else {
            return Ok(None);
        };
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
        let changed = self.connection.execute(
            "UPDATE tasks SET status='running',updated_at=?2 WHERE id=?1 AND status='scheduled'",
            params![task_id, now_epoch()],
        ).map_err(|e| e.to_string())?;
        Ok(changed == 1)
    }

    pub fn update_task(&mut self, task_id: &str, update: TaskUpdate) -> Result<Task, String> {
        let task = self
            .task(task_id)?
            .ok_or_else(|| "task not found".to_string())?;
        if task.status != "scheduled" {
            return Err("only scheduled tasks can be updated".to_string());
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
        validate_cwd(&cwd)?;
        self.connection.execute(
            "UPDATE tasks SET title=?2,prompt=?3,success_criteria=?4,cwd=?5,run_at=?6,timezone=?7,difficulty=?8,model=?9,effort=?10,updated_at=?11 WHERE id=?1",
            params![task_id, update.title.unwrap_or(task.title), update.prompt.unwrap_or(task.prompt),
                update.success_criteria.unwrap_or(task.success_criteria), cwd, run_at, timezone,
                difficulty.to_string(), model, effort, now_epoch()],
        ).map_err(|e| e.to_string())?;
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
        self.set_status(task_id, "cancelled", None)?;
        self.task(task_id)?
            .ok_or_else(|| "cancelled task disappeared".to_string())
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

    pub fn batch(&self, batch_id: &str) -> Result<Option<Batch>, String> {
        self.connection.query_row(
            "SELECT id,idempotency_key,budget_mode,token_cap,consumed_tokens,cap_percent,cap_basis,created_at,window_reset_at,baseline_weekly_used_percent,allowance_points,consumed_points FROM batches WHERE id=?1",
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
            "SELECT id,idempotency_key,budget_mode,token_cap,consumed_tokens,cap_percent,cap_basis,created_at,window_reset_at,baseline_weekly_used_percent,allowance_points,consumed_points FROM batches WHERE idempotency_key=?1",
            params![key], row_to_batch,
        ).optional().map_err(|e| e.to_string())
    }

    fn tasks_for_batch(&self, batch_id: &str) -> Result<Vec<Task>, String> {
        let mut statement = self.connection.prepare(
            "SELECT id,batch_id,title,prompt,success_criteria,cwd,run_at,timezone,difficulty,model,effort,status,created_at,updated_at,last_error,position,depends_on_task_id FROM tasks WHERE batch_id=?1 ORDER BY run_at ASC, position ASC",
        ).map_err(|e| e.to_string())?;
        let tasks = statement
            .query_map(params![batch_id], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(tasks)
    }
}

fn validate_batch_input(input: &ScheduleBatchInput) -> Result<(), String> {
    if input.idempotency_key.trim().is_empty() {
        return Err("idempotency_key is required".to_string());
    }
    requested_budget(input)?;
    if input.tasks.is_empty() {
        return Err("at least one task is required".to_string());
    }
    for task in &input.tasks {
        if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
            return Err("every task needs a title and prompt".to_string());
        }
    }
    normalize_drafts(&input.tasks)?;
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
    depends_on_previous: bool,
}

fn normalize_drafts(drafts: &[TaskDraft]) -> Result<Vec<NormalizedDraft>, String> {
    let mut normalized: Vec<NormalizedDraft> = Vec::with_capacity(drafts.len());
    for (index, draft) in drafts.iter().enumerate() {
        validate_cwd(&draft.cwd)?;
        let timezone = draft
            .timezone
            .clone()
            .unwrap_or_else(crate::config::system_timezone);
        validate_timezone(&timezone)?;
        let run_at = if draft.after_previous {
            if index == 0 {
                return Err("the first task cannot use after_previous".to_string());
            }
            if draft.run_at.is_some() {
                return Err("a task with after_previous=true must omit run_at".to_string());
            }
            normalized[index - 1].run_at
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
            depends_on_previous: draft.after_previous,
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
    })
}

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let timestamp: i64 = row.get(6)?;
    let timezone: String = row.get(7)?;
    let run_at_iso = DateTime::<Utc>::from_timestamp(timestamp, 0)
        .and_then(|utc| {
            timezone
                .parse::<Tz>()
                .ok()
                .map(|tz| utc.with_timezone(&tz).to_rfc3339())
        })
        .unwrap_or_else(|| timestamp.to_string());
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
        timezone,
        difficulty: row.get(8)?,
        model: row.get(9)?,
        effort: row.get(10)?,
        status: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        last_error: row.get(14)?,
    })
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
    use chrono::Duration;
    #[test]
    fn rejects_bad_caps() {
        let input = ScheduleBatchInput {
            idempotency_key: "key".into(),
            budget_mode: Some(BudgetMode::Percentage),
            weekly_cap_percent: Some(0.0),
            token_cap: None,
            cap_percent: None,
            tasks: vec![],
        };
        assert!(validate_batch_input(&input).is_err());
    }
    #[test]
    fn terminal_statuses_are_stable() {
        assert!(TERMINAL_STATUSES.contains(&"quota_interrupted"));
        assert!(!TERMINAL_STATUSES.contains(&"running"));
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

    fn sample_input(key: &str, cap_percent: f64) -> ScheduleBatchInput {
        let run_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
        ScheduleBatchInput {
            idempotency_key: key.into(),
            budget_mode: Some(BudgetMode::Percentage),
            weekly_cap_percent: Some(cap_percent),
            token_cap: None,
            cap_percent: None,
            tasks: vec![TaskDraft {
                title: "test".into(),
                prompt: "make a harmless change".into(),
                success_criteria: "tests pass".into(),
                cwd: "/tmp".into(),
                run_at: Some(run_at),
                after_previous: false,
                timezone: Some("UTC".into()),
                difficulty: Difficulty::Standard,
                model: None,
                effort: None,
            }],
        }
    }

    #[test]
    fn schedule_is_idempotent_and_routes_defaults() {
        let mut store = Store::in_memory().unwrap();
        let first = store.schedule_batch(sample_input("same", 50.0)).unwrap();
        let second = store.schedule_batch(sample_input("same", 50.0)).unwrap();
        assert!(!first.idempotent_replay);
        assert!(second.idempotent_replay);
        assert_eq!(first.batch.id, second.batch.id);
        assert_eq!(first.tasks[0].model, "gpt-5.6-terra");
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
            timezone: Some("UTC".into()),
            difficulty: Difficulty::Simple,
            model: None,
            effort: None,
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
            timezone: Some("UTC".into()),
            difficulty: Difficulty::Simple,
            model: None,
            effort: None,
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
        let task = store.task("t").unwrap().unwrap();
        assert_eq!(task.position, 0);
        assert_eq!(task.depends_on_task_id, None);
    }
}
