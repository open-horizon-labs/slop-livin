---
id: agent-adapter-test-contract
severity: hard
statement: "Every adapter proves the same five things about itself, by these exact test names: unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess."
outcome: decision-relevant-storage-evidence
audit: agent_adapter_test_contract
---

## Rationale

The 2026-09-21 review's P1 on completion claims: the support matrix
labelled every tool "Supported" while the session notes admitted an
assumed Windsurf layout and unconfirmed Cline/Roo project fields. Test
*coverage* varied per adapter, so "Supported" meant different things in
different rows and nothing checked the difference.

Naming the five obligations makes the label mean one thing. An adapter
that cannot honestly write one of these tests is not Supported, and the
matrix has to say `Unverified`.

## Detection

The modules that declare a tool are derived (a `*_TOOL_ID` constant or an `Adapter` type under `agents/`/`build_adapters/`). Each must define the five contract tests as parsed `#[test]` functions inside a `#[cfg(test)]` module, not `#[ignore]`d, each asserting something (an `assert*!`, a `panic!`, an `unwrap_err`, or a `contract::` helper).

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapter_test_contract/01-required-test-ignored`, `agent_adapter_test_contract/02-required-test-missing`, `agent_adapter_test_contract/03-required-test-renamed`, `agent_adapter_test_contract/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- the five tests themselves, per adapter
- `crates/core/tests/agent_storage_validation.rs`
