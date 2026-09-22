//! The mutation corpus (`GUARDRAILS_SPEC.md` section 17, item 6).
//!
//! A background mutation sweep over the 2026-09-22 re-review bypassed
//! **all 42** audits with compiling, harmful mutations on the first try
//! (`review/AUDIT-MUTATION-SWEEP.md`). Synthetic pass/fail fixtures
//! written by the same person who wrote the audit are not enough: they
//! test the shape the author already had in mind. So every slip the
//! sweep found becomes a fixture here, applied to a copy of the **real**
//! workspace, and the audit that let it through has to reject it.
//!
//! A fixture is a Rust file under
//! `tests/mutations/<audit-name>/<n>-<slug>.rs` with a header:
//!
//! ```text
//! //! target: crates/core/src/actions.rs
//! //! mode: append            # append (default) | replace
//! //! expect: reject          # reject (default) | accept
//! //! why: one line naming the sweep slip this reproduces
//! ```
//!
//! `append` adds the fixture body to the end of the real target file, so
//! the mutation lives beside real code and the audit has to find it
//! among everything else -- which is the property the synthetic
//! two-file fixtures never had. `replace` overwrites the target.
//!
//! The whole workspace is copied once; each fixture is applied, the
//! named audit is run, and the target file is restored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use swamp_source_audit::audits::AUDITS;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Directories an audit may read. `target/` and `.git/` are excluded:
/// the copy is for reading source, not for building.
const COPIED: &[&str] = &["crates", ".oh", "docs", "scripts", "Cargo.toml"];

fn copy_tree(from: &Path, to: &Path) {
    if from.is_file() {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from, to).unwrap();
        return;
    }
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        copy_tree(&e.path(), &to.join(&name));
    }
}

struct Fixture {
    audit: String,
    name: String,
    target: String,
    mode: String,
    expect: String,
    why: String,
    body: String,
}

fn parse_fixture(audit: &str, path: &Path) -> Fixture {
    let text = std::fs::read_to_string(path).unwrap();
    let mut header: BTreeMap<String, String> = BTreeMap::new();
    let mut body = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("//! ")
            && let Some((k, v)) = rest.split_once(':')
            && ["target", "mode", "expect", "why"].contains(&k.trim())
            && body.is_empty()
        {
            header.insert(k.trim().to_string(), v.trim().to_string());
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    Fixture {
        audit: audit.to_string(),
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        target: header
            .get("target")
            .unwrap_or_else(|| panic!("{} has no `//! target:` header", path.display()))
            .clone(),
        mode: header
            .get("mode")
            .cloned()
            .unwrap_or_else(|| "append".to_string()),
        expect: header
            .get("expect")
            .cloned()
            .unwrap_or_else(|| "reject".to_string()),
        why: header.get("why").cloned().unwrap_or_default(),
        body,
    }
}

fn fixtures() -> Vec<Fixture> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mutations");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for audit_dir in rd.flatten() {
        if !audit_dir.path().is_dir() {
            continue;
        }
        let audit = audit_dir.file_name().to_string_lossy().into_owned();
        let mut files: Vec<PathBuf> = std::fs::read_dir(audit_dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rs"))
            .collect();
        files.sort();
        for f in files {
            out.push(parse_fixture(&audit, &f));
        }
    }
    out.sort_by(|a, b| (&a.audit, &a.name).cmp(&(&b.audit, &b.name)));
    out
}

/// Every fixture is applied to a copy of the real workspace and the
/// named audit is run against it. A `reject` fixture that passes is
/// reported with the sweep slip it reproduces, so the failure names the
/// hole rather than a fixture number.
#[test]
fn every_mutation_fixture_is_rejected_by_its_audit() {
    let fixtures = fixtures();
    assert!(
        !fixtures.is_empty(),
        "the mutation corpus is empty; every audit needs at least three rejection fixtures \
         including one alias/rename and one discarded-result variant"
    );
    let tmp = tempfile::tempdir().expect("tempdir");
    let work = tmp.path().join("workspace");
    for entry in COPIED {
        let from = repo_root().join(entry);
        if from.exists() {
            copy_tree(&from, &work.join(entry));
        }
    }

    let mut problems: Vec<String> = Vec::new();
    for f in &fixtures {
        let Some((_, audit)) = AUDITS.iter().find(|(n, _)| *n == f.audit) else {
            problems.push(format!(
                "{}/{}: no audit named `{}` is registered",
                f.audit, f.name, f.audit
            ));
            continue;
        };
        let target = work.join(&f.target);
        let original = std::fs::read_to_string(&target).unwrap_or_default();
        let mutated = match f.mode.as_str() {
            "replace" => f.body.clone(),
            _ => format!("{original}\n{}", f.body),
        };
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, &mutated).unwrap();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audit(&work)));

        if original.is_empty() {
            let _ = std::fs::remove_file(&target);
        } else {
            std::fs::write(&target, &original).unwrap();
        }

        match outcome {
            Err(_) => problems.push(format!(
                "{}/{}: the audit panicked on the mutated tree",
                f.audit, f.name
            )),
            Ok(Ok(())) if f.expect == "reject" => problems.push(format!(
                "{}/{}: ACCEPTED a mutation it must reject -- {}\n    (applied to {})",
                f.audit, f.name, f.why, f.target
            )),
            Ok(Err(e)) if f.expect == "accept" => problems.push(format!(
                "{}/{}: REJECTED a legitimate shape -- {}\n    audit said: {e}",
                f.audit, f.name, f.why
            )),
            Ok(_) => {}
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} mutation fixtures behaved wrongly:\n{}",
        problems.len(),
        fixtures.len(),
        problems.join("\n")
    );
}

/// The unmutated copy must pass every audit, or a "rejected" result
/// above would prove nothing.
#[test]
fn the_unmutated_workspace_copy_reproduces_the_real_audit_result() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let work = tmp.path().join("workspace");
    for entry in COPIED {
        let from = repo_root().join(entry);
        if from.exists() {
            copy_tree(&from, &work.join(entry));
        }
    }
    for (name, audit) in AUDITS {
        let on_copy = audit(&work);
        let on_real = audit(&repo_root());
        assert_eq!(
            on_copy.is_ok(),
            on_real.is_ok(),
            "audit `{name}` disagrees between the real tree and its copy: copy={on_copy:?} \
             real={on_real:?}"
        );
    }
}

/// Section 17, item 6: an audit with no rejection fixture is an audit
/// nobody has shown rejects anything. The sweep's own result is the
/// argument -- all 42 passed their own tests and all 42 slipped.
///
/// Porting every audit is not one session's work, so this is a
/// **ratchet**, not a wish: `NOT_YET_IN_THE_CORPUS` lists the audits
/// still to port, and the test fails in both directions. An audit
/// removed from the list without fixtures fails; an audit that gains
/// fixtures while still listed also fails, so the list can only shrink.
/// It is checked in beside the fixtures rather than described in a
/// session note, because a gap nothing executes is a gap nobody closes.
#[test]
fn every_audit_has_rejection_fixtures_or_is_on_the_shrinking_list() {
    let fixtures = fixtures();
    let uncovered_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mutations/NOT_YET_IN_THE_CORPUS");
    let listed: Vec<String> = std::fs::read_to_string(&uncovered_path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();

    let mut problems: Vec<String> = Vec::new();
    for name in &listed {
        if !AUDITS.iter().any(|(n, _)| n == name) {
            problems.push(format!(
                "{name} is on the to-port list but is not a registered audit"
            ));
        }
    }
    for (name, _) in AUDITS {
        let n = fixtures
            .iter()
            .filter(|f| f.audit == *name && f.expect == "reject")
            .count();
        let on_list = listed.iter().any(|l| l == name);
        if n >= 3 && on_list {
            problems.push(format!(
                "{name} now has {n} rejection fixtures: remove it from \
                 tests/mutations/NOT_YET_IN_THE_CORPUS"
            ));
        }
        if n < 3 && !on_list {
            problems.push(format!(
                "{name} has {n}/3 rejection fixtures and is not on the to-port list: add three \
                 (one alias/rename, one discarded-result, one audit-specific) or add the audit to \
                 tests/mutations/NOT_YET_IN_THE_CORPUS with a reason"
            ));
        }
    }
    problems.sort();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
