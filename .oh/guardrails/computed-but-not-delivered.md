---
id: computed-but-not-delivered
severity: soft
statement: "A fact is done only when it is wired from extraction through the schema to rendering and seen in real output; a populated struct field nobody renders is a defect."
outcome: disk-growth-by-project
audit: computed_but_not_delivered
---

## Rationale

Carried from repo-native-alignment. Mole's `RepoRootID` was declared,
computed and never rendered for months.

The 2026-09-22 independent re-review found the same shape here, and found
it *because* this was the one guardrail in the directory whose frontmatter
said `audit: none`: `CHANGELOG.md` claimed decision evidence was
"attached to ... nested build-artifact units", while
`NestedArtifact::decision_evidence` was written only as `Vec::new()`
(`cargo_artifacts.rs`, `cli/src/main.rs`),
`report::attach_decision_evidence` never touched `report.nested_artifacts`,
`skip_serializing_if` hid the empty vector from `--view rust` JSON, and no
test existed. An unwatched guardrail is the one the next overclaim
violates.

## Detection

Delivered surfaces are derived: the modules defining the types reachable, through field types, from what `bus::run_report` and `report::observe_scope` return, and their child modules; every public struct in one of them that is serialized or carries an evidence-vocabulary field. A public field whose every struct-literal initializer (including inside macros) is empty and that nothing mutates is written only as a default; a `skip_serializing_if` field must be read somewhere.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `computed_but_not_delivered/01-empty-default-only`, `computed_but_not_delivered/02-populated-but-hidden-and-unread`, `computed_but_not_delivered/03-aliased-evidence-type`, `computed_but_not_delivered/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/nested_artifact_evidence_is_delivered.rs` — a real
  nested build artifact carries decision evidence, and `--view rust` JSON
  shows it.
- The frame goldens (`crates/tui/tests/frames`) and render snapshots,
  which render real fixture reports.
