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

Every production literal in the three crates, every macro literal (with `concat!`/`format!` pieces joined) and every constant any production function names (followed through constants naming constants) must be free of the verdict vocabulary, after whole negating phrases are removed; snake_case identifiers are names, not prose. A macro the layer cannot read, in a string-producing function, fails.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_interface_facts_not_verdicts/01-concat-verdict`, `agent_interface_facts_not_verdicts/02-format-verdict`, `agent_interface_facts_not_verdicts/03-verdict-in-tui`, `agent_interface_facts_not_verdicts/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

