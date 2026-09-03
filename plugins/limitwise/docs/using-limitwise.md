---
layout: default
title: Using LimitWise
---

# Using LimitWise

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Docs menu

[Home](index.md) | [Getting started](getting-started.md) | [Using LimitWise](using-limitwise.md) | [Troubleshooting](troubleshooting.md) | [Architecture](ARCHITECTURE.md)

## Schedule a task

Always prepare schedules in Plan mode. Give Codex:

- the absolute project path;
- an exact date and local time;
- the IANA timezone, such as `Europe/Paris` or `America/New_York`;
- the work to perform;
- a clear success condition;
- a percentage or token budget.

Example:

```text
Use $schedule-codex-tasks. Plan this batch, but do not schedule it yet.

Project: /home/me/projects/example
Timezone: Europe/Paris
Budget mode: percentage
Weekly cap: 2 percentage points of the full weekly limit.

Run at 2026-09-05T09:30:00+02:00:
Update the project README with setup instructions.
Success: the README contains installation and usage sections.
```

LimitWise reads current quota, inspects the project, chooses a model and reasoning effort, and estimates usage when enough local history exists. It then shows one proposal for review.

Nothing is scheduled in Plan mode. After you confirm the proposal, leave Plan mode and say `Create this confirmed schedule.`

## Choose a budget

### Percentage budget

A percentage budget uses percentage points from the full weekly limit. A cap of `2` means at most two percentage points for the batch in each weekly reset window. It does not mean 2% of the currently remaining quota.

Use this when you think about your Codex allowance as a share of the weekly limit.

### Token budget

A token budget is one input-plus-output token limit shared by the whole batch:

```text
Budget mode: tokens
Batch token cap: 150000 total input-plus-output tokens.
```

Use this when you want a concrete token ceiling. Token totals become available when Codex finishes a turn, so the active task can exceed the cap before LimitWise can stop later work.

## Chain tasks

The first task needs an exact time. Later tasks can start immediately after the previous task succeeds:

```text
Task 1 — run at 2026-09-05T09:30:00+02:00:
Create the database migration.

Task 2 — run immediately after Task 1 succeeds:
Update the application code for the migration.

Task 3 — run immediately after Task 2 succeeds:
Update the documentation.
```

If one task does not complete successfully, the next task is marked `blocked` and does not run.

## Manage tasks

Ask Codex in normal mode:

| Action | Request |
| --- | --- |
| List scheduled tasks | `Use $schedule-codex-tasks and list my scheduled LimitWise tasks.` |
| List all history | `List all LimitWise tasks with IDs, times, models, and statuses.` |
| Inspect one task | `Show the full status, token use, and last error for LimitWise task TASK_ID.` |
| Change the time | `Update scheduled task TASK_ID to run at 2026-09-05T11:00:00+02:00 in Europe/Paris.` |
| Change the work | `Update scheduled task TASK_ID with this prompt and success condition: ...` |
| Cancel a task | `Cancel scheduled LimitWise task TASK_ID.` |
| Check quota | `Show current five-hour and weekly usage, remaining quota, and reset times.` |
| Check token history | `Show LimitWise token stats for the last year, month, week, each of the last seven days, and each recent run.` |
| Estimate work | `Estimate usage for these tasks and warn me if this cap looks too low: ...` |

Only a task still marked `scheduled` can be changed or cancelled. A chained task has no clock time to change, but its prompt, success condition, project, model, and effort can be changed before it starts.

## Understand statuses

| Status | Meaning |
| --- | --- |
| `scheduled` | Waiting for its time or previous task. |
| `running` | Codex is working on it. |
| `completed` | Work finished successfully. |
| `failed` | Codex ran but the task failed. |
| `blocked` | The task required an approval or disallowed capability, or its previous task did not complete successfully. |
| `quota_skipped` | Quota or batch budget was too low before launch. |
| `quota_interrupted` | Quota reached its limit while Codex was running. |
| `missed` | The computer or service was unavailable for more than five minutes after the due time. |
| `cancelled` | The task was cancelled before it started. |

## Local data

LimitWise keeps schedules, results, quota snapshots, and token totals on your computer:

- Linux: `~/.local/share/limitwise`
- Linux with `XDG_DATA_HOME`: `$XDG_DATA_HOME/limitwise`
- macOS: `~/Library/Application Support/LimitWise`

The SQLite database and JSONL transcripts use private, user-only permissions. History is kept until you purge it.
