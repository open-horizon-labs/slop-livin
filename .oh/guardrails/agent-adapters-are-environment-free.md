---
id: agent-adapters-are-environment-free
severity: hard
statement: "An adapter knows only the home it is handed. It never reads the process environment, never resolves a home directory, and hardcodes no user path."
outcome: coverage-aware-storage-history
audit: agent_adapters_are_environment_free
---

## Rationale

Home resolution -- including every documented override variable -- is a
detector's job, and detectors are already fixture-injectable through
`locations::Environment`. An adapter that reads `HOME` itself is
untestable without touching the real machine, which is exactly what the
privacy rule forbids: fixtures must be synthetic, and no test may read a
real tool home.

## Detection

No adapter module may reference `std::env`, `env::var`, `dirs::` or
`home_dir`, nor contain the string literal `"HOME"` or a literal
beginning `/Users/` or `/home/`.

**Limits.** A helper that reads the environment on the adapter's behalf
would need to live in a non-adapter module, where it is visible and
reviewable.

## Runtime tests that complete it

- every adapter's own tests, all of which inject a `tempfile` home
