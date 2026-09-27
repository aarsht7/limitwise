# Changelog

> Compatibility: LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested.

## Unreleased

## 1.0.0 - 2026-09-27

- Group composer-created chains as batches on the dashboard and add atomic pending-batch editing with task reorder/add/remove. A renewable edit lease prevents the daemon from starting the first task while the editor is active, and a passed start time must be moved into the future before save.
- Discover visible models and per-model reasoning efforts from Codex `model/list`, expose them through MCP and the browser API, add catalog-driven UI selectors, accept supported `max`/`ultra` choices, and move automatic routing to `gpt-6-luna`, `gpt-6-sol`, and `gpt-6-astra` without automatically selecting `max` or `ultra`.
- Warn and continue with the weekly limit or token budget when five-hour telemetry is missing; keep missing weekly or ambiguous telemetry fail-closed.
- Add closed `restricted` and separately confirmed `networked` task permission profiles while keeping `workspace-write`, external apps disabled, and approval `never` fixed across scheduling, updates, retries, MCP, HTTP, and the browser UI.
- Add read-only `limitwise doctor` human/JSON diagnostics and the equivalent `diagnostics_snapshot` MCP tool with stable checks, redacted evidence, non-mutating SQLite inspection, and explicit exit semantics.
- Add the foreground `limitwise ui` local-browser dashboard, linear visual composer, and operations surface through an authenticated loopback API and a shared Rust application-service boundary. Production frontend, API, browser, packaged-binary, Linux, and ARM macOS proof remains required before release.
- Add immutable retry/quota-resume attempts with additive lineage migration, read-only preview, atomic idempotent confirmation, matching MCP/HTTP contracts, and one-action task-detail UX. Focused, migration, API/security, browser, end-to-end, global, and packaged Linux x86-64 proof passes; native ARM macOS release proof remains required.
- Resolve Codex from service-safe absolute locations, persist its path in generated service definitions, allow fresh retry when a quota-skipped run captured no usable snapshot, paginate browser task lists at 20 rows with newest-first sorting, and add running-task stop plus terminal retry controls. Pause remains intentionally unsupported because Codex has no safe persistent pause contract.
- Redesign the browser UI with flat, minimalist dark and light themes, high-contrast electric-blue and violet accents, a dark default, and a persisted accessible theme control.
- Prefill editable composer and retry `run_at` fields with the current local time plus a one-minute scheduling margin while keeping them fully editable.
- Keep the linear chained-task composer in Simple mode and add a distinct Codex Plan-mode review step before scheduling. The user can choose the planning model (`gpt-6-sol` by default) and each task's permission profile while planning stays read-only with `high` reasoning and returns bounded per-task difficulty/model/effort routes; Advanced mode retains full manual routing control.
- Prevent long plan, estimate, and exact-payload text from widening the composer after review.
- Add one project selector to both composer modes, default it to the UI launch directory, and provide a native folder picker with an editable absolute-path fallback. Require a weekly-percentage limit for every UI-planned batch while retaining the global 10% five-hour reserve when that quota window is available.
- Add task-list removal backed by an additive `archived_at` soft delete. Scheduled tasks are cancelled, running tasks must be stopped first, and run/token/model metadata remains available to local usage and prediction history.

## 0.6.2 - 2026-09-23

- Accept checksum filenames with or without a leading `./`, and emit bare filenames in future release checksum manifests.
- Build Linux releases with current stable Rust inside the fixed GLIBC 2.31 environment.
- Raise the source-build minimum to Rust 1.85 for Rust 2024 dependency manifests.
- Test the declared minimum Rust version on every pull request and push to `main`.

## 0.6.0 - 2026-09-23

- Add optional per-batch five-hour percentage caps that renew with provider windows without weakening the global 10% reserve.
- Add explicit cross-batch continuations from `quota_interrupted` or `quota_skipped` tasks, scheduled at the recorded reset and resumed from the stored Codex session when available.
- Add backward-compatible SQLite fields for five-hour accounting and typed task dependencies.
- Document how existing users update complete and marketplace-only installations without deleting local state.
- Build Linux release artifacts against GLIBC 2.31 and reject newer symbol requirements so the one-line installer supports Ubuntu 20.04 and newer.

## 0.5.0 - 2026-09-03

- Add local-history p50 and p90 predictions for token and weekly-percentage usage.
- Add cap assessments that warn when a proposed percentage or token budget looks insufficient or tight.
- Report prediction cohorts, sample counts, confidence, and cold-start limitations without changing user caps automatically.
- Add default terse replies for LimitWise skill usage and scheduled-task final summaries without changing scheduling or quota behavior.
- Simplify the README and add beginner-friendly GitHub Pages documentation.

## 0.4.0 - 2026-09-03

- Record and notify token usage for every scheduled run in both budget modes.
- Add rolling one-year, 30-day, and seven-day token summaries, seven local daily buckets, and individual seven-day run history through MCP and CLI.
- Backfill retained historical transcripts; preserve unavailable values instead of estimating them.

## 0.3.0 - 2026-09-02

- Add explicit percentage or token budget selection for each batch.
- Track shared batch token consumption from Codex JSONL transcripts and fail closed when token accounting is unavailable.
- Continue enforcing rolling five-hour and weekly availability for both budget modes.
- Replace a running service binary atomically and restart the daemon during upgrades.

## 0.2.0 - 2026-09-02

- Interpret new batch caps as percentage points of the full weekly limit; preserve the legacy remaining-percentage basis for existing batches.
- Add `after_previous` task chains that run only after the preceding task succeeds and block safely when it does not.
- Require exact local RFC3339 timestamps in planning output and return scheduled times in their retained IANA timezone.

## 0.1.0 - 2026-09-01

- Initial LimitWise plugin, MCP server, native daemon, service installers, quota adapter, SQLite state, model routing, task management, tests, and four-target release automation.
