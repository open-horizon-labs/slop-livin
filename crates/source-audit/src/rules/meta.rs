//! Guardrail and ADR metadata, checked as data.
//!
//! Re-review 3: a new `severity: hard` guardrail with no `audit:` field
//! passed, because the rule only checked that a *present* `audit:`
//! resolved. That is how `coverage-changes-are-not-storage-changes.md`
//! went unwatched into re-review 2's CE4. Now:
//!
//! * a hard guardrail names a registered audit, or says `audit: none`
//!   with a dated `audit_none_reason:` and the `runtime_tests:` that
//!   watch it instead, each of which must exist;
//! * a guardrail's `## Detection` section names at least one mutation
//!   corpus fixture of its audit (`<audit>/<fixture>`), so the prose says
//!   which rejected shape the claim rests on;
//! * an ADR's `audits:` resolve and its `cargo_tests:` are parsed
//!   `#[test]` functions, not text a comment could satisfy.

use super::verdict;
use crate::audits::AUDITS;
use quote::ToTokens;
use std::collections::HashSet;
use std::path::Path;
use syn::visit::Visit;

fn frontmatter(text: &str) -> Vec<String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    lines.take_while(|l| l.trim() != "---").map(str::to_string).collect()
}

fn value(front: &[String], key: &str) -> Option<String> {
    front
        .iter()
        .find_map(|l| l.strip_prefix(&format!("{key}:")).map(|v| v.trim().trim_matches('"').to_string()))
        .filter(|v| !v.is_empty())
}

fn list(front: &[String], key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut on = false;
    for l in front {
        let t = l.trim();
        if t == format!("{key}:") {
            on = true;
            continue;
        }
        if on {
            match t.strip_prefix("- ") {
                Some(item) => out.push(item.trim().to_string()),
                None => on = false,
            }
        }
    }
    out
}

/// The text of the `## Detection` section.
fn detection(text: &str) -> String {
    let mut out = String::new();
    let mut on = false;
    for l in text.lines() {
        if l.starts_with("## ") {
            on = l.trim_start_matches('#').trim().to_ascii_lowercase().starts_with("detection");
            continue;
        }
        if on {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// `(name, ignored, asserts)` for every `#[test]` function in a file.
fn test_fns(file: &syn::File) -> Vec<(String, bool, bool)> {
    struct V(Vec<(String, bool, bool)>);
    impl<'ast> Visit<'ast> for V {
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            if f.attrs.iter().any(|a| a.path().is_ident("test")) {
                let body = f.block.to_token_stream().to_string();
                let asserts = ["assert", "panic !", "unwrap_err", "expect_err", "contract ::"].iter().any(|a| body.contains(a));
                self.0.push((f.sig.ident.to_string(), f.attrs.iter().any(|a| a.path().is_ident("ignore")), asserts));
            }
            syn::visit::visit_item_fn(self, f);
        }
    }
    let mut v = V(Vec::new());
    v.visit_file(file);
    v.0
}

/// Every running, asserting `#[test]` in the workspace's crates, sources
/// and integration tests alike.
fn all_tests(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(rd) = std::fs::read_dir(root.join("crates")) else {
        return out;
    };
    for krate in rd.flatten() {
        let name = krate.file_name().to_string_lossy().into_owned();
        for sub in ["src", "tests"] {
            for rel in crate::resolve::rust_files_recursive(root, &format!("crates/{name}/{sub}")) {
                let Ok(text) = std::fs::read_to_string(root.join(&rel)) else { continue };
                let Ok(ast) = crate::ast::parse_cached(&rel, &text) else { continue };
                for (n, ignored, asserts) in test_fns(&ast) {
                    if !ignored && asserts {
                        out.insert(n);
                    }
                }
            }
        }
    }
    out
}

/// Whether `rel` defines a running, asserting `#[test] fn name`.
pub fn integration_test_exists(root: &Path, rel: &str, name: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
        return false;
    };
    let Ok(ast) = crate::ast::parse_cached(rel, &text) else {
        return false;
    };
    test_fns(&ast).iter().any(|(n, ignored, asserts)| n == name && !ignored && *asserts)
}

/// Every fixture id (`<audit>/<stem>`) in the mutation corpus.
fn fixture_ids(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let dir = root.join("crates/source-audit/tests/mutations");
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    for a in rd.flatten() {
        if !a.path().is_dir() {
            continue;
        }
        let audit = a.file_name().to_string_lossy().into_owned();
        for f in std::fs::read_dir(a.path()).into_iter().flatten().flatten() {
            let n = f.file_name().to_string_lossy().into_owned();
            if let Some(stem) = n.strip_suffix(".rs") {
                out.insert(format!("{audit}/{stem}"));
            }
        }
    }
    out
}

fn dated(s: &str) -> bool {
    s.as_bytes().windows(10).any(|w| {
        w.iter().enumerate().all(|(i, b)| if i == 4 || i == 7 { *b == b'-' } else { b.is_ascii_digit() })
    })
}

pub fn adr_validation(root: &Path) -> Result<(), String> {
    let names: Vec<&str> = AUDITS.iter().map(|(n, _)| *n).collect();
    let mut problems = Vec::new();
    let tests = all_tests(root);
    let fixtures = fixture_ids(root);
    let gdir = root.join(".oh/guardrails");
    let mut seen = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&gdir).map_err(|e| format!("read .oh/guardrails: {e}"))?.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        if e.path().extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let rel = format!(".oh/guardrails/{}", e.file_name().to_string_lossy());
        let text = std::fs::read_to_string(e.path()).map_err(|e| e.to_string())?;
        seen += 1;
        let front = frontmatter(&text);
        let severity = value(&front, "severity").unwrap_or_default();
        match value(&front, "audit").as_deref() {
            None if severity == "hard" => problems.push(format!(
                "{rel}: `severity: hard` with no `audit:` -- a hard guardrail names the audit that \
                 watches it, or says `audit: none` with a dated reason and the runtime tests that \
                 watch it instead"
            )),
            None => {}
            Some("none") => {
                let reason = value(&front, "audit_none_reason").unwrap_or_default();
                let runtime = list(&front, "runtime_tests");
                if severity == "hard" && (!dated(&reason) || runtime.is_empty()) {
                    problems.push(format!(
                        "{rel}: `audit: none` on a hard guardrail needs a dated `audit_none_reason:` \
                         and a non-empty `runtime_tests:` list"
                    ));
                }
                for t in runtime {
                    // `path/to/test.rs` (a test file with at least one
                    // running, asserting test) or `path/to/test.rs::name`.
                    let (file, name) = match t.split_once(".rs::") {
                        Some((f, n)) => (format!("{f}.rs"), Some(n.to_string())),
                        None if t.ends_with(".rs") => (t.clone(), None),
                        None => (String::new(), Some(t.rsplit("::").next().unwrap_or(&t).to_string())),
                    };
                    let ok = match (&file, &name) {
                        (f, Some(n)) if !f.is_empty() => integration_test_exists(root, f, n),
                        (f, None) => std::fs::read_to_string(root.join(f)).ok().and_then(|text| crate::ast::parse_cached(f, &text).ok()).is_some_and(|ast| test_fns(&ast).iter().any(|(_, ig, asserts)| !ig && *asserts)),
                        (_, Some(n)) => tests.contains(n),
                    };
                    if !ok {
                        problems.push(format!("{rel}: runtime test `{t}` is not a running, asserting #[test]"));
                    }
                }
            }
            Some(a) => {
                if !names.contains(&a) {
                    problems.push(format!("{rel}: names audit `{a}`, which is not registered"));
                } else {
                    let det = detection(&text);
                    let named = fixtures.iter().any(|id| id.starts_with(&format!("{a}/")) && det.contains(id.as_str()));
                    if !named {
                        problems.push(format!(
                            "{rel}: its Detection section names no mutation-corpus fixture of \
                             `{a}` (write one as `{a}/<fixture>`)"
                        ));
                    }
                }
            }
        }
    }
    if seen == 0 {
        problems.push(".oh/guardrails is empty".into());
    }
    let adr_dir = root.join("docs/ADRs");
    for e in std::fs::read_dir(&adr_dir).map_err(|e| format!("read docs/ADRs: {e}"))?.flatten() {
        if e.path().extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).map_err(|e| e.to_string())?;
        let front = frontmatter(&text);
        let rel = format!("docs/ADRs/{}", e.file_name().to_string_lossy());
        for a in list(&front, "audits") {
            if !names.contains(&a.as_str()) {
                problems.push(format!("{rel}: names audit `{a}`, which is not registered"));
            }
        }
        for t in list(&front, "cargo_tests") {
            let fn_name = t.rsplit("::").next().unwrap_or(&t).to_string();
            if !tests.contains(&fn_name) {
                problems.push(format!("{rel}: names test `{t}`, which is not a running, asserting #[test]"));
            }
        }
    }
    verdict("guardrail and ADR metadata resolve to real audits, fixtures and tests", problems)
}
