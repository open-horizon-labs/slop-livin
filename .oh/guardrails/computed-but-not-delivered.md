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

AST audit `computed_but_not_delivered`. For every `pub` struct field on a
delivered-surface type (`artifact.rs`, `external.rs`, `report.rs`,
`agents/mod.rs`) whose declared type mentions `Evidence`:

- every initializer that field is ever given, anywhere in
  `crates/{core,cli,tui}/src`, is collected; if all of them are empty
  (`Vec::new()`, `vec![]`, `Default::default()`, `None`) and no site ever
  assigns/extends/pushes to it, the audit fails naming the field; and
- if the field *is* computed but no delivery file (`render.rs`,
  `agent_json.rs`, `report.rs`, `cli/src/main.rs`) mentions it, the audit
  fails: a populated struct field nobody renders is a defect.

**Limits.** Scoped to evidence-typed fields. That is the surface the
#53-#59 matrix promises row by row and the surface two reviews found holes
in; it keeps the rule mechanical rather than guessing at every field in
the crate. A field a delivery file merely *mentions* counts as delivered,
so the frame goldens and render snapshots below are still what prove the
value reaches a human.

## Runtime tests that complete it

- `crates/core/tests/nested_artifact_evidence_is_delivered.rs` — a real
  nested build artifact carries decision evidence, and `--view rust` JSON
  shows it.
- The frame goldens (`crates/tui/tests/frames`) and render snapshots,
  which render real fixture reports.
