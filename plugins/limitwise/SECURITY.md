# Security policy

## Supported versions

Only the latest LimitWise release receives security fixes. Upgrade before reporting an issue against an older release.

## Reporting a vulnerability

Report suspected vulnerabilities through [GitHub private vulnerability reporting](https://github.com/aarsht7/limitwise/security/advisories/new). Include affected versions, reproduction steps, impact, and any proposed mitigation.

Do not disclose a vulnerability in a public issue before a fix is available. If private reporting is unavailable, contact the maintainer through the [GitHub profile](https://github.com/aarsht7) without including sensitive details publicly.

## Task permission profiles

LimitWise accepts only two closed task profiles:

- `restricted` is the default. It uses the `workspace-write` sandbox with workspace network access off and web search disabled.
- `networked` uses the same `workspace-write` sandbox but enables workspace network access and live web search. It requires a separate explicit `networked_confirmed: true` acknowledgement when scheduled, selected during an update, or inherited/selected for a retry.

Both profiles keep external apps disabled and approval policy set to `never`. Neither profile permits arbitrary command fragments, `danger-full-access`, another sandbox, external apps, or interactive approval. A profile may be updated only while the task is still `scheduled`.

For local-browser reports, include whether the issue bypasses loopback binding, per-launch bearer/bootstrap authentication, same-origin checks, JSON/body limits, fixed embedded routes, transcript-content exclusion, retry eligibility, permission-profile acknowledgement, explicit confirmation, or idempotency. Never include a live launch URL, bearer token, task prompt, transcript, credential, or private path in the report.

## Response

The maintainer will acknowledge a report, assess impact, prepare a fix when needed, and coordinate disclosure with the reporter. No response-time guarantee is provided.
