---
layout: default
title: Troubleshooting
---

# Troubleshooting

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

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

## Remove LimitWise

Remove the background service but keep schedules and history:

```sh
./scripts/launch-limitwise uninstall
```

Remove the service and permanently delete all LimitWise schedules, transcripts, and history:

```sh
./scripts/launch-limitwise uninstall --purge
```

Then remove the plugin from Codex:

```sh
codex plugin remove limitwise
```

`--purge` cannot be undone.
