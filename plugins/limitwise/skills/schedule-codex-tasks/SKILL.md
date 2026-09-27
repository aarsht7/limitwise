---
name: schedule-codex-tasks
description: Plan, confirm, schedule, retry, resume, inspect, update, or cancel one-off coding tasks with LimitWise while respecting Codex rolling five-hour and weekly usage limits. Use whenever a user asks Codex to run coding work later or asks about LimitWise schedules, attempts, or quota.
---

# LimitWise Scheduler

Use the LimitWise MCP tools for quota-aware local coding tasks and sequential task chains on Linux and macOS.

Compatibility warning: LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested. When the user is on macOS, state this warning before planning, scheduling, setup, or task-management instructions.

## Terse output mode

Use terse mode by default whenever this skill is active. Reduce token usage in user-facing replies without reducing planning accuracy, safety, or scheduling detail.

- Remove filler, pleasantries, repeated summaries, and unnecessary explanation.
- Prefer compact fragments when meaning stays clear.
- Keep exact commands, file paths, code, JSON, timestamps, IDs, model names, effort values, and error text unchanged.
- Do not shorten compatibility warnings, destructive-action confirmations, quota-risk warnings, or any wording where compression could change meaning.
- Keep confirmation tables complete: task, plan, success criteria, difficulty, model, effort, exact timestamp/timezone or dependency trigger, project, permissions, shared budget mode/cap, estimate range, and confidence.
- If the user says `normal mode`, use fuller prose for the current conversation while keeping all LimitWise safety rules.
- If the user says `terse mode`, return to this default.
- If the user says `ultra terse`, answer even shorter, but preserve all exact technical values and safety warnings.

## Plan the batch

When the user is in Plan mode:

1. Inspect each selected project and turn the requested work into a concise executable plan with measurable success criteria.
2. Call `model_catalog` and `usage_snapshot`. Use only a model/effort pair reported for that model by the installed Codex app-server. If weekly telemetry is missing or any telemetry is ambiguous, say that execution will fail closed and do not invent quota values. If only five-hour telemetry is missing, warn that the global reserve and any five-hour batch cap cannot be enforced; continue with the weekly limit or token budget.
3. Classify from the inspected plan and propose exactly this route:
   - `simple`: localized work → `gpt-6-luna`, `low`
   - `standard`: normal multi-file implementation/testing → `gpt-6-sol`, `medium`
   - `complex`: architecture, migration, or difficult diagnosis → `gpt-6-astra`, `high`
   - `exceptional`: high-risk or verification-heavy work → `gpt-6-astra`, `xhigh`
4. Never propose `max`, `ultra`, fast, or ultrafast automatically. Let the user revise a supported model or effort.
5. Select one closed permission profile for each task:
   - `restricted` (default): `workspace-write`, network off, web search disabled, external apps disabled, and approval `never`.
   - `networked`: `workspace-write`, network on, web search enabled, external apps disabled, and approval `never`.
   Omission resolves to `restricted`. Never propose or accept arbitrary command fragments, `danger-full-access`, external apps, interactive approval, or another sandbox. If any task selects `networked`, show its plain-language risk and require a separate explicit acknowledgement that network and web search are enabled while the remaining prohibitions stay fixed.
6. Require the user to choose one whole-batch budget mode and its cap:
   - `percentage`: 1–100 percentage points of the total weekly limit per weekly reset window. `1` means exactly 1% of the full weekly limit, not 1% of the remaining allowance. Show current remaining percentage separately. If less quota remains, explain that effective allowance is limited to what remains.
   - `tokens`: a positive integer token cap for the entire batch, not per task and not reset weekly. Count input plus output tokens reported by Codex; cached input is included, while reasoning tokens are already part of output tokens. Explain that Codex CLI reports usage at turn boundaries, so one running task may overshoot before LimitWise can stop later work.
   - Optionally propose `five_hour_cap_percent` as a separate per-provider-window cap. It accepts more than 0 through 100 percentage points, resets with the provider five-hour window, and never overrides the global 10% reserve. Omission means no batch-specific five-hour cap.
7. Call `estimate_batch_usage` with the proposed task routes and chosen cap. Show its likely p50 and conservative p90 token estimates, weekly-percentage estimates, cohort size, and confidence. Estimates use local completed-run history and are never guarantees. For a percentage-mode batch spanning weekly reset windows, estimate and assess each window's task group separately because the cap renews per window; treat a chained task whose window cannot be known as uncertain.
8. If `cap_assessment.level` is `likely_insufficient`, clearly warn that the cap is below the likely estimate. If it is `tight`, warn that the cap is below the conservative estimate. Require explicit confirmation of that risk before scheduling; never raise the cap, downgrade the route, or change the task automatically. If assessment is unavailable or low-confidence, say so and recommend a safety margin without inventing a value.
9. Resolve every clock-based execution time to an exact local RFC3339 timestamp with an explicit offset and retain the IANA timezone. Use the system timezone by default. Never leave relative wording such as "in two minutes" in the proposal or payload. Ask about a local time if DST makes it nonexistent or ambiguous.
10. A task may instead run immediately after the preceding task completes successfully. Show its trigger as `after Task N succeeds`; pass `after_previous: true` and omit `run_at`. The first task cannot use this trigger. If the prerequisite does not complete successfully, the dependent task becomes `blocked`.
11. An existing terminal task may create one explicit new attempt. Verify it with `get_task_status`, require a fresh percentage or token budget, then call read-only `preview_retry_task`. The service infers the only eligible action: `quota_interrupted`/`quota_skipped` become `quota_resume` at the stored provider reset; `failed`/`blocked`/`missed`/`cancelled` become a fresh `retry` and require a selected future `run_at`; `completed`/`scheduled`/`running` reject. The retry inherits the source `permission_profile` unless the user selects a change. Show the inherited or changed profile with copied fields, next attempt number, exact run/reset time, resume/fallback method, fresh budget, and current estimate. A networked attempt requires its own separate acknowledgement. Never create an attempt automatically.
12. Show one confirmation table with task, plan, success criteria, difficulty, model, effort, exact timestamp/timezone or dependency trigger, project, permission profile and risk, shared budget mode/cap, optional five-hour cap, estimate range, and confidence.
13. Do not call `schedule_batch`, `retry_task`, `setup_service`, `update_task`, or `cancel_task` in Plan mode. Ask the user to confirm the table and switch to normal mode.

## Create only a confirmed schedule

Outside Plan mode, call `schedule_batch` only when the user has explicitly confirmed the complete proposal. Generate a stable idempotency key from the confirmed batch details and reuse it when retrying the same creation. Pass `budget_mode` plus exactly one matching cap: `weekly_cap_percent` for percentage mode or `token_cap` for token mode. Pass `five_hour_cap_percent` only when separately confirmed. Never convert these values.

Pass each task's confirmed `permission_profile`; omission is allowed only for `restricted` compatibility. If any task is `networked`, require a distinct explicit acknowledgement after showing the network risk, then pass `networked_confirmed: true`. General schedule confirmation is not the networked acknowledgement.

Call `setup_service` only with explicit approval. It installs a systemd user service on Linux or a LaunchAgent on macOS. Before macOS setup, warn that the macOS and Apple Silicon paths are untested.

Call `retry_task` only after the user explicitly confirms the complete `preview_retry_task` result. Reuse a stable idempotency key for duplicate delivery of that same confirmation. Never reuse a key to request a distinct attempt. A retry/resume creates a fresh batch/task and never updates the source task or its runs. Ordinary retry always starts a fresh session. Only quota resume may use the stored session or transcript/worktree fallback. If the preview profile is `networked`, require the distinct network acknowledgement and pass `networked_confirmed: true`.

Explain that:

- Work starts only if reliable weekly telemetry is available, weekly usage remains, and the selected shared budget remains. When five-hour telemetry is available, rolling usage must also be below 90%; when it is missing, LimitWise warns and continues without enforcing the global reserve or any five-hour batch cap.
- Token mode fails closed if a completed Codex run does not report token usage. Usage reporting occurs at turn boundaries, so its cap is best-effort for the currently running task; exhausted caps block later launches.
- An ordinary quota-short task is skipped, not deferred or silently downgraded. An explicitly confirmed continuation is deferred to the next provider reset when the global reserve still blocks it.
- Execution always uses `workspace-write` in the selected project, approval `never`, external apps disabled, and no dangerous bypass. `restricted` disables network and web search. `networked` enables only network and web search; it does not enable external apps, interactive approval, `danger-full-access`, or another sandbox.
- More than five minutes of lateness, including wake from sleep, marks the task `missed`.
- For a chained task, the five-minute grace starts when its prerequisite completes successfully.
- Every finished run records input-plus-output tokens when Codex reports them, regardless of budget mode. Runs skipped before Codex launches record zero tokens; launched runs without final token telemetry report usage as unavailable rather than inventing a value.

## Manage tasks

Use `model_catalog`, `usage_snapshot`, `estimate_batch_usage`, `preview_retry_task`, `list_tasks`, `get_task_status`, and `task_usage_stats` freely because they are read-only. `model_catalog` returns visible Codex models and the exact reasoning efforts supported by each model. `get_task_status` shows tokens for each run plus attempt lineage. `task_usage_stats` returns rolling one-year, 30-day, and seven-day totals, seven local calendar-day buckets, and individual run details for the last seven days. Update or cancel only tasks still in `scheduled` state and summarize the resulting task record.

When updating a scheduled task's `permission_profile` to `networked`, show the same risk and require a separate explicit acknowledgement, then pass `networked_confirmed: true`. Never attempt a profile update after a task starts.
