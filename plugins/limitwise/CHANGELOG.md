# Changelog

> Compatibility: LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested.

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
