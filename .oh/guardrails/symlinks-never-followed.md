---
id: symlinks-never-followed
severity: hard
statement: "The walk never follows a symlink: every directory listing that descends checks is_symlink first and discards the entry."
outcome: disk-growth-by-project
audit: symlinks_never_followed
---

## Rationale
Following symlinks double-counts bytes and can loop. Symlinks are either optimized (counted once by inode) or discarded; they are never traversed.

## Detection
AST audit `symlinks_never_followed`, over `walk.rs` and `attribution.rs`:
1. no call to `fs::metadata` (follows links; `symlink_metadata` does not);
2. no `is_dir()` / `is_file()` / `exists()` on a path expression (`entry.path().is_dir()`, `x.join(y).exists()` stat through the link);
3. in every directory loop, a discard guard (an `if` naming `is_symlink` that ends in `continue`/`return`) is a top-level statement at or before the first statement that descends on `is_dir`.

Proven by `scripts/audit-mutants.sh`: replacing `symlink_metadata` with `metadata`, deleting the loop guard, deciding `is_dir` on `entry.path()`, and moving the descent above the guard each fail this audit.
