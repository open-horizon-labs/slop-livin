---
id: agent-interface-facts-not-verdicts
severity: hard
statement: "Every agent-facing and human-facing surface states facts with their terms; no verdict vocabulary (safe, stale, unused, can be deleted) appears in rendered text or tool output."
outcome: disk-growth-by-project
audit: agent_interface_facts_not_verdicts
---

## Rationale
Worktree staleness is not decidable. A verdict word is a claim the tool cannot back; a fact with terms is one the agent can reason from.

## Detection
No string literal in `render.rs`, `mcp/main.rs` or the TUI contains a verdict phrase. AST audit `agent_interface_facts_not_verdicts` (a `syn` literal visitor, not a grep, so identifiers and comments do not trip it).
