# slop-livin

Agent-operated disk cleanup for developers who run many coding agents. Know what grew, by project, and what to do about it — delete, archive, prevent, or leave — with the agent proposing and you granting.

Status: first working core. The workspace contains the fact/index model, real
Parquet persistence, Git/Docker extractors, grants and sink rechecks, an
independent action ledger, CLI JSON, and MCP stdio surface. See
`docs/solution-space.md`, `docs/dissent.md`, `docs/discovery.md`, and the epic
at https://github.com/open-horizon-labs/slop-livin/issues/1.

Build and verify with `./scripts/check.sh`. The platform-specific FSEvents
adapter intentionally falls back to a typed full refresh on unsupported or
stale event histories.
