---
id: agent-adapters-read-bounded-headers-only
severity: hard
statement: "An adapter's only access to file contents is the shared capped reader agents::bounded_io::read_header(path, max_bytes), whose cap is at most 64 KiB. No adapter reads a whole file, streams lines, or deserializes from a reader. Everything else it knows comes from metadata."
outcome: decision-relevant-storage-evidence
audit: adapters_do_not_reach_gates, gate_paths_only_inside_gates
compile_fail:
  - content_reads_need_a_cap
  - caps_are_named_constants
  - no_unbounded_read_in_the_gate
runtime_tests:
  - identification_reads_no_more_than_header_cap
  - crates/core/tests/incremental_external_and_agent_measurement.rs
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

Mechanism: type, gate audit, runtime test.

**Type.** The gate's content read is `fs_gate::read::bounded_read(path, BoundedCap)`; a `BoundedCap` is one of the named constants or `header_at_most(n)` (clamped to the header cap), and there is no whole-file read of user content.

**Gate audit.** `std::io::{Read, BufRead, BufReader, copy, read_to_string}`, `std::fs` and `serde_json::from_reader`-style reads need `std::io`/`std::fs` paths the gate audit rejects outside `fs_gate`; adapters reach reads only through `IdentifyCtx`.

Retired 2026-09-22: the `agent_adapters_read_bounded_headers_only` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `content_reads_need_a_cap`, `caps_are_named_constants`, `no_unbounded_read_in_the_gate`.

## Runtime tests that complete it

- every adapter's `identification_reads_no_more_than_header_cap` and
  `canary_content_never_appears_in_output`
- `crates/core/tests/incremental_external_and_agent_measurement.rs` —
  header bytes counted per pass
