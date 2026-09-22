---
id: agent-adapters-do-not-emit-content
severity: hard
statement: "Adapters return data; they never print, log or trace. Rendering happens in the shared layer, which is the only place that knows what may be shown."
outcome: decision-relevant-storage-evidence
audit: agent_adapters_do_not_emit_content
---

## Rationale

The privacy contract is about what leaves the process, not only about
what enters a struct field. An adapter debugging a header parse with
`eprintln!("{line}")` would print a conversation's first line to the
terminal and into any CI log — past every redaction the rendering layer
applies, and past the canary assertions, which check returned values
rather than stdout.

## Detection

No adapter module may contain `println!`, `eprintln!`, `print!`, `dbg!`,
`log::` or `tracing::`.

**Limits.** A `panic!`/`expect` message can still carry text; adapters
are written to return `unknown-format` rather than panic, and the
per-adapter `unknown_format_is_explicit_not_empty` test pins that.

## Runtime tests that complete it

- every adapter's `canary_content_never_appears_in_output`
