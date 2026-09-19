# Epic #1 discovery answers (#15)

Historical record from Epic #1 (2026-09-17). The project was reframed on
2026-09-18. Statements below describe decisions and expectations at that time,
including features that were deferred, replaced, or later implemented. For
current behavior, use the [usage](usage.md) and [architecture](architecture.md)
guides. These notes are not current guarantees or test results.

These are explicit boundaries for the first implementation, not hidden future
work. Each can become a follow-up when evidence justifies it.

1. **Generator entity:** `prevent` records a `generator` relationship on the
   artifact (tool name, command family, and restore source). The current core
   keeps the relationship optional until an extractor can observe it.
2. **Snapshots:** remain typed residual causes until a snapshot extractor can
   observe ownership and a safe sink. No snapshot deletion is exposed.
3. **Confidence decay:** facts carry observation time; consumers must apply a
   freshness window. Fresh low-confidence facts do not become high-confidence
   through age.
4. **Entity merge/split:** identity changes create a new entity and retain the
   old ledger entries; a future explicit merge record can connect them. No
   silent path-key migration is allowed.
5. **Authorization silence:** grants expire; the surface degrades to read-only
   and returns `awaiting-authorization`, never retries destructive work.
6. **Circuit breaker:** proposal and refused-unit retry limits belong to the
   agent surface, with `busy`/`claimed` stop states rather than looping.
7. **Shared caches:** represented as artifacts with no single project owner and
   a recovery contract; they are never attributed by name alone.
8. **How the agent notices:** the initial surface is poll-based (`changed_since`)
   with scheduled refresh. Push notification is deferred until a stable daemon
   lifecycle exists.
9. **Managed machines:** unsupported in this release; the report must say so
   rather than infer single-user ownership.
