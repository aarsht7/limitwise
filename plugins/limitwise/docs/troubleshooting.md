---
layout: default
title: Troubleshooting
---

# Troubleshooting

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Docs menu

[Home](index.md) | [Demos](demos.md) | [Getting started](getting-started.md) | [Using LimitWise](using-limitwise.md) | [Local browser UI](local-browser-ui.md) | [Troubleshooting](troubleshooting.md) | [Architecture](ARCHITECTURE.md)

## The local browser UI does not open

Run it without the browser opener and copy the printed launch URL into a browser on the same machine:

```sh
limitwise ui --no-open
```

Do not share that URL; its fragment is a single-use browser bootstrap. If the fragment has already been consumed, stop the process and launch a new UI session. A port collision from `--port <port>` is fatal; omit the flag to choose an available loopback port. The UI never binds externally, and it must not be placed behind a proxy or tunnel.

If the page reports stale or unavailable quota, run `limitwise doctor` and `limitwise usage`. Missing weekly telemetry, ambiguous telemetry, or a stale full snapshot blocks scheduling. If only five-hour telemetry is unavailable, LimitWise warns and continues with the weekly limit or token budget; its global reserve and any five-hour batch cap cannot be enforced. Browser task detail intentionally shows only a transcript path, never transcript contents.

## Start with read-only diagnostics

```sh
limitwise doctor
limitwise doctor --json
```

Doctor does not repair or mutate anything. A warning still exits `0`; a blocking failure exits `1`. Common check IDs and remedies:

| Check | Meaning | Remedy |
| --- | --- | --- |
| `codex.executable` | Codex is missing, cannot start, or returned malformed version output. | Install or repair Codex, then run `codex --version`. |
| `codex.login` | Codex is logged out or login status is unavailable. | Run `codex login`. |
| `codex.quota` | Weekly telemetry is missing, telemetry is ambiguous/invalid/timed out, or optional five-hour telemetry is unavailable. | Run `limitwise usage`; blocking failures remain fail-closed, while missing five-hour telemetry produces a warning and leaves the weekly limit or token budget active. |
| `plugin.installed` | `limitwise@limitwise` is absent from the Codex plugin inventory. | Run `codex plugin add limitwise@limitwise`. |
| `plugin.mcp_configuration` | The LimitWise MCP server is absent from the Codex MCP inventory. | Inspect `codex mcp list`, then refresh the plugin registration if needed. |
| `storage.paths` | Existing local state has unsafe ownership or group/other permissions. | Restrict directories to `0700` and files to `0600`; verify ownership before changing it. |
| `storage.database` | SQLite is corrupt, unreadable, has missing core schema, or foreign-key violations. | Back up the files and restore or repair from a known-good copy; doctor never migrates or repairs. |
| `service.state` | The optional service is absent, stopped, unreadable, points at a stale binary, or cannot be queried from a sandboxed MCP session. | Absence or sandbox-limited verification is only a warning. Run `limitwise doctor` in a user terminal to verify live state; for a broken installed service, inspect it before choosing whether to run `limitwise setup`. |
| `logs.daemon_errors` | Recent error categories were found in a bounded stderr tail. | Inspect the private local log directly; the report intentionally excludes raw lines. |

## Codex does not see LimitWise

Check the plugin:

```sh
codex plugin list
```

If `limitwise@limitwise` is missing, add the GitHub marketplace, then reinstall:

```sh
codex plugin marketplace add aarsht7/limitwise
codex plugin add limitwise@limitwise
```

Open a new Codex conversation after installing or updating the plugin. Existing conversations may still use the older plugin definition.

## LimitWise is outdated

For a complete installation, rerun the installer:

```sh
curl -fsSL https://raw.githubusercontent.com/aarsht7/limitwise/main/install.sh | sh
```

Choose `y` at the background-service prompt if you use scheduled execution. The installer replaces the binary, refreshes the marketplace, and restarts the service without deleting existing schedules or local data.

For a marketplace-only installation, run:

```sh
codex plugin marketplace upgrade limitwise
codex plugin add limitwise@limitwise
```

Then open a new Codex conversation.

## The background service is not running

Install or restart it from the plugin directory:

```sh
./scripts/launch-limitwise setup
```

On Linux, inspect the service and recent messages:

```sh
systemctl --user status limitwise.service
journalctl --user -u limitwise.service -n 100 --no-pager
```

On macOS (untested, including Apple Silicon), inspect the service and recent error log:

```sh
launchctl print gui/$(id -u)/io.openai.limitwise
tail -n 100 "$HOME/Library/Application Support/LimitWise/logs/daemon.stderr.log"
```

If a run says `quota telemetry unavailable: cannot start Codex app-server: No such file or directory (os error 2)`, the background service could not resolve the Codex executable from its restricted service `PATH`. Update LimitWise, then run confirmed service setup again. Current setup stores the resolved Codex path in the service definition, and runtime discovery also checks `$HOME/.local/bin/codex` plus the Codex standalone-install path.

After service setup, retry the skipped task as a new immutable attempt. **Continue after quota reset** is correctly rejected for this error because no valid quota snapshot was captured.

## Stop or retry a running task

Open **Operations**, select the running task, and confirm **Stop running task**. Stop is cooperative: the daemon receives the request, interrupts Codex, records `cancelled`, then exposes **Retry as new run** with a fresh budget and future time. Pause is unavailable because Codex `exec` has no safe persistent pause/resume contract.

## A task says `missed`

LimitWise allows five minutes of delay. If the computer was asleep, offline, or the service was stopped for longer, the task is marked `missed` instead of running late.

Create a new schedule with a future exact time. A missed task is not deferred automatically.

## A task says `quota_skipped`

The task did not start because the five-hour reserve, weekly quota, or selected batch budget was exhausted. Ask Codex:

```text
Use $schedule-codex-tasks. Show my current quota and the full status of task TASK_ID.
```

LimitWise does not silently downgrade ordinary quota-short work. Open the task detail and use **Continue after quota reset**, or ask for `preview_retry_task` with a fresh budget before explicitly confirming `retry_task`. Only `quota_skipped` and `quota_interrupted` tasks with valid recorded five-hour reset metadata can use quota resume.

## A task says `quota_interrupted`

Codex started, but a quota threshold was reached while it was running. Check the task transcript and repository state, then use the task detail's **Continue after quota reset** preview and explicitly confirm the new attempt. LimitWise never creates the follow-up automatically and never overwrites the interrupted task or its run history.

## A continuation remains `scheduled`

The provider five-hour window reset, but global usage was still at or above the 90% threshold. LimitWise moved the same continuation to the next reported reset rather than failing it. Inspect current quota and task status before changing it.

## Quota is unavailable

LimitWise needs unambiguous five-hour and weekly quota data from Codex. Confirm that the Codex CLI is installed and signed in:

```sh
codex login
./scripts/launch-limitwise usage
```

If quota data is still unavailable, LimitWise will not launch scheduled Codex work. This is intentional.

## View token history

Ask Codex:

```text
Use $schedule-codex-tasks and show my LimitWise token stats for the last year, month, week, and each of the last seven days.
```

You can also view the local JSON output:

```sh
./scripts/launch-limitwise stats
```

Runs stopped before Codex starts use zero tokens. If Codex started but did not report its final usage, LimitWise shows the token count as unavailable instead of guessing.

## Remove LimitWise (complete cleanup)

Use the flow that matches your install method.

Before removing anything, close active Codex conversations that are currently using LimitWise.

### Method A: Installed with the one-line installer (`curl ... | sh`)

1. Run uninstall with purge using the installed binary.

```sh
# Linux
${XDG_DATA_HOME:-$HOME/.local/share}/limitwise/bin/limitwise uninstall --purge

# macOS (untested, including Apple Silicon)
"$HOME/Library/Application Support/LimitWise/bin/limitwise" uninstall --purge
```

2. Remove Codex plugin registration.

```sh
codex plugin remove limitwise
```

3. Remove the marketplace source installed by the installer.

```sh
codex plugin marketplace remove limitwise
```

If your marketplace entry was saved with the full source name, remove that too:

```sh
codex plugin marketplace remove aarsht7/limitwise
```

4. Optional: remove plugin cache leftovers.

```sh
rm -rf ~/.codex/plugins/cache/limitwise
rm -rf ~/.codex/plugins/limitwise
```

### Method B: Installed only with marketplace commands

If you installed with:

```sh
codex plugin marketplace add aarsht7/limitwise
codex plugin add limitwise@limitwise
```

Then remove:

```sh
codex plugin remove limitwise
codex plugin marketplace remove limitwise
```

No prebuilt binary cleanup is needed unless you manually installed one.

If `codex plugin marketplace remove limitwise` fails, run `codex plugin marketplace list` and remove the exact entry name shown there.

### Method C: Running from source checkout

If you used `./scripts/launch-limitwise`, run from `plugins/limitwise`:

```sh
./scripts/launch-limitwise uninstall --purge
codex plugin remove limitwise
codex plugin marketplace remove limitwise
```

Optional: delete build artifacts and local repo checkout:

```sh
rm -rf target
# Optional if you want to remove the local clone entirely:
# rm -rf /path/to/LimitWise-codex-plugin
```

### Emergency cleanup if uninstall command cannot run

Use this when the installed binary fails to start (for example `GLIBC_* not found`).

```sh
# Linux service cleanup
systemctl --user disable --now limitwise.service 2>/dev/null || true
rm -f ~/.config/systemd/user/limitwise.service
systemctl --user daemon-reload 2>/dev/null || true

# macOS service cleanup (untested, including Apple Silicon)
launchctl bootout gui/$(id -u)/io.openai.limitwise 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/io.openai.limitwise.plist"

# Data and binary cleanup
rm -rf ~/.local/share/limitwise
[ -n "${XDG_DATA_HOME:-}" ] && rm -rf "$XDG_DATA_HOME/limitwise"
[ -n "${LIMITWISE_HOME:-}" ] && rm -rf "$LIMITWISE_HOME/.local/share/limitwise"
rm -rf "$HOME/Library/Application Support/LimitWise"

# Plugin and marketplace cleanup
codex plugin remove limitwise 2>/dev/null || true
codex plugin marketplace remove limitwise 2>/dev/null || true
codex plugin marketplace remove aarsht7/limitwise 2>/dev/null || true

# Optional cache cleanup
rm -rf ~/.codex/plugins/cache/limitwise
rm -rf ~/.codex/plugins/limitwise
```

### Verify cleanup

```sh
codex plugin list
codex plugin marketplace list

# Linux
systemctl --user status limitwise.service

# macOS (untested, including Apple Silicon)
launchctl print gui/$(id -u)/io.openai.limitwise
```

Expected result:

- no `limitwise@limitwise` in `codex plugin list`;
- no LimitWise marketplace entry;
- no active LimitWise user service.

`--purge` permanently deletes schedules, transcripts, and local usage history. This action cannot be undone.
