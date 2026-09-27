# Privacy policy

LimitWise is a local scheduler. It stores schedules, task prompts, success criteria, project paths, retry/resume lineage, quota snapshots, run metadata, and Codex JSONL transcripts on the user's computer. These files remain until the user purges them.

LimitWise does not send its local database or transcript files to a LimitWise-operated server. The installer and updater access GitHub to download releases. When LimitWise launches Codex, Codex handles model requests under OpenAI's terms and privacy policies.

The optional browser UI is served by the local LimitWise process on `127.0.0.1`. It reads data through the authenticated local API, not SQLite. Its explicitly confirmed native folder picker returns only the selected canonical project path; it does not upload or enumerate project files in the browser. Task detail can display stored prompts, success criteria, project paths, errors, and transcript paths, but the API excludes transcript contents, per-run raw quota snapshots, and Codex session identifiers. The UI adds no remote telemetry.

Scheduled tasks run inside the selected project with workspace-write access. Interactive approvals, external apps, web search, and tool network access are disabled.

See the [uninstall and cleanup guide](docs/troubleshooting.md#remove-limitwise-complete-cleanup) to remove stored data.
