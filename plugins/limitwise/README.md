# LimitWise

LimitWise is a Codex plugin that runs coding tasks later without ignoring your Codex usage limits.

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

It can:

- run a task at an exact local time;
- run several tasks in order;
- use either a weekly percentage budget or a token budget;
- answer tersely by default while preserving exact technical details;
- warn when a budget may be too small;
- stop or skip work when quota is unavailable or nearly exhausted;
- record task status and token usage locally.

LimitWise includes Linux and macOS support, but only Linux x86-64 has been tested.

## Requirements

- Codex installed and signed in
- Linux x86-64 (tested), or Linux/macOS on another architecture (untested)

## Install from GitHub

Linux x86-64 installs without a platform prompt. Other supported release platforms show the compatibility warning and require confirmation.

```sh
curl -fsSL https://raw.githubusercontent.com/aarsht7/limitwise/main/install.sh | sh
```

The installer checks Codex sign-in, downloads a prebuilt release, verifies its SHA-256 checksum, installs the GitHub marketplace, and installs `limitwise@limitwise`. It asks separately before installing the background service. Open a new Codex conversation afterward.

What this method installs:

- plugin: `limitwise@limitwise`
- marketplace source: `aarsht7/limitwise`
- prebuilt binary in user data directory
- optional background service (`systemd --user` on Linux, `launchctl` LaunchAgent on macOS)

Technical users can install the marketplace without the helper:

```sh
codex plugin marketplace add aarsht7/limitwise
codex plugin add limitwise@limitwise
```

That direct flow does not install a prebuilt binary or background service.

If you need a local machine-compatible binary, build from source with Rust 1.71+ (see [getting-started guide](docs/getting-started.md)).

Quick verification after install:

```sh
codex plugin list
codex plugin marketplace list
```

For service status:

```sh
# Linux
systemctl --user status limitwise.service

# macOS (untested, including Apple Silicon)
launchctl print gui/$(id -u)/io.openai.limitwise
```

If service setup fails with a runtime loader error such as `GLIBC_* not found`, build and run from source instead of the prebuilt binary.

## Uninstall and cleanup

For complete removal, use the method-based cleanup guide in [docs/troubleshooting.md](docs/troubleshooting.md#remove-limitwise-complete-cleanup).

In short, full cleanup usually includes:

```sh
codex plugin remove limitwise
codex plugin marketplace remove limitwise
```

and, when installed via the one-line installer, running uninstall with purge:

```sh
# Linux
${XDG_DATA_HOME:-$HOME/.local/share}/limitwise/bin/limitwise uninstall --purge

# macOS (untested, including Apple Silicon)
"$HOME/Library/Application Support/LimitWise/bin/limitwise" uninstall --purge
```

## Use LimitWise

Open Plan mode and tell Codex what to run, where to run it, the exact local time, timezone, and budget. Start with:

```text
Use $schedule-codex-tasks. Plan this batch, but do not schedule it yet.
```

LimitWise checks your quota, chooses a model and reasoning effort, estimates usage from previous local runs, and shows a proposal. Review it and confirm it. Leave Plan mode, then say:

```text
Create this confirmed schedule.
```

Nothing is scheduled while you are still in Plan mode.

### Terse mode

Terse output is built into `$schedule-codex-tasks`; there is no separate skill to install or invoke. LimitWise uses terse mode by default whenever the skill is active. Replies are shorter, but exact commands, paths, code, JSON, timestamps, task IDs, model names, effort values, error strings, quota warnings, compatibility warnings, and destructive-action confirmations stay unchanged.

Use these switches in the same Codex conversation:

| What you want | What to say |
| --- | --- |
| Default shorter replies | `terse mode` |
| Fuller prose | `normal mode` |
| Shortest replies | `ultra terse` |

Terse mode only changes user-facing wording and scheduled-task final summaries. It does not compress your prompts, change model routing, alter quota checks, or modify scheduling behavior.

## Quick local check

Create an empty folder:

```sh
mkdir -p /absolute/path/to/limitwise-test
```

Choose a time about three minutes ahead and copy the result:

```sh
# Linux
date -d '+3 minutes' --iso-8601=seconds

# macOS (untested, including Apple Silicon)
date -v+3M '+%Y-%m-%dT%H:%M:%S%z' | sed -E 's/([+-][0-9]{2})([0-9]{2})$/\1:\2/'
```

In Plan mode, paste this request and replace the path, timestamp, and timezone:

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

Confirm the proposal, leave Plan mode, and say `Create this confirmed schedule.` Then ask:

```text
Use $schedule-codex-tasks and list all LimitWise tasks with their status.
```

When all three tasks are `completed`, open `status.txt`. It should contain:

```text
It's working
Edited by LimitWise
Modified by LimitWise
```

## Common requests

| What you want | What to ask Codex |
| --- | --- |
| See scheduled tasks | `Use $schedule-codex-tasks and list my scheduled LimitWise tasks.` |
| See all task history | `List all LimitWise tasks with IDs, times, models, and statuses.` |
| Check one task | `Show the full status and last error for LimitWise task TASK_ID.` |
| Change a task | `Update LimitWise task TASK_ID to run at <EXACT TIMESTAMP> in <TIMEZONE>.` |
| Cancel a task | `Cancel scheduled LimitWise task TASK_ID.` |
| Check quota | `Show my current five-hour and weekly Codex usage and reset times.` |
| Check token usage | `Show my LimitWise token stats for the last year, month, week, and each of the last seven days.` |

Only tasks that have not started can be changed or cancelled.

## Budgets

- **Percentage:** `1` means one percentage point of the full weekly limit. The allowance renews when the weekly window resets.
- **Tokens:** one token cap is shared by the whole batch. A running task can slightly exceed it because Codex reports usage at turn boundaries.

Both modes keep a 10% reserve in the rolling five-hour window. If quota data is missing, LimitWise does not start Codex.

## More help

- [Getting started](docs/getting-started.md)
- [Scheduling and managing tasks](docs/using-limitwise.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Architecture](docs/ARCHITECTURE.md)

LimitWise stores its database and transcripts privately on your computer. It runs Codex with write access only to the selected project, no interactive approvals, and no external apps or network access.

Licensed under the [MIT License](LICENSE).
