//! The type-level half of the guardrails: every shortcut a retired
//! semantic audit used to look for, written against swamp-core's
//! production API, must fail to compile with the expected error.
//!
//! The cases live in `crates/core/tests/compile_fail/` (each with its
//! expected `.stderr`; guardrail frontmatter names them under
//! `compile_fail:`), and run from this crate so they build against
//! swamp-core *without* its `testing` feature. Regenerate the expected
//! output after an intended API change with `TRYBUILD=overwrite`.

#[test]
#[ignore = "heavy harness: scripts/check-full.sh runs it once, with --ignored"]
fn retired_rule_shortcuts_do_not_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("../core/tests/compile_fail/*.rs");
}
