# Dissent report: Epic #1 architecture

**Decision under review:** Build one Rust core around entities, facts, grants,
and a ledger, then expose it through CLI JSON and MCP.

**Stakes:** This tool can move or remove developer data. A wrong authorization
boundary is more serious than an incomplete feature.

## Steel-man position

The core model makes evidence rebuildable, decisions durable, and destructive
execution impossible without a human-created grant whose predicate is checked
again at the sink. Thin interfaces avoid duplicating safety logic.

## Contrary evidence and pre-mortem

1. **Functional failure:** an apparently durable store is only JSON with a
   `.parquet` suffix. The test must read a real Parquet footer/schema.
2. **Adoption failure:** fact-only candidates are too difficult for an agent to
   use. The MCP/CLI contract must be question-shaped and return typed stop
   states, while never emitting verdict words.
3. **Opportunity cost:** a full archive/Docker sink consumes time without
   evidence. Docker stays extraction-only and archive is explicit, higher-bar
   work until a real partial-failure observation exists.

## Hidden assumptions

| Assumption | Evidence | Risk | Test |
| --- | --- | --- | --- |
| Git object-store identity is stable across renames | Git repository identity | Non-git projects need a low-confidence fallback | Rename and fallback identity tests |
| Facts can be re-observed at execution | Filesystem and extractor APIs | Stale plans could delete changed content | Mutate-after-plan test |
| The human grant channel is outside scanned disk | CLI/MCP input | Repo-local instructions could self-authorize | Ignore `.swamprc` test |

## Reconstructed story

- **Still true:** evidence is not authorization; decisions must outlive the index.
- **Weakest assumption:** the local machine can provide enough fresh facts for
  a safe sink recheck.
- **Changed situation model:** the first release must make the safety boundary
  useful even when platform integrations are unavailable.
- **Changed belief:** high confidence in the core; medium confidence in the
  reference-machine measurements and CoreServices integration.
- **Next action:** implement and test the core contracts before platform adapters.

## Decision

**PROCEED.** The counter-arguments are addressed by real Parquet validation,
entity/ledger tests, and fail-closed execution. Human verification remains
required for macOS FSEvents and the reference-machine measurements.
