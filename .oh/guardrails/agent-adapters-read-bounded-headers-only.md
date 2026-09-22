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

No adapter may call `fs::read_to_string`, `fs::read(`, `.read_to_end(`,
`.read_to_string(`, `serde_json::from_reader(` or `BufReader::new(`.
`agents/bounded_io.rs` must exist and define `MAX_HEADER_BYTES`.

**Limits.** A whole-file read through an unusual API (`memmap`, a
third-party parser taking a path) is not in the list; the per-adapter
`identification_reads_no_more_than_header_cap` test is the behavioural
check.

## Runtime tests that complete it

- every adapter's `identification_reads_no_more_than_header_cap` and
  `canary_content_never_appears_in_output`
- `crates/core/tests/incremental_external_and_agent_measurement.rs` —
  header bytes counted per pass
