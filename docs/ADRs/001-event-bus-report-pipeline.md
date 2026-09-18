---
id: 001-event-bus-report-pipeline
status: implemented
validate:
  cargo_tests:
    - bus::tests::follow_on_events_are_routed_to_subscribers
    - bus::tests::follow_on_events_dispatch_depth_first
    - bus::tests::consumer_receives_events_regardless_of_registration_order
    - bus::tests::registration_is_closed_once_run_starts
  audits:
    - no_consumer_knows_other_consumers
    - static_registration_only
    - all_report_paths_through_bus
    - extractors_are_pluggable
---

# The report pipeline is consumers on an in-memory event bus

**Guardrails:** event-bus-pluggable-consumers, extractors-are-pluggable
**Date:** 2026-09-18
**Reference:** repo-native-alignment `docs/ADRs/001-event-bus-extraction-pipeline.md`

## Context

The restart brief named the repo-native-alignment architecture: extractors per source, async
enrichers, an event bus with consumers, provenance and confidence on every fact. The pipeline
that shipped instead was `report_full_mode_with_source`, one function of roughly a thousand
lines calling discovery, attribution, grouping, git signals, GitHub, Docker, tracking,
ecosystems, growth, history and assembly in fixed order. Every enrichment added since landed as
another block in that function.

## Decision

Every stage is an independent `Consumer` that declares which `EventKind`s wake it and returns
follow-on events. The `EventBus` holds the registry and routes; it is the only coupling.

**Static registration, dynamic routing.** `EventBus::with_builtins()` registers every consumer
before the first event fires. There is no runtime registration and no conditional wiring.

**Runtime.** Consumers are `async fn on_event`, dispatched on a tokio current-thread runtime so
inherently async work (GitHub, Docker daemon, FSEvents replay) does not block the dispatch loop
and sync consumers cost nothing. Subscribers of one event run concurrently; follow-on events are
dispatched depth-first.

**Facts flow as events, not as mutation.** A consumer never receives `&mut Report`. It receives
the facts it subscribed to and emits new facts; the `AssemblyGate` waits for the set it needs and
emits the assembled rows; the `ReportAssembler` folds the final facts into `Report`.

## Event flow

```
RootRequested(ctx)
  → WalkConsumer                → RootObserved(discovered, attribution)        [FSEvents-first]
    → ProjectsConsumer          → ProjectsGrouped(projects, worktree_paths, remotes)
      → SignalsConsumer         → SignalsComputed(by_worktree)
        → GithubConsumer        → GithubEnriched(facts_by_worktree, summary, notes)
      → EcosystemConsumer       → EcosystemsDetected(by_project)
      → DockerConsumer          → DockerJoined(rows_by_worktree, unowned, bytes, notes)
      → AssemblyGate  (waits: Signals, Github, Ecosystems, Docker)
                                → RowsAssembled(projects, unowned, dirs, files, notes)
        → GrowthConsumer        → GrowthAnnotated(projects, dirs, files, schedule_line)   [store]
          → TrackingConsumer    → TrackingAnnotated(projects, dirs)
            → HistoryConsumer   → HistoryLoaded(series_by_key, total_series, window)
              → ReportAssembler → ReportAssembled(report)
                → CacheWriter   (last_report-<hash>.json)
```

## Adding a consumer

1. Implement `Consumer` in a new file under `crates/core/src/consumers/`.
2. Register it in `EventBus::with_builtins()`.

Nothing else changes. The audits below fail the build if a consumer imports another, registers
at runtime, or if `report.rs` calls a stage directly.

## Consequences

- `report_full_mode_with_source` becomes `bus::run_report(ctx)`; every existing golden and
  fixture test must produce a byte-identical `Report` across the change.
- Stage bodies move, unchanged, from `report.rs` into their consumer files.
- `tokio` and `async-trait` enter the core crate.
