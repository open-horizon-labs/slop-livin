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

All eight build-adapter rules range over **derived** sets
(`crates/source-audit/src/build_audits.rs`, module doc): the governed
modules are every file under `crates/core/src/build_adapters/` --
`mod.rs`, `registry.rs`, `matrix.rs`, `jvm_common.rs` and `bounded_io.rs`
included, no file exempt by name -- plus any workspace file holding an
`impl BuildAdapter for ..`; adapters, their types and their ids are read
from those impls. Re-review 3 (`review/REVIEW-STACK-3.md` section 1)
found 31 of 43 audit slips were a hand-written list that did not contain
the thing; these rules keep no such list except the four bounded
primitives, each of which is itself checked to name its cap.

`bounded_io.rs` must define `MAX_MANIFEST_BYTES`; the bounded
primitives (`bounded_io::{read_manifest, read_whole_manifest}`,
`locations::{shallow_list, shallow_dir_names}`) must name their cap or
delegate to one that does, and are the graph's cut points. Every other
governed function fails if it reaches `fs::read_to_string`, `fs::read`,
`std::io::read_to_string`, `File::open`, anything in `OpenOptions`,
`BufReader::new`, `serde_json::from_reader`, or the methods
`read_to_end`/`read_to_string`/`read_line` -- including through a helper
elsewhere (`report::load_last_report`) and in `bounded_io.rs` itself
outside the two primitives.

**Limits.** The call graph is lexical: trait-object dispatch
(`adapter.identify(..)` through `dyn BuildAdapter`), function pointers
stored in a struct and closures passed across modules are not followed,
and a method call resolves only to the `impl`s of types the calling
function names in its signature or body. A primitive *named* anywhere in
a governed function (a function pointer, a type) is flagged even when it
is not called. Unknown macros in a governed module fail the rule.

## Runtime tests that complete it

- `build_adapters/bounded_io.rs` unit tests (cap, counting, missing file)
- `build_adapters/{cargo,node,gradle,maven}.rs::identification_reads_no_more_than_manifest_cap`
