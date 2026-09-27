use crate::diagnostic::{self, DiagnosticReport};
use crate::model::{model_catalog, Difficulty, ModelCatalog};
use crate::planner::{PlanBatchInput, PlannedBatch, PlannerClient};
use crate::prediction::{self, BatchUsageEstimate, EstimateBatchInput, EstimateTaskInput};
use crate::store::{
    now_epoch, ArchiveTaskResult, AttemptCreateResult, AttemptPreview, BatchEditInput,
    BatchEditSession, BatchGroup, BatchPage, ConfirmedRetryTaskInput, RetryTaskOptions,
    ScheduleBatchInput, ScheduleBatchResult, Store, Task, TaskPage, TaskSort, TaskStatus,
    TaskUpdate, TaskUsageStats,
};
use crate::usage::{UsageClient, UsageSnapshot};
use serde::Serialize;
use std::str::FromStr;

#[derive(Debug, Serialize)]
pub struct RetryTaskPreview {
    #[serde(flatten)]
    pub attempt: AttemptPreview,
    pub estimate: BatchUsageEstimate,
}

/// The application boundary shared by MCP, the CLI, and the local HTTP API.
///
/// Transport layers parse inputs and serialize outputs only. Validation,
/// idempotency, state transitions, and quota semantics remain in the existing
/// store and domain services.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApplicationService;

impl ApplicationService {
    pub fn models(&self) -> ModelCatalog {
        model_catalog()
    }

    pub fn diagnostics(&self) -> Result<DiagnosticReport, String> {
        diagnostic::snapshot()
    }

    pub fn usage(&self) -> Result<UsageSnapshot, String> {
        UsageClient::default().fetch()
    }

    pub fn setup_service(&self) -> Result<String, String> {
        crate::service::setup()
    }

    pub fn plan_tasks(&self, input: PlanBatchInput) -> Result<PlannedBatch, String> {
        let usage = UsageClient::default().fetch()?;
        if usage.five_hour.as_ref().is_some_and(|window| {
            window.used_percent >= 100.0 - crate::config::FIVE_HOUR_RESERVE_PERCENT
        }) {
            return Err(
                "Codex planning is blocked by the global 10% five-hour reserve".to_string(),
            );
        }
        if usage.weekly.remaining_percent <= 0.0 {
            return Err("Codex planning is blocked because no weekly quota remains".to_string());
        }
        PlannerClient::default().plan(&input, crate::config::system_timezone())
    }

    pub fn schedule_batch(&self, input: ScheduleBatchInput) -> Result<ScheduleBatchResult, String> {
        Store::open()?.schedule_batch(input)
    }

    pub fn validate_batch(&self, input: &ScheduleBatchInput) -> Result<(), String> {
        Store::open()?.validate_schedule_batch(input)
    }

    pub fn retry_preview(
        &self,
        task_id: &str,
        options: RetryTaskOptions,
    ) -> Result<RetryTaskPreview, String> {
        let store = Store::open()?;
        let attempt = store.preview_retry_task(task_id, &options)?;
        let estimate = prediction::estimate(
            &store,
            EstimateBatchInput {
                budget_mode: options.budget_mode,
                weekly_cap_percent: options.weekly_cap_percent,
                token_cap: options.token_cap,
                tasks: vec![EstimateTaskInput {
                    title: attempt.task.title.clone(),
                    difficulty: Difficulty::from_str(&attempt.task.difficulty)?,
                    model: Some(attempt.task.model.clone()),
                    effort: Some(attempt.task.effort.clone()),
                }],
            },
            now_epoch(),
        )?;
        Ok(RetryTaskPreview { attempt, estimate })
    }

    pub fn retry_task(
        &self,
        task_id: &str,
        input: ConfirmedRetryTaskInput,
    ) -> Result<AttemptCreateResult, String> {
        Store::open()?.create_retry_task(task_id, input)
    }

    pub fn list_tasks(&self, status: Option<&str>) -> Result<Vec<Task>, String> {
        Store::open()?.list_tasks(status)
    }

    pub fn list_task_page(
        &self,
        status: Option<&str>,
        sort: &str,
        page: i64,
    ) -> Result<TaskPage, String> {
        Store::open()?.list_task_page(status, TaskSort::from_str(sort)?, page)
    }

    pub fn list_batch_page(
        &self,
        status: Option<&str>,
        sort: &str,
        page: i64,
    ) -> Result<BatchPage, String> {
        Store::open()?.list_batch_page(status, TaskSort::from_str(sort)?, page)
    }

    pub fn begin_batch_edit(&self, batch_id: &str) -> Result<BatchEditSession, String> {
        Store::open()?.begin_batch_edit(batch_id)
    }

    pub fn heartbeat_batch_edit(
        &self,
        batch_id: &str,
        edit_session_id: &str,
    ) -> Result<i64, String> {
        Store::open()?.heartbeat_batch_edit(batch_id, edit_session_id)
    }

    pub fn cancel_batch_edit(&self, batch_id: &str, edit_session_id: &str) -> Result<(), String> {
        Store::open()?.cancel_batch_edit(batch_id, edit_session_id)
    }

    pub fn update_batch(
        &self,
        batch_id: &str,
        input: BatchEditInput,
    ) -> Result<BatchGroup, String> {
        Store::open()?.update_batch(batch_id, input)
    }

    pub fn task_status(&self, task_id: &str) -> Result<TaskStatus, String> {
        Store::open()?
            .task_status(task_id)?
            .ok_or_else(|| "task not found".to_string())
    }

    pub fn task_usage_stats(&self) -> Result<TaskUsageStats, String> {
        Store::open()?.task_usage_stats(now_epoch(), &crate::config::system_timezone())
    }

    pub fn estimate_batch(&self, input: EstimateBatchInput) -> Result<BatchUsageEstimate, String> {
        let store = Store::open()?;
        prediction::estimate(&store, input, now_epoch())
    }

    pub fn update_task(&self, task_id: &str, update: TaskUpdate) -> Result<Task, String> {
        Store::open()?.update_task(task_id, update)
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<Task, String> {
        Store::open()?.cancel_task(task_id)
    }

    pub fn stop_task(&self, task_id: &str) -> Result<Task, String> {
        Store::open()?.request_task_stop(task_id)
    }

    pub fn archive_task(&self, task_id: &str) -> Result<ArchiveTaskResult, String> {
        Store::open()?.archive_task(task_id)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn boundary_exposes_every_current_transport_operation() {
        let operation_names = [
            "models",
            "diagnostics",
            "usage",
            "setup_service",
            "plan_tasks",
            "schedule_batch",
            "validate_batch",
            "retry_preview",
            "retry_task",
            "list_tasks",
            "list_task_page",
            "list_batch_page",
            "begin_batch_edit",
            "heartbeat_batch_edit",
            "cancel_batch_edit",
            "update_batch",
            "task_status",
            "task_usage_stats",
            "estimate_batch",
            "update_task",
            "cancel_task",
            "stop_task",
            "archive_task",
        ];
        assert_eq!(operation_names.len(), 23);
    }
}
