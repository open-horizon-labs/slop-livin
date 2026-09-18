---
id: scheduled-refresh-launchagent
severity: hard
statement: "A scheduled observation exists as an opt-in per-user LaunchAgent so a previous observation is there without a human running anything."
outcome: disk-growth-by-project
audit: scheduled_refresh_launchagent
---

## Rationale
Growth needs a baseline. Without a schedule the first question a user asks has no history behind it.

## Detection
The CLI exposes a `Schedule` subcommand and `core::schedule` drives `launchctl`. AST audit `scheduled_refresh_launchagent`.
