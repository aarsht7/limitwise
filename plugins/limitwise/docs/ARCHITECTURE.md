---
layout: default
title: Architecture
---

# Architecture

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Docs menu

[Home](index.md) | [Demos](demos.md) | [Getting started](getting-started.md) | [Using LimitWise](using-limitwise.md) | [Local browser UI](local-browser-ui.md) | [Troubleshooting](troubleshooting.md) | [Architecture](ARCHITECTURE.md)

## Components

`limitwise mcp` is a JSONL MCP server used by the planning skill. It exposes quota reads, explicit service setup, idempotent batch scheduling, read-only attempt preview, idempotent retry/resume confirmation, and local task management. `limitwise daemon` is the long-running scheduler installed as a systemd user service or macOS LaunchAgent.

`limitwise doctor`, `limitwise doctor --json`, and the read-only `diagnostics_snapshot` MCP tool share one versioned diagnostic service and serialized report type. The service discovers paths but never calls `Paths::ensure`, `Store::open`, schema migration, service setup, or plugin installation/refresh. SQLite opens with read-only flags; missing state is inspected as missing rather than created. Checks have stable IDs and deterministic order. Any failed check makes the report fail, otherwise any warning makes it warn, otherwise it passes.

`ApplicationService` is the shared orchestration boundary for MCP and the local HTTP transport. It owns calls for usage, diagnostics, Codex-backed UI planning, list/detail, paginated/sorted browser task and batch lists, pending-batch edit leases and atomic replacement, statistics, estimate, schedule, retry/resume preview and confirmation, update, cancellation, archive, running-task stop, and service setup; transports retain their existing envelopes while store/domain code remains authoritative for validation, idempotency, quota rules, and state transitions.

LimitWise reads visible models and their per-model `supportedReasoningEfforts` through the installed Codex app-server's paginated `model/list` method. The catalog is cached for five minutes and exposed through the read-only `model_catalog` MCP tool and `GET /api/v1/models`. If discovery fails, a bundled catalog keeps the scheduler usable and the HTTP response includes the exact warning. Route validation always checks the selected effort against the selected model.

Simple browser planning starts an ephemeral Codex app-server thread under the real Plan collaboration mode. The planning turn uses the user-selected catalog model (`gpt-6-sol` by default) with fixed `high` reasoning, a read-only sandbox, no approval prompts, no network, no apps, and a strict structured-output schema. LimitWise checks quota first and accepts only the bounded difficulty/model/effort routes in its contract: `gpt-6-luna/low`, `gpt-6-sol/medium`, `gpt-6-astra/high`, and `gpt-6-astra/xhigh`. Automatic planning never selects `max` or `ultra`; users may select them manually only for models that advertise them. The returned execution prompts and success criteria remain a review artifact until a separate confirmed scheduling request. Simple and Advanced modes share the same project-level directory selector, required weekly-percentage limit, per-task permission selector, and linear-chain editor. The directory defaults to the canonical UI launch directory and may be replaced through an explicitly confirmed native folder picker; planning validates that it is an existing absolute directory. Simple preserves each selected execution permission profile across planning, while Advanced also sends its explicit route fields through the existing schedule contract.

Task removal is an additive soft-delete migration: `tasks.archived_at` hides rows from normal lists and scheduler claims. Pending tasks become `cancelled`; running tasks cannot be archived. Foreign-key-linked `runs`, lineage, route metadata, and usage/prediction queries remain unchanged and retained.

`limitwise ui` embeds the Vite production assets into the Rust binary and serves only fixed asset routes from a `127.0.0.1` listener. A one-time URL-fragment bootstrap returns a separate per-launch bearer after same-origin validation. Every later `/api/v1` request requires the bearer, the loopback Host/same-origin browser context, and JSON content type. The server has bounded headers/bodies, fixed JSON errors, restrictive browser headers, no permissive CORS, no arbitrary path-enumeration route, and no transcript-content response. The project picker API can return the canonical launch path and invoke a native folder chooser after explicit confirmation, but does not return file contents or accept a directory to enumerate. The foreground listener stops on `SIGINT`/`SIGTERM` and is never installed as a service.

Diagnostic process output is reduced to status, validated version text, or fixed categories. Storage paths use placeholders, the daemon stderr read is bounded, and raw log lines, prompts, transcripts, bearer tokens, credentials, and environment values are excluded from the public contract.

SQLite is the durable boundary. `batches` stores the selected budget mode, percentage or token cap, token consumption, compatibility basis, active weekly-window accounting, optional five-hour-window accounting, and nullable batch-editor session/heartbeat fields. `tasks` stores the confirmed prompt, success criteria, project, UTC instant, IANA timezone, optional prerequisite and its `success` or `quota_reset` type, batch position, difficulty, model, effort, state, nullable `source_task_id`, nullable `attempt_kind`, and non-null `attempt_number`. `runs` stores before/after quota snapshots, token usage, timing, Codex session id, transcript path, outcome, and failure reason.

`task_stop_requests` is an additive control table keyed by running task. The HTTP process records an idempotent request; the owning daemon polls it every 500 ms, interrupts its own child process group, records the run/task as `cancelled`, then clears the request. No process ID crosses the database boundary.

Migration only adds nullable or defaulted columns, so existing rows and schedules retain old semantics. Existing tasks are idempotently backfilled as attempt `1` with no source or attempt kind. Rolling back the binary leaves those columns intact; older binaries ignore lineage while existing dependency and quota-reset columns keep their prior meaning. Before rollback, cancel any scheduled task using a five-hour cap or `quota_reset`; older binaries do not understand those enforcement rules.

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

The adapter accepts at most one 300-minute window and one unique longest window above it. Missing weekly telemetry and malformed or ambiguous telemetry are errors. A missing 300-minute window produces a snapshot with unavailable five-hour data: LimitWise warns, skips the global reserve and any five-hour batch-cap enforcement, and continues with the weekly limit or token budget. No Codex task starts at 90% or more five-hour usage when that window is available, at exhausted weekly usage, or when its selected primary batch budget is exhausted. Observable threshold crossings interrupt a running task.

When `five_hour_cap_percent` is present, each batch separately records the provider five-hour reset identity, baseline, allowance, and consumption. The allowance is clamped to capacity below the global 90% threshold. A new provider reset creates a fresh allowance. Omission leaves only the global reserve. Provider deltas can include concurrent interactive use, so enforcement remains conservative.

## Task lifecycle

```text
scheduled -> running -> completed
                     -> failed
                     -> blocked
                     -> quota_interrupted
                     -> cancelled (confirmed stop)
scheduled -> cancelled
scheduled -> missed
scheduled -> quota_skipped
```

Claiming is an atomic SQLite state transition, so duplicate daemon delivery cannot run a task twice. Idempotency keys similarly make repeated `schedule_batch` calls return the original batch.

Opening a pending batch editor atomically records a random edit session and heartbeat on the batch only if the first task is still `scheduled` and later tasks have not run. The browser renews the 60-second lease every 15 seconds. Due-task discovery, failed-dependency handling, and the final atomic claim all exclude an active lease, closing the discovery-to-claim race. Saving validates a future first-task time, rewrites retained/new task rows and chain dependencies in one immediate transaction, soft-archives removed rows, and clears the lease. An abandoned editor becomes runnable again after lease expiry.

Retry/resume confirmation starts an immediate SQLite transaction and creates exactly one fresh batch plus one fresh task. The batch idempotency key makes duplicate delivery return the same attempt. The source task and all of its runs remain unchanged. `source_task_id`, `attempt_kind` (`retry` or `quota_resume`), and `attempt_number` make the full lineage queryable after restart. Fresh retries have no continuation dependency and always start a fresh Codex session. Quota resumes also store the source as the existing `quota_reset` dependency, so only they can use session resume or transcript/worktree fallback.

A task with `after_previous` stores the preceding task id as its prerequisite and becomes eligible only after that task reaches `completed`. Its five-minute grace begins at prerequisite completion. Any other terminal prerequisite state records the dependent task as `blocked`; the rule propagates through a chain.

A task with `continue_from_task_id` stores a `quota_reset` prerequisite. Creation requires an existing `quota_interrupted` or `quota_skipped` predecessor in the same worktree with a valid quota snapshot. Its due time is the predecessor's recorded five-hour reset, or immediately when that reset has passed. The new batch owns fresh weekly/token and optional five-hour budgets. No task is created automatically. If the global reserve still blocks launch, the claimed continuation returns to `scheduled` at the next reset instead of becoming terminal.

The C04 interfaces expose that behavior as read-only `preview_retry_task` plus idempotent `retry_task`, and as `POST /api/v1/tasks/{id}/retry-preview` plus `POST /api/v1/tasks/{id}/retry`. Eligibility is derived from source state and retained run metadata: `quota_interrupted`/`quota_skipped` with a valid reset get quota resume; quota failures without a usable snapshot and `failed`/`blocked`/`missed`/`cancelled` get a fresh retry; `completed`/`scheduled`/`running` are rejected. Every preview requires a fresh percentage or token budget, includes the current local estimate, and performs no write. Confirmation additionally requires `confirmed: true` and a non-empty idempotency key.

## Execution boundary

The daemon launches the confirmed Codex model, effort, and closed permission profile through one argument mapper. `restricted` uses `workspace-write`, network off, and web search disabled. `networked` keeps `workspace-write` but enables network and live web search after a separate acknowledgement. Both profiles keep interactive approval at `never`, external apps disabled, user configuration ignored, and dangerous bypasses unavailable. A process group receives `SIGINT` first on quota interruption, followed by `SIGTERM` after a grace period. JSONL output is kept as the transcript and scanned for the persistent session id. A continuation resumes that session when supported and present; otherwise, its fresh prompt includes predecessor ID, prompt, success criteria, transcript path, a bounded transcript excerpt, and worktree context.
