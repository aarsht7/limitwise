---
layout: default
title: Local browser UI
---

# Local browser UI

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

The local UI exposes the existing scheduler, diagnostics, quota, statistics, task-management, and service-setup operations without reading SQLite in browser code.

## Launch

```sh
limitwise ui
limitwise ui --no-open
limitwise ui --port 43121
```

The default chooses an available port on `127.0.0.1` and asks the operating system to open the browser. `--no-open` prints a launch URL and remains in the foreground. The URL carries a one-time bootstrap secret in its fragment, not the long-lived bearer token. Do not share the URL. The page removes the fragment from visible navigation before exchanging it and keeps the returned per-launch bearer only in browser session storage.

Press `Ctrl-C` or send `SIGTERM` to stop the UI. `limitwise ui` never installs itself as a background service. Scheduler execution still uses the separately confirmed user service.

## What is available

- Dashboard: five-hour and weekly quota/reset times, diagnostic health, local usage summaries, task filters, server-side sorting, 20-batch pages, every active/terminal status, and stored task detail. Tasks created as one composer chain stay in one batch group. Batch lists default to newest-created first.
- Task detail: specification, batch budget, runs, errors, dependency, transcript path, attempt lineage, exactly one eligible retry/resume action, and removal from normal task lists. Removal archives the task row instead of deleting it, preserving run/token/model metadata and local usage totals.
- Composer: Simple and Advanced modes share a project selector, required weekly-percentage limit, task permission profiles, and visual linear-chain editor. The project defaults to the directory where `limitwise ui` was launched; **Choose folder…** opens the operating system's folder picker, and the absolute path remains editable as a fallback. Model selectors come from the installed Codex app-server's `model/list` catalog, and effort selectors show only the values supported by the selected model. **Plan with GPT** runs Codex Plan mode with a user-selected planning model (`gpt-6-sol` by default) and `high` reasoning, then returns an execution prompt, success criteria, and bounded difficulty/model/effort route for every step. The selected project, weekly limit, and per-task execution permissions are carried unchanged into the reviewed schedule. The user can revise the chain or explicitly confirm it; planning never schedules by itself. Advanced mode additionally exposes manual task routing without the GPT planning step. Editable run times start at the current local time plus a one-minute scheduling margin.
- Operations: the same sorting and 20-task pages, pending-task update/cancellation, running-task stop, terminal-task retry/resume, task-list removal with retained run metadata, and confirmed service setup.
- Appearance: a gradient-free, high-contrast dark theme is the default. The header can switch to a warmer off-white/cream light theme, and that preference is retained in browser local storage.

The composer cannot create branches, joins, or cycles. The first ordinary task requires a future RFC3339 `run_at` with an explicit offset. Every later task emits only `after_previous: true`. Retry/resume starts from eligible task detail: quota outcomes with a valid recorded reset get **Continue after quota reset**; quota failures without a usable snapshot and failed/blocked/missed/cancelled outcomes get **Retry as new run**. Completed, scheduled, and running tasks show no attempt action.

Before the first task starts, **Edit batch…** opens the complete pending chain. Existing tasks can be changed or reordered, new tasks can be added, and tasks can be removed. Opening the editor creates a renewable 60-second local edit lease; while heartbeats continue, both daemon discovery and the final atomic claim reject the first task. Saving atomically rewrites the linear chain and releases the lease. Discarding releases it immediately; closing the editor attempts release and otherwise lets it expire. If `run_at` passes while editing, the UI warns and the server rejects save until the first task is assigned a future time.

Simple planning launches an ephemeral, read-only Codex app-server thread in Plan collaboration mode. The user may choose any visible model that supports `high`; the default is `gpt-6-sol`, and planning reasoning remains `high`. It consumes Codex quota, can inspect only the selected project read-only, cannot use network or apps, and must preserve the submitted task count and order. Planning access is separate from the per-task `restricted` or separately acknowledged `networked` profile used later for execution. Planner output is restricted to `simple/gpt-6-luna/low`, `standard/gpt-6-sol/medium`, `complex/gpt-6-astra/high`, or `exceptional/gpt-6-astra/xhigh`; `max` and `ultra` are available only through explicit manual selection when the chosen model reports them. Uncertain work defaults to `standard/gpt-6-sol/medium`. Both modes require a total-weekly percentage limit greater than zero and no more than 100. When Codex exposes a five-hour quota window, the global 10% reserve remains authoritative. Choose Advanced when difficulty, execution model, or effort needs manual control.

The retry form requires a fresh budget and allows model/effort selection plus a future time for ordinary retries. Its read-only preview shows copied fields, attempt number, reset/run time, fresh-session or quota-resume fallback behavior, and the current estimate. Confirmation uses a per-review idempotency key, creates a new batch/task once, refreshes lineage, and leaves predecessor history unchanged. The global 10% rolling five-hour reserve remains authoritative whenever that telemetry is available.

A running task can be stopped from Operations. The daemon checks stop requests at least twice per second, interrupts the Codex process group, records the attempt as `cancelled`, and then exposes **Retry as new run**. Pause is not offered: Codex `exec` has no safe persistent pause/resume contract, and an operating-system process freeze would block the single scheduler and become unsafe across daemon restarts.

Removing a scheduled task cancels it before hiding it. Running tasks must be stopped first. Archived tasks are excluded from Dashboard, Operations, normal lists, and daemon claims; their task rows and run records remain in SQLite so token totals, model/effort, status, and historical estimates remain intact.

Missing weekly telemetry or ambiguous telemetry blocks confirmation. Missing five-hour telemetry shows a warning and leaves the weekly limit or token budget active; the global reserve and any five-hour batch cap cannot be enforced until that window returns. Confirmation derives a stable idempotency key from the canonical frozen payload, so a repeated identical submission resolves through existing `schedule_batch` idempotency.

## Local security boundary

- The listener is constructed only for `127.0.0.1`; there is no external-bind option.
- Each launch gets independent 256-bit operating-system-random bootstrap and bearer secrets.
- `/api/v1` requires the exact loopback Host and same-origin browser context, `Content-Type: application/json`, and the bearer after the one-time bootstrap.
- JSON bodies are limited to 64 KiB and headers to 32 KiB. Chunked request bodies are rejected.
- Responses use a fixed error envelope: `{"error":{"code":"...","message":"...","fields":{}}}`.
- Static routes are fixed embedded assets. The authenticated project endpoint returns the UI launch directory or invokes the native folder picker after explicit confirmation; it does not accept a browser-supplied path to enumerate files. The server never maps URL paths to the filesystem and emits no permissive CORS header.

On Linux, folder selection uses `zenity` and falls back to `kdialog`; if neither is installed, type the absolute project path directly. On macOS it uses the system folder chooser through `osascript`.
- Content Security Policy, frame denial, no-sniff, no-referrer, and no-store API headers are emitted by the Rust server.

Any local process running as the same user is inside the machine's trust boundary. Close the UI when finished, do not paste its launch URL elsewhere, and do not expose its port through a proxy or tunnel.

## Browser and source-build baseline

The source targets current Chromium, Firefox, and Safari releases with ES2022, Web Crypto, modules, session storage, and CSS grid support. Exact Linux and ARM macOS browser acceptance remains required before C03 can be considered complete.

The production binary embeds `ui/dist`; Node is not needed at runtime. Frontend development is pinned to Node `22.11.0` and npm `10.9.0`:

```sh
cd plugins/limitwise/ui
npm ci
npm run typecheck
npm test
npm run build
npm run e2e
```

Run the frontend build before a Rust release build so `include_bytes!` captures the generated `dist/index.html`, `dist/assets/app.js`, and `dist/assets/app.css`. These commands are documented integration gates; their results must not be assumed without executing them.
