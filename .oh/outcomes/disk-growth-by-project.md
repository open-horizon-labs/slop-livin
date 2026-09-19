---
id: disk-growth-by-project
status: active
mechanism: |-
  Developers running many coding agents keep free space workable without emergency scans.
  The tool answers "what grew on this disk, by project, and what do I do about it" from a
  persistent column store of observations, and lets a human or an agent act on the answer
  with the facts in front of them. The agent is the everyday operator; only a human
  authorizes anything destructive.
files:
- crates/core/src/walk.rs
- crates/core/src/growth.rs
- crates/core/src/fs_events.rs
- crates/core/src/bus/*
- crates/core/src/consumers/*
- crates/core/src/report.rs
- crates/mcp/src/main.rs
- crates/tui/src/*
- .oh/guardrails/*
---

# Disk growth by project, acted on with facts

Every technical constraint Muness specified for this tool is recorded as a hard guardrail
under `.oh/guardrails/` and enforced as an AST audit in `crates/source-audit` (`cargo run
-p swamp-source-audit -- --list` names them). A constraint that is not enforced by an
audit is not considered implemented.

## Guardrails
- fsevents-before-full-walk
- column-store-parquet-zstd
- reverse-delta-current-plus-deltas
- scheduled-refresh-launchagent
- folding-only-for-artifacts
- symlinks-never-followed
- incremental-walk-only-changed-subtrees
- walk-optimized-parallel-pool
- dir-mtime-int32-minutes
- agent-interface-facts-not-verdicts
- human-only-authorization
- event-bus-pluggable-consumers
- extractors-are-pluggable
- one-byte-formatter
- computed-but-not-delivered
