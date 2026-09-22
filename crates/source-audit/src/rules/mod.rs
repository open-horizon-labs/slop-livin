//! Every audit rule, written only against the program model
//! (`program.rs`).
//!
//! The 2026-09-22 re-review 3 sweep bypassed 43 of 45 audits
//! (`review/REVIEW-STACK-3.md`). The resolver had closed the alias class;
//! the rules still carried hand-written file lists, hand-written name
//! lists, checks that a name *appears* rather than that a behaviour
//! happens, and checks of one named function that never followed its
//! callees. The rules here are expressed through four primitives only:
//!
//! 1. **The whole program.** [`Program::load`] parses every file under
//!    `crates/{core,cli,tui}/src`. A rule never names a file to scan; it
//!    derives the region it governs from structure (adapters are the
//!    modules under `agents/`; consumers are the implementors of
//!    `bus::Consumer`; the report path is whatever calls `run_report`).
//! 2. **Derived sets.** Destructive, traversing, reading-unbounded,
//!    emitting and environment-reading functions are closures over the
//!    call graph of capability predicates on the standard library
//!    (`program::destructive_call` and friends), never lists of this
//!    workspace's function names.
//! 3. **Behaviour.** A required call must be *honoured* (its answer
//!    reaches control flow); a guard must be the *condition* of the write
//!    it guards; a required value must *flow* into the thing it protects.
//! 4. **Declared anchors.** What a guardrail is *about* -- the one
//!    protection predicate, the three rechecks, the bounded primitives
//!    and their caps -- is named once, resolved, and checked to exist, so
//!    a rename fails loudly instead of emptying the rule.

pub mod adapters;
pub mod bus;
pub mod evidence;
pub mod execution;
pub mod meta;
pub mod scope;
pub mod store;
pub mod tui;
pub mod walk;

use crate::program::{Fun, Program};
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

pub(crate) fn load(root: &Path) -> Rc<Program> {
    Program::load(root)
}

/// `Ok(())` when nothing was found; otherwise every problem, sorted and
/// de-duplicated, under a one-line statement of the rule.
pub(crate) fn verdict(rule: &str, mut problems: Vec<String>) -> Result<(), String> {
    if problems.is_empty() {
        return Ok(());
    }
    problems.sort();
    problems.dedup();
    Err(format!("{rule}:\n  {}", problems.join("\n  ")))
}

/// The definitions whose absolute path ends in each anchor; an anchor
/// that resolves to nothing is itself a failure, so renaming the thing a
/// guardrail is about can never quietly empty the rule.
pub(crate) fn anchors(p: &Program, paths: &[&str], problems: &mut Vec<String>) -> HashSet<usize> {
    let mut out = HashSet::new();
    for a in paths {
        let found = p.defs(a);
        if found.is_empty() {
            problems.push(format!(
                "`{a}` is not defined anywhere: the rule is about it, so it has to exist"
            ));
        }
        out.extend(found);
    }
    out
}

/// Whether a definition's body names the constant `cap` and stops on it
/// (`break`, a `truncated`/`Truncated` signal, or `.min(`/`.take(`):
/// what earns a bounded primitive its exemption.
pub(crate) fn is_bounded_by(f: &Fun, cap: &str) -> bool {
    crate::program::contains_token(&f.body, cap)
        && (f.body.contains("break")
            || f.body.contains("truncated")
            || f.body.contains("Truncated")
            || f.body.contains(". min (")
            || f.body.contains(". take ("))
}

/// Resolved paths a definition names, including its signature: every
/// `a :: b :: c` run in the token text.
pub(crate) fn named_paths(text: &str) -> Vec<String> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if is_ident(toks[i]) {
            let mut segs = vec![toks[i]];
            let mut j = i;
            while j + 2 < toks.len() && toks[j + 1] == "::" && is_ident(toks[j + 2]) {
                segs.push(toks[j + 2]);
                j += 2;
            }
            if segs.len() > 1 {
                out.push(segs.join("::"));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn is_ident(t: &str) -> bool {
    !t.is_empty()
        && t.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !t.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// The string a token-text expression evaluates to when it is a literal
/// or names a constant (see `Program::eval_literal`).
pub(crate) fn literal(p: &Program, expr: &str) -> Option<String> {
    p.eval_literal(expr)
}

/// Whether an expression is empty by construction: `""`, a constant
/// holding `""`, `String::new()`, `Default::default()`, `"".into()`.
pub(crate) fn empty_text(p: &Program, expr: &str) -> bool {
    let e = expr.replace(' ', "");
    if matches!(
        e.as_str(),
        "\"\"" | "String::new()" | "Default::default()" | "String::default()" | "\"\".into()"
            | "\"\".to_string()" | "\"\".to_owned()" | "&\"\"" | "Some(String::new())"
    ) {
        return true;
    }
    let base = e
        .trim_end_matches(".to_string()")
        .trim_end_matches(".into()")
        .trim_end_matches(".to_owned()");
    literal(p, base).is_some_and(|v| v.is_empty())
}
