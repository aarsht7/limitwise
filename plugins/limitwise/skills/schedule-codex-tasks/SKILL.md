---
name: schedule-codex-tasks
description: Plan, confirm, schedule, inspect, update, or cancel one-off coding tasks with LimitWise while respecting Codex rolling five-hour and weekly usage limits. Use whenever a user asks Codex to run coding work later or asks about LimitWise schedules or quota.
---

# LimitWise Scheduler

Use the LimitWise MCP tools for quota-aware local coding tasks and sequential task chains on Linux and macOS.

Compatibility warning: LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested. When the user is on macOS, state this warning before planning, scheduling, setup, or task-management instructions.

## Plan the batch

When the user is in Plan mode:

1. Inspect each selected project and turn the requested work into a concise executable plan with measurable success criteria.
2. Call `usage_snapshot`. If telemetry is missing or ambiguous, say that execution will fail closed and do not invent quota values.
3. Classify from the inspected plan and propose exactly this route:
   - `simple`: localized work → `gpt-5.6-luna`, `low`
   - `standard`: normal multi-file implementation/testing → `gpt-5.6-terra`, `medium`
   - `complex`: architecture, migration, or difficult diagnosis → `gpt-5.6-sol`, `high`
   - `exceptional`: high-risk or verification-heavy work → `gpt-5.6-sol`, `xhigh`
4. Never propose `max`, `ultra`, fast, or ultrafast automatically. Let the user revise a supported model or effort.
5. Require the user to choose one whole-batch budget mode and its cap:
   - `percentage`: 1–100 percentage points of the total weekly limit per weekly reset window. `1` means exactly 1% of the full weekly limit, not 1% of the remaining allowance. Show current remaining percentage separately. If less quota remains, explain that effective allowance is limited to what remains.
   - `tokens`: a positive integer token cap for the entire batch, not per task and not reset weekly. Count input plus output tokens reported by Codex; cached input is included, while reasoning tokens are already part of output tokens. Explain that Codex CLI reports usage at turn boundaries, so one running task may overshoot before LimitWise can stop later work.
6. Call `estimate_batch_usage` with the proposed task routes and chosen cap. Show its likely p50 and conservative p90 token estimates, weekly-percentage estimates, cohort size, and confidence. Estimates use local completed-run history and are never guarantees. For a percentage-mode batch spanning weekly reset windows, estimate and assess each window's task group separately because the cap renews per window; treat a chained task whose window cannot be known as uncertain.
7. If `cap_assessment.level` is `likely_insufficient`, clearly warn that the cap is below the likely estimate. If it is `tight`, warn that the cap is below the conservative estimate. Require explicit confirmation of that risk before scheduling; never raise the cap, downgrade the route, or change the task automatically. If assessment is unavailable or low-confidence, say so and recommend a safety margin without inventing a value.
8. Resolve every clock-based execution time to an exact local RFC3339 timestamp with an explicit offset and retain the IANA timezone. Use the system timezone by default. Never leave relative wording such as "in two minutes" in the proposal or payload. Ask about a local time if DST makes it nonexistent or ambiguous.
9. A task may instead run immediately after the preceding task completes successfully. Show its trigger as `after Task N succeeds`; pass `after_previous: true` and omit `run_at`. The first task cannot use this trigger. If the prerequisite does not complete successfully, the dependent task becomes `blocked`.
10. Show one confirmation table with task, plan, success criteria, difficulty, model, effort, exact timestamp/timezone or dependency trigger, project, permissions, shared budget mode/cap, estimate range, and confidence.
11. Do not call `schedule_batch`, `setup_service`, `update_task`, or `cancel_task` in Plan mode. Ask the user to confirm the table and switch to normal mode.

## Create only a confirmed schedule

Outside Plan mode, call `schedule_batch` only when the user has explicitly confirmed the complete proposal. Generate a stable idempotency key from the confirmed batch details and reuse it when retrying the same creation. Pass `budget_mode` plus exactly one matching cap: `weekly_cap_percent` for percentage mode or `token_cap` for token mode. Never convert either value.

Call `setup_service` only with explicit approval. It installs a systemd user service on Linux or a LaunchAgent on macOS. Before macOS setup, warn that the macOS and Apple Silicon paths are untested.

Explain that:

- Work starts only if reliable quota telemetry is available, rolling five-hour usage is below 90%, weekly usage remains, and the selected shared budget remains.
- Token mode fails closed if a completed Codex run does not report token usage. Usage reporting occurs at turn boundaries, so its cap is best-effort for the currently running task; exhausted caps block later launches.
- A quota-short task is skipped, not deferred or silently downgraded.
- Execution uses `workspace-write` in the selected project, no interactive approvals, no external apps/network, and no dangerous bypass.
- More than five minutes of lateness, including wake from sleep, marks the task `missed`.
- For a chained task, the five-minute grace starts when its prerequisite completes successfully.
- Every finished run records input-plus-output tokens when Codex reports them, regardless of budget mode. Runs skipped before Codex launches record zero tokens; launched runs without final token telemetry report usage as unavailable rather than inventing a value.

## Manage tasks

Use `usage_snapshot`, `estimate_batch_usage`, `list_tasks`, `get_task_status`, and `task_usage_stats` freely because they are read-only. `get_task_status` shows tokens for each run. `task_usage_stats` returns rolling one-year, 30-day, and seven-day totals, seven local calendar-day buckets, and individual run details for the last seven days. Update or cancel only tasks still in `scheduled` state and summarize the resulting task record.
