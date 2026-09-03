---
layout: default
title: Getting started
---

# Getting started

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Before you begin

You need:

- Codex installed and signed in;
- Linux x86-64 (tested), or Linux/macOS on another architecture (untested);
- `curl` and `tar`.

## Install LimitWise

Run:

```sh
curl -fsSL https://raw.githubusercontent.com/aarsht7/limitwise/main/install.sh | sh
```

The installer checks your Codex sign-in, detects your platform, downloads the matching GitHub Release archive, and verifies its SHA-256 checksum. Linux x86-64 proceeds automatically. Every untested platform requires confirmation.

It then installs the marketplace and plugin:

```sh
codex plugin marketplace add aarsht7/limitwise
codex plugin add limitwise@limitwise
```

The background service is a separate opt-in prompt. Only approval runs:

```sh
limitwise setup
```

This creates a systemd user service on Linux or a LaunchAgent on macOS. The macOS LaunchAgent path is untested, including on Apple Silicon. Open a new Codex conversation after installation.

## Build from source

Rust 1.71 or newer is required. From `plugins/limitwise`:

```sh
cargo build --release
./target/release/limitwise setup
```

## Run a small example

Create an empty project folder:

```sh
mkdir -p /absolute/path/to/limitwise-test
```

Generate an exact time about three minutes ahead:

```sh
# Linux
date -d '+3 minutes' --iso-8601=seconds

# macOS (untested, including Apple Silicon)
date -v+3M '+%Y-%m-%dT%H:%M:%S%z' | sed -E 's/([+-][0-9]{2})([0-9]{2})$/\1:\2/'
```

Open Codex Plan mode. Paste this request after replacing the project path, timestamp, and timezone:

```text
Use $schedule-codex-tasks. Plan this batch, but do not schedule it yet.

Project: /absolute/path/to/limitwise-test
Timezone: Europe/Paris
Weekly cap: 1 percentage point of the full weekly limit.

Task 1 — run at <EXACT TIMESTAMP>:
Create status.txt containing exactly:
It's working
Success: status.txt exists with exactly that line.

Task 2 — run immediately after Task 1 succeeds:
Replace status.txt with exactly:
It's working
Edited by LimitWise
Success: status.txt contains exactly those two lines.

Task 3 — run immediately after Task 2 succeeds:
Replace status.txt with exactly:
It's working
Edited by LimitWise
Modified by LimitWise
Success: status.txt contains exactly those three lines.
```

Codex will show a proposal. Check the path, time, timezone, budget, model, and effort. Confirm it, leave Plan mode, then say:

```text
Create this confirmed schedule.
```

When asked, allow LimitWise to set up its background service. Do not close or suspend the computer before the first task is due.

Check progress by asking:

```text
Use $schedule-codex-tasks and list all LimitWise tasks with their status.
```

After all tasks show `completed`, `status.txt` should contain:

```text
It's working
Edited by LimitWise
Modified by LimitWise
```

## Next steps

- [Schedule real work](using-limitwise.md#schedule-a-task)
- [Use a token budget](using-limitwise.md#choose-a-budget)
- [Change or cancel a task](using-limitwise.md#manage-tasks)
- [Fix a task that did not run](troubleshooting.md)
