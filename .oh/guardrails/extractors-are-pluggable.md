---
id: extractors-are-pluggable
severity: hard
statement: "A new fact source (a Docker extractor, a GitHub enricher, an ecosystem detector) is a consumer registered in EventBus::with_builtins; it never requires a change to the walk or to report assembly."
outcome: disk-growth-by-project
audit: extractors_are_pluggable
---

## Detection
Every `impl Consumer for X` under `consumers/` appears as `Box::new(X` in `with_builtins`, and nothing under `consumers/` is referenced from `walk.rs`. AST audit `extractors_are_pluggable`.
