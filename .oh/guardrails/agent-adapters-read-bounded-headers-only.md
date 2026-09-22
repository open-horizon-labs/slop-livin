---
id: agent-adapters-read-bounded-headers-only
severity: hard
statement: "An adapter's only access to file contents is the shared capped reader agents::bounded_io::read_header(path, max_bytes), whose cap is at most 64 KiB. No adapter reads a whole file, streams lines, or deserializes from a reader. Everything else it knows comes from metadata."
outcome: decision-relevant-storage-evidence
audit: agent_adapters_read_bounded_headers_only
---

## Rationale

Privacy is a hard contract in this project: nothing may read past a
transcript's first line, and no path's contents may reach an AgentUnit
field, a log, or a test fixture. Today that holds because each adapter
was written carefully; it is not enforced. One `fs::read_to_string` in a
new adapter — the obvious way to parse a small JSON config — would read
an entire conversation transcript into memory and, with a serde error
message, into an error string.

The cap is also what makes identification cost bounded: header reads are
the per-session cost, so they must be provably small.

## Detection

No adapter function is in the derived unbounded-read set (every function that transitively calls `fs::read`, `fs::read_to_string`, `io::read_to_string`, a `read_to_string`/`read_to_end` method, `serde_json::from_reader` or `BufReader::new`), with the closure stopped only at `bounded_io::read_header` (which must name and stop on `MAX_HEADER_BYTES`, evaluate to at most 64 KiB, and neither list nor walk) and the declared-project handoff.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_read_bounded_headers_only/01-aliased-read-to-string`, `agent_adapters_read_bounded_headers_only/02-serde-from-reader`, `agent_adapters_read_bounded_headers_only/03-buf-reader-lines`, `agent_adapters_read_bounded_headers_only/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- every adapter's `identification_reads_no_more_than_header_cap` and
  `canary_content_never_appears_in_output`
- `crates/core/tests/incremental_external_and_agent_measurement.rs` —
  header bytes counted per pass
