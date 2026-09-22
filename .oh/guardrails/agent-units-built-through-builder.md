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

No adapter function builds a `CandidateAgentUnit`/`AgentUnit` struct literal (resolved path; literals inside macro arguments count), writes a field the builder decides from the category (derived: the fields `AgentUnitBuilder` initializes from values it derives from `category`), or calls `unprotect_with_reason` with an argument that evaluates empty.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_units_built_through_builder/01-aliased-struct-literal`, `agent_units_built_through_builder/02-plain-struct-literal`, `agent_units_built_through_builder/03-unprotect-without-reason`, `agent_units_built_through_builder/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- every adapter's `protected_categories_default_protected`
- `crates/core/tests/agent_refusal_matrix.rs`
