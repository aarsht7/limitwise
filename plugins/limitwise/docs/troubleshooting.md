---
layout: default
title: Troubleshooting
---

# Troubleshooting

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Docs menu

[Home](index.md) | [Getting started](getting-started.md) | [Using LimitWise](using-limitwise.md) | [Troubleshooting](troubleshooting.md) | [Architecture](ARCHITECTURE.md)

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

## A task says `missed`

LimitWise allows five minutes of delay. If the computer was asleep, offline, or the service was stopped for longer, the task is marked `missed` instead of running late.

Create a new schedule with a future exact time. A missed task is not deferred automatically.

## A task says `quota_skipped`

The task did not start because the five-hour reserve, weekly quota, or selected batch budget was exhausted. Ask Codex:

```text
Use $schedule-codex-tasks. Show my current quota and the full status of task TASK_ID.
```

LimitWise does not silently downgrade or postpone quota-short work. Schedule a new task after quota resets or choose a different confirmed budget.

## A task says `quota_interrupted`

Codex started, but a quota threshold was reached while it was running. Check the task transcript and repository state before scheduling the remaining work again.

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
