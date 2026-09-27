---
layout: default
title: Using LimitWise
---

# Using LimitWise

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Schedule a task

Always prepare schedules in Plan mode. Give Codex:

- the absolute project path;
- an exact date and local time;
- the IANA timezone, such as `Europe/Paris` or `America/New_York`;
- the work to perform;
- a clear success condition;
- a percentage or token budget;
- a permission profile, if the default `restricted` profile is not appropriate;
- optionally, a five-hour batch cap.

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

LimitWise reads current quota and the installed Codex app-server's visible model catalog, inspects the project, chooses a supported model/effort pair, and estimates usage when enough local history exists. Manual selectors show only the reasoning efforts supported by the selected model. It then shows one proposal for review.

Nothing is scheduled in Plan mode. After you confirm the proposal, leave Plan mode and say `Create this confirmed schedule.`

## Choose a permission profile

`restricted` is the default for old and new clients. It uses `workspace-write` with network off and web search disabled.

`networked` enables network access and live web search. Selecting it requires a separate explicit acknowledgement in addition to the normal schedule or retry confirmation. It still uses `workspace-write`, disables external apps, and sets approval policy to `never`. It cannot enable `danger-full-access`, another sandbox, arbitrary command arguments, external apps, or interactive approval.

Retries inherit the source task profile unless you confirm a change. A stored task profile can be updated only before the task starts.

## Diagnose LimitWise without changing it

Run the human-readable report:

```sh
limitwise doctor
```

For scripts, request the versioned JSON contract:

```sh
limitwise doctor --json
```

Exit `0` means LimitWise is usable, including reports with warnings. Exit `1` means one or more checks failed. Exit `2` means the arguments were invalid or the diagnostic report could not be created. The equivalent read-only MCP tool is `diagnostics_snapshot {}`.

Doctor inspects existing state only. It does not create directories or databases, migrate SQLite, install or restart the service, or install or refresh the plugin. Diagnostic evidence is redacted and daemon logs are returned only as bounded error categories, never as raw log, prompt, or transcript content.

## Use terse mode

Terse output is built into `$schedule-codex-tasks`; there is no separate skill to install or invoke. LimitWise uses terse mode by default when the skill is active. It reduces extra prose in planning, status, and result summaries while keeping exact technical content unchanged.

Preserved exactly:

- commands, paths, code, and JSON;
- timestamps, timezones, task IDs, model names, and effort values;
- error strings, quota warnings, compatibility warnings, and destructive-action confirmations.

Switch modes in the same Codex conversation:

| Mode | Request |
| --- | --- |
| Terse default | `terse mode` |
| Fuller prose | `normal mode` |
| Shortest safe replies | `ultra terse` |

This is output-only. It does not compress your task prompt, reduce what scheduled Codex receives, change quota enforcement, or change scheduling logic.

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

### Optional five-hour budget

`five_hour_cap_percent` limits one batch's consumption in each provider five-hour window. For example, `5` allows at most five percentage points in that window, still bounded by the global 10% reserve. The allowance resets when the provider window resets. Omit this field when only the global reserve should apply. If five-hour telemetry is missing, LimitWise warns and continues with the weekly limit or token budget; neither this cap nor the global reserve can be enforced until the window returns.

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

## Retry or resume an immutable task

Task detail exposes exactly one action when the source is eligible:

- `quota_interrupted` or `quota_skipped`: **Continue after quota reset**. The run time comes from the stored provider reset; a passed reset is immediately eligible. LimitWise resumes the source Codex session when supported and available, otherwise it starts fresh with the stored predecessor/transcript/worktree context.
- `failed`, `blocked`, `missed`, or `cancelled`: **Retry as new run**. You must choose a future RFC3339 run time. This always starts a fresh Codex session and never imports session or transcript fallback context.

`completed`, `scheduled`, and `running` tasks have no retry action. Every action requires a fresh percentage or token budget, permits a confirmed model/effort selection, previews copied fields, attempt number, execution/reset time, resume/fallback behavior, and the current local estimate, then requires explicit confirmation. The global 10% rolling five-hour reserve remains unchanged whenever that telemetry is available.

Confirmation atomically creates a new batch/task with `source_task_id`, `attempt_kind`, and the next `attempt_number`. The source task and all source run records remain immutable. Repeating a confirmation with the same idempotency key returns the same attempt. No successor is created automatically. Missing or invalid quota metadata rejects quota resume; if the reserve is still exhausted when a confirmed quota resume becomes due, that same new attempt is deferred to the next provider reset.

Existing `schedule_batch` clients using `continue_from_task_id` remain compatible, but the GUI uses the explicit preview/confirm retry flow.

## Manage tasks

Ask Codex outside Plan mode:

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
| Preview a retry | `Preview a fresh retry for task TASK_ID at 2026-09-05T12:00:00+02:00 with a 1% weekly budget.` |
| Preview quota resume | `Preview continuing quota-limited task TASK_ID with a fresh 1% weekly budget.` |
| Confirm a new attempt | `Confirm the previewed retry/resume for task TASK_ID exactly once.` |

Only a task still marked `scheduled` can be changed or cancelled. A success-chained task has no clock time to change, but its prompt, success condition, project, model, and effort can be changed before it starts. A quota-reset continuation must remain in its predecessor worktree.

The browser dashboard groups every composer-created chain by batch. Before the first task starts, choose **Edit batch…** to change or reorder tasks, add tasks, or remove tasks. LimitWise pauses the first task while the editor stays connected. If the start time passes during editing, change it to a future RFC3339 time before saving; the server will reject an expired time. Save applies the whole revised chain atomically.

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
