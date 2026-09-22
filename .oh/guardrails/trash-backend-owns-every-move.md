---
id: trash-backend-owns-every-move
severity: hard
statement: "Every move of user data into a Trash goes through platform::trash (move_item / Envelope): a rename or a refusal, never a copy, never a permanent-deletion fallback, with the restore location (and on Linux the .trashinfo record) returned to the ledger."
outcome: disk-growth-by-project
audit: trash_backend_owns_every_move
---

## Rationale

Before #85 five places renamed a unit into a trash directory themselves: the CLI's `execute` (three branches), Cargo's grouped removal, the agent-storage cache and session removals, and the TUI's confirm. Each had its own naming, its own device check (or none), and none wrote the freedesktop `.trashinfo` a Linux file manager restores from. A Linux build therefore moved build directories into `~/.local/share/Trash` itself — not under `files/`, with no record — so no desktop could find or restore them.

The obvious fix, `trash::delete` from the `trash` crate, was read before it was adopted, and it does the one thing this guardrail forbids: on `EXDEV` its `move_items_no_replace` **copies the tree and then `remove_dir_all`s the source**, and a per-volume trash that is not writable falls back to the home trash across devices — the same copy. A copy of a build directory splits its hardlinks and expands its sparse files, so it can need more space than the disk has, and it is not atomic. It also returns `()`, so the ledger could not say where anything went. So the backend is swamp's own, and the crate is the independent *reader* in the Linux tests (`linux_trash.rs` lists and restores swamp's items through `trash::os_limited`).

## Detection

AST audit `trash_backend_owns_every_move` (`crates/source-audit/src/linux_audits.rs`), derived-set form (`review/REVIEW-STACK-3.md` §1):

- **The backend is found by module path** (`platform::trash`), not a file name.
- **The destructive sinks are derived**: every production definition that calls something returning `OccupancyState` (the tri-state gate every destructive action consults), excluding the gate itself. Their transitive callees (every candidate edge) are the region the rule holds.
- **Outside the backend, in that region, no rename moves existing data.** A rename is allowed there only when it publishes a file the same function created earlier (`fs::write` / `File::create` of the rename's source) — the atomic write-then-rename every control file uses. Anything else is a move that bypassed the backend.
- **Inside the backend, `std::fs` is inverted**: only reads, `create_dir(_all)`, `set_permissions`, `rename`, `remove_file` and `OpenOptions` are allowed; `copy`, `remove_dir_all`, `remove_dir`, `hard_link`, `write` (anything unlisted) fail closed, as does `std::io::copy`.
- **A move's result is honoured** — `let _ = rename_no_replace(..)` fails — and **a function that moves a path never removes that same path** (the permanent-deletion fallback).

`execution_sinks_recheck_live_state` complements it: a call into a backend definition that moves or removes is treated as a destructive call *at the caller*, which must carry the full honoured rechecks before it.

Seven fixtures in `crates/source-audit/tests/mutations/trash_backend_owns_every_move/`: a sink renaming into a trash directory itself, the same through `use std::fs::rename as shift` (alias), through a helper one call away, the backend copying + `remove_dir_all` across devices, a discarded move result, the source removed when the move fails, and one accepted shape (write-then-rename publish reachable from a sink).

**Limits.** Trait-object dispatch and function pointers are not edges in the call graph; a rename reached only that way is not seen. A rename whose source is a fresh file created by something other than `fs::write`/`File::create` (e.g. `tempfile`) outside the backend is treated as a move and would be reported — the fail-closed direction. `libc::rename*` is recognised by path; a raw `syscall(SYS_renameat2, ..)` is not.

## Runtime tests that complete it

- `crates/core/src/platform/trash.rs::tests` — directory target renames under the given name and never replaces; an envelope rolls back what it moved.
- `crates/core/tests/linux_trash.rs` (Linux CI) — recoverability and restore through an independent spec reader, original path and deletion time, name collisions, symlinked home trash and `files/`, an interrupted move leaving no orphan record, another mount's own trash, a symlinked `.Trash-$uid`, cross-device refusal with no partial copy, and the ledger recording the location and the `.trashinfo`.
