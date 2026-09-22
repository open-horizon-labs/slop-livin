---
id: build-adapters-read-bounded-manifests-only
severity: hard
statement: "A build adapter's only file-content access is the shared, capped `build_adapters::bounded_io::read_manifest(path, cap)`, for named manifest, lockfile and fingerprint files. Never a whole file, never an arbitrary path."
outcome: disk-growth-by-project
audit: build_adapters_read_bounded_manifests_only
---

## Rationale

The cost argument and the safety argument point the same way.

Cost: a `node_modules` tree holds one `package.json` per installed
package -- tens of thousands in a monorepo -- and a `pnpm-lock.yaml` can
be tens of megabytes. An adapter that reads whole files reads gigabytes
to answer "which packages are installed", on every refresh. The cap is
also what makes the cost *provable*: manifest bytes are counted
(`work_counters`), so `identification_reads_no_more_than_manifest_cap`
can assert a number rather than a hope.

Safety: an unbounded read of an arbitrary path under a build directory is
an unbounded read of whatever the build put there. A `.env` copied into
`dist/`, a test fixture holding a token, a core dump in `target/`: none
of that belongs in a serde error message.

## Detection

Adapter modules may not use `fs::read_to_string`, `fs::read`,
`read_to_end`, `serde_json::from_reader`, `BufReader::new` or
`.lines()`; resolved calls catch the aliased forms. `bounded_io.rs` must
define a `MAX_MANIFEST_BYTES` cap constant (256 KiB).

**Limits.** The rule bounds one read, not the number of reads. An adapter
that calls `read_manifest` once per file in a large tree is within this
guardrail and outside the cost budget; that is what the per-adapter
manifest-cap test and the benchmark in
`.oh/sessions/2026-09-21-build-adapters-node-jvm.md` are for.

## Runtime tests that complete it

- `build_adapters/bounded_io.rs` unit tests (cap, counting, missing file)
- `build_adapters/{cargo,node,gradle,maven}.rs::identification_reads_no_more_than_manifest_cap`
