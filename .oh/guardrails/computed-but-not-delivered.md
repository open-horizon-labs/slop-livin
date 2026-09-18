---
id: computed-but-not-delivered
severity: soft
statement: "A fact is done only when it is wired from extraction through the schema to rendering and seen in real output; a populated struct field nobody renders is a defect."
outcome: disk-growth-by-project
audit: none
---

## Rationale
Carried from repo-native-alignment. Mole's `RepoRootID` was declared, computed and never rendered for months.

## Detection
Human review plus the frame goldens (`crates/tui/tests/frames`) and render snapshots, which render real fixture reports; a new report field lands with a golden that shows it.
