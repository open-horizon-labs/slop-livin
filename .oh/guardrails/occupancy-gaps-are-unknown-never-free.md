---
id: occupancy-gaps-are-unknown-never-free
severity: hard
statement: "Inside an occupancy probe a read that fails -- a process table, an fd directory, a link, a capture of a tool's output -- is a question that went unanswered, which is OccupancyState::Unknown. It is never skipped, defaulted or flattened into 'nothing holds it', except where the error says the thing read is gone (a process that exited)."
outcome: decision-relevant-storage-evidence
audit: occupancy_gaps_are_unknown_never_free
---

## Rationale

#86 replaced `lsof` on Linux with a direct procfs probe. procfs is exactly the kind of source that fails *quietly*: every process's `fd/`, `cwd`, `exe` and `maps` are separate reads, any of them can be denied (a non-dumpable process of this user, Yama, an LSM) or vanish (the process exited), and the idiomatic Rust for "read what you can" — `read_dir(..).flatten()`, `.ok()`, `if let Ok(..)` with no `else` — turns every denied read into "this process holds nothing". A probe written that way answers `Free` precisely when it could not look, and `Free` is the one answer that lets a removal proceed.

Writing the audit found a real instance in code that predates #86: the macOS `lsof` probe read its captured stdout with `let _ = f.read_to_string(&mut s)`, so a capture that could not be read back became "lsof printed nothing", i.e. `Free`. It is fixed in the same change.

## Detection

AST audit `occupancy_gaps_are_unknown_never_free` (`crates/source-audit/src/linux_audits.rs`):

- **The probes are derived**: every definition in `crates/core` whose return type is `OccupancyState`, plus what they call through exact edges (a helper one call away that reads `/proc` is part of the probe).
- **Fallible sources**: `std::fs` reads (resolved through aliases), their path-method forms (`p.read_dir()`, `f.read_to_string(..)`), and any definition in the probe set that returns `Result`; names bound from them (`let`, `for`, match arms) are tracked.
- Rejected on a fallible source: `let _ = ..`; the standard library's error-discarding adapters (`ok`, `err`, `flatten`, `unwrap_or*`, `map_or*`, `is_ok*`, `is_err*`, `filter_map`, `flat_map`); `if let Ok(..)` with no `else`; `let Ok(..) = .. else { <absence> }`; a `match` whose `Err`/`_` arm's outcome is absence (`continue`, `break`, `()`, `{}`, `Free`, `None`, `false`, `Ok(None)`, a default) **unless** its guard tests "gone" (`NotFound`/`ENOENT`/`ESRCH`, directly or in the one function the guard calls); and a read inside a macro argument, which the call graph cannot follow.

Nine fixtures in `crates/source-audit/tests/mutations/occupancy_gaps_are_unknown_never_free/`: `.flatten()` over the process listing, `.ok()` through `use std::fs::read_link as peek_link` (alias), an unguarded `Err(_) => continue`, a discarded read, a `let Ok .. else { return Free }`, a helper answering `is_ok()`, `if let Ok` without `else`, a read hidden in `format!`, and one accepted shape (a `NotFound`-guarded skip of an exited process).

**Limits.** Trait-object dispatch and function pointers are not edges. A read through a non-`std::fs` API (a raw `libc::read`) is not a recognised source. The "gone" guard is read one call deep. The probe's *scope* — processes of this user only — is a documented decision, not something this audit decides: another user's processes are not inspectable without privileges on either platform, and the evidence's coverage note says so.

## Runtime tests that complete it

- `crates/core/src/occupancy.rs::tests::procfs_*` — against fixture procfs trees: a descendant file, cwd or mapped file held; a deleted-but-open file; an unreadable process of this user is `Unknown` while another user's is not read; a foreign PID namespace and a missing `/proc` are `Unknown`; a process that exited mid-scan is skipped; the time bound.
- `crates/core/tests/linux_occupancy.rs` (Linux CI) — real processes holding a cwd and a descendant file, this process's own handle, process churn during the scan, no `lsof` needed, and the nested-PID-namespace limit recorded.
