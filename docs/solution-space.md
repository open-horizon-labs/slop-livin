# Epic #1 solution-space selection

## Problem

Developers running many coding agents need an honest, project-aware account of
disk growth and a safe way to act on it without allowing an agent or a scanned
file to authorize destructive work.

## Candidates

| Option | Level | Trade-off |
| --- | --- | --- |
| A. Port Mole's inventory and deletion UI | Local optimum | Fastest path, but path-keyed decisions and human-only interaction remain. |
| B. Rust workspace with a fact graph, durable ledger, CLI JSON, and MCP stdio surface | Selected | More up-front modeling, but safety and agent operation share one auditable core. |
| C. Redesign around a database/daemon first | Redesign | Strong query capabilities, but adds deployment and authorization surface before the model is proven. |

## Decision

Select B. The core owns facts, entities, grants, plans, sink rechecks, and
ledger records. CLI and MCP are adapters. Docker remains fact-only; Trash is
the default removal sink; unknown and occupied states fail closed.

## Risk retirement

| Risk | Disposition | Check |
| --- | --- | --- |
| A local patch could call a sink without a live grant recheck | Retired by evidence | Execution tests mutate activity and reject stale plans; source audit rejects sink calls without rebind. |
| A fake extension could claim to be Parquet | Retired by evidence | Store tests open the file with the Parquet reader and check schema metadata. |
| Path identity could lose decisions after a rename | Retired by evidence | Git object-store identity and rename tests resolve the same project and ledger entries. |
| FSEvents/CoreServices behavior on non-macOS | Accepted with rationale | The core exposes a typed full-refresh fallback; platform-specific FFI is isolated and requires macOS verification. |
| Reference-machine byte attribution | Accepted with rationale | It requires access to the maintainer's machine; the report preserves named residuals rather than inventing values. |

## Invalidation and stop triggers

Reconsider the approach if the durable identity cannot be recovered from Git,
if sink re-observation is impossible, or if the reference workflow requires a
grant that the human did not explicitly create. Stop an individual operation
on stale facts, occupancy, missing evidence, budget overflow, or claimed units.
