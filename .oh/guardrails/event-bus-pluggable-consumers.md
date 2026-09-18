---
id: event-bus-pluggable-consumers
severity: hard
statement: "The report pipeline is consumers on an in-memory event bus (tokio runtime, static registration, dynamic routing): no consumer imports another, no consumer registers consumers at runtime, and every report is assembled by EventBus::run."
outcome: disk-growth-by-project
audit: event_bus_pluggable_consumers
---

## Rationale
This is the repo-native-alignment architecture the tool was specified to follow, and it was not built the first time: `report_full_mode_with_source` grew into a thousand-line hardwired stage chain and each new enrichment (git signals, GitHub, Docker, ecosystems, tracking) was bolted into it. The bus is the only coupling between stages, so a new fact source is one file that registers itself.

## Detection
Three AST audits: `no_consumer_knows_other_consumers` (no file under `consumers/` names another consumer or the bus), `static_registration_only` (no `register` call inside any `on_event`), `all_report_paths_through_bus` (`report.rs` calls no stage function directly; the stage entry points are called only under `consumers/`). ADR 001.
