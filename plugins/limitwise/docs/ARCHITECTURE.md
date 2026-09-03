# Architecture

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Docs menu

[Home](index.md) | [Getting started](getting-started.md) | [Using LimitWise](using-limitwise.md) | [Troubleshooting](troubleshooting.md) | [Architecture](ARCHITECTURE.md)

## Components

`limitwise mcp` is a JSONL MCP server used by the planning skill. It exposes quota reads, explicit service setup, idempotent batch scheduling, and local task management. `limitwise daemon` is the long-running scheduler installed as a systemd user service or macOS LaunchAgent.

SQLite is the durable boundary. `batches` stores the selected budget mode, percentage or token cap, token consumption, compatibility basis, and active weekly-window accounting. `tasks` stores the confirmed prompt, success criteria, project, UTC instant, IANA timezone, optional prerequisite, batch position, difficulty, model, effort, and state. `runs` stores before/after quota snapshots, token usage, timing, Codex session id, transcript path, outcome, and failure reason.

Token recording is independent of budget enforcement. Percentage-mode and token-mode Codex runs both persist reported input-plus-output usage. Scheduler decisions made before launch persist zero; launched runs without a final usage event remain explicitly unavailable. An additive `token_usage_state` migration backfills retained transcripts idempotently while preserving unknown values. `task_usage_stats` derives rolling 365-day, 30-day, and seven-day summaries, local daily buckets, and recent per-run details from this single run history without duplicating aggregates.

## Prediction

`estimate_batch_usage` is a read-only planning path over completed runs from the previous 365 days. It computes p50 likely and p90 conservative values for both reported tokens and observed weekly-percentage changes. Cohort selection prefers at least three exact difficulty/model/effort matches, then broadens to model/effort, difficulty, and all completed history. Results include cohort, sample count, and confidence so cold-start uncertainty remains visible.

Percentage samples require valid before/after snapshots with the same weekly reset identity and a positive usage delta. The delta can contain concurrent interactive usage, intentionally making the estimate conservative but noisy. The tool compares the chosen cap with likely and conservative batch totals. It only returns an assessment: it does not persist predictions, modify caps, change model routing, or weaken runtime quota checks. No schema migration is required for version 0.5.

## Quota accounting

At the first task a batch touches in a weekly reset window:

```text
allowance_points = min(100 - weekly_used_at_first_task, weekly_cap_percent)
```

Thus `weekly_cap_percent = 1` allocates at most one percentage point of the full weekly limit. The daemon reconciles batch consumption to at least the increase from that baseline, so concurrent interactive use reduces what remains available to the batch. A new reset identity creates a new baseline and allowance at that window's first task. Rounded or delayed server values make this conservative enforcement best-effort rather than transactional.

Databases created before 0.2 retain `remaining_percent` as the basis for already-scheduled batches. New batches use `total_weekly_percent`; the additive migration does not reinterpret existing user budgets.

Token-mode batches use one non-resetting `token_cap` shared by every task. The daemon sums each Codex `turn.completed` event's input and output token counts; cached input is included and reasoning is already included in output. It records that value on the run and increments `consumed_tokens`. Missing usage exhausts the stored cap and fails closed. Current Codex CLI JSON exposes usage at turn completion, so LimitWise reliably blocks later launches but cannot guarantee a hard stop inside one active turn.

The adapter accepts only one 300-minute window and one unique longest window above it. Missing, malformed, or ambiguous data is an error. No Codex task starts at 90% or more five-hour usage, at exhausted weekly usage, when its selected batch budget is exhausted, or when telemetry is unavailable. Observable threshold crossings interrupt a running task.

## Task lifecycle

```text
scheduled -> running -> completed
                     -> failed
                     -> blocked
                     -> quota_interrupted
scheduled -> cancelled
scheduled -> missed
scheduled -> quota_skipped
```

Claiming is an atomic SQLite state transition, so duplicate daemon delivery cannot run a task twice. Idempotency keys similarly make repeated `schedule_batch` calls return the original batch.

A task with `after_previous` stores the preceding task id as its prerequisite and becomes eligible only after that task reaches `completed`. Its five-minute grace begins at prerequisite completion. Any other terminal prerequisite state records the dependent task as `blocked`; the rule propagates through a chain.

## Execution boundary

The daemon launches the confirmed Codex model and effort with `workspace-write`, `approval_policy="never"`, user configuration ignored, no web/network/apps, and no dangerous bypass. A process group receives `SIGINT` first on quota interruption, followed by `SIGTERM` after a grace period. JSONL output is kept as the transcript and scanned for the persistent session id.
