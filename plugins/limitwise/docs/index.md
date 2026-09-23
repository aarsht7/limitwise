---
layout: default
title: LimitWise
---

# LimitWise

Schedule Codex work for a specific time while protecting your five-hour and weekly usage limits.

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are currently untested.

## Docs menu

[Home](index.md) | [Getting started](getting-started.md) | [Using LimitWise](using-limitwise.md) | [Troubleshooting](troubleshooting.md) | [Architecture](ARCHITECTURE.md)

LimitWise is designed for people who want to prepare work now and let Codex run it later. You describe the work in Plan mode, review the proposed model, effort, time, and budget, then confirm the schedule.

## What LimitWise does

- Runs one task at an exact local time.
- Runs a sequence where each task starts after the previous task succeeds.
- Supports percentage and token budgets.
- Predicts likely and conservative usage from your local task history.
- Uses terse replies by default while preserving exact technical values and warnings.
- Records statuses, errors, transcripts, and token use on your computer.
- Refuses to start when reliable quota information is unavailable.

## Start here

1. [Install LimitWise](getting-started.md#install-limitwise), or [update an existing installation](getting-started.md#update-limitwise).
2. [Run the small file example](getting-started.md#run-a-small-example).
3. [Learn how to schedule and manage tasks](using-limitwise.md).
4. Open [troubleshooting](troubleshooting.md) if a task does not run.

## Supported systems

LimitWise includes Linux and macOS code for x86-64 and ARM64, including Apple Silicon. Only Linux x86-64 has been tested. Windows is not supported in this version.

## Safe defaults

LimitWise keeps 10% of the rolling five-hour quota in reserve. It never silently changes the confirmed model or lowers reasoning effort. Ordinary work is skipped or interrupted when quota is low. An explicitly confirmed `continue_from_task_id` task may be deferred to the next provider reset while the global reserve remains exhausted.

Scheduled Codex runs use write access only inside the project you selected. Interactive approvals, external apps, web search, and network access are disabled.

[View source README](../README.md) · [Technical architecture](ARCHITECTURE.md)
