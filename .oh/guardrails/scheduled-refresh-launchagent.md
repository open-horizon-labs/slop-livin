---
id: scheduled-refresh-launchagent
severity: hard
statement: "Where the platform has a user scheduler, a scheduled observation exists as an opt-in per-user job so a previous observation is there without a human running anything. Where it does not, the command refuses with a named reason and writes nothing -- it never installs a job that cannot run."
outcome: disk-growth-by-project
audit: scheduled_refresh_launchagent
---

## Rationale
Growth needs a baseline. Without a schedule the first question a user asks has no history behind it.

## Detection
The CLI exposes a `Schedule` subcommand and `core::schedule` drives `launchctl` where launchd exists. AST audit `scheduled_refresh_launchagent`. The platform-neutral half -- every `install` consults the scheduling capability, honours the answer, and does so before any write -- is audit `platform_capabilities_gate_their_backends`, with the runtime half in `schedule::tests::install_refuses_and_writes_nothing_where_there_is_no_scheduler`.

## Limits
Linux has no scheduler here at all (#86). The guardrail holds by refusal, which is the outcome it is for: a baseline nobody has is visible, and a job that silently never runs is not.
