---
id: agent-units-built-through-builder
severity: hard
statement: "Adapters build units through AgentUnitBuilder::new(tool, category, path), whose constructor applies the protected-by-default categories (credentials, configuration, skills, automation definitions, databases). Lifting a default protection requires an explicit reason."
outcome: decision-relevant-storage-evidence
audit: agent_units_built_through_builder
---

## Rationale

"Protect credentials/config/skills/automation definitions by default" is
a guardrail from the handoff, currently implemented as a `match` on
category inside `discover_and_measure` plus each adapter remembering to
set `protected: true` on the paths its own tool treats specially. A
struct literal with `protected: false` is one keystroke, reviews as
noise, and silently makes a credentials file actionable.

A constructor that starts protected and requires an argued exception
inverts the default: forgetting to think about it yields the safe answer.

## Detection

No adapter module may contain a `CandidateAgentUnit { .. }` or
`AgentUnit { .. }` struct literal; `agents/mod.rs` must define
`AgentUnitBuilder`.

**Limits.** The audit cannot check that the builder's default table is
*correct*; `protected_categories_default_protected` in each adapter does.

## Runtime tests that complete it

- every adapter's `protected_categories_default_protected`
- `crates/core/tests/agent_refusal_matrix.rs`
