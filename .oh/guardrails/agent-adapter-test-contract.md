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

Each adapter module's text must contain a `fn <name>(` for all five
names. The failure lists the adapter and the missing names.

**Limits.** Presence by name, not quality. A test that asserts nothing
passes this audit; that is what review is for. The names are chosen so a
hollow one is obvious.

## Runtime tests that complete it

- the five tests themselves, per adapter
- `crates/core/tests/agent_storage_validation.rs`
