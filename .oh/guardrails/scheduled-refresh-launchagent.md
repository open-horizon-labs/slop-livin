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

The CLI has `Command::Schedule`. In the scheduler (derived: the module that writes the LaunchAgent plist, with child modules), every subprocess spawn must be reachable from the CLI's `main` -- what schedules is what runs -- and a `launchctl` spawn must be.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `scheduled_refresh_launchagent/01-no-schedule-subcommand`, `scheduled_refresh_launchagent/02-cron-instead-of-launchd`, `scheduled_refresh_launchagent/03-schedule-variant-renamed-away`, `scheduled_refresh_launchagent/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

