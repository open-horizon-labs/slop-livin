---
date: 2026-09-22
outcome: disk-growth-by-project
issues: [69, 70, 71, 64, 65, 72]
---

# Build adapters: Python, Go, Apple, Android, BuildKit -- and the store join

## The new rule landed first, failing

One new rule is introduced by this chunk: machine-wide stores reach a
build adapter by declared capability (the detector's `build_stores()`, the
adapter's `store_kinds()`), never by a detector id, an adapter id or a
path shape (`.oh/guardrails/build-stores-join-by-capability.md`). It was
registered with eight rejection fixtures and one accept fixture before
the join existed. The repo-level run at that commit:

```
FAIL  build_stores_join_by_capability: no production function hands a machine-wide store to a
      build adapter: the adapters identify npm/pnpm stores, Gradle homes, Maven repositories, Go
      and Python caches, DerivedData and the Android SDK, and none of it reaches a live report
      until the external observation joins its measured stores to them by capability
1 audit(s) failed
```
