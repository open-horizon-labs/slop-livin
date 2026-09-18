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
Every function in `walk.rs` and `attribution.rs` that calls `read_dir` and descends (`is_dir`) also calls `is_symlink`. AST audit `symlinks_never_followed`.
