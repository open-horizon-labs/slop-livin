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

/// One worker's private copy of the workspace, so fixtures can be
/// applied in parallel without two of them editing the same file.
fn fresh_copy(tmp: &Path) -> PathBuf {
    let work = tmp.join("workspace");
    for entry in COPIED {
        let from = repo_root().join(entry);
        if from.exists() {
            copy_tree(&from, &work.join(entry));
        }
    }
    work
}

/// Applies one fixture to `work`, runs its audit, restores the file, and
/// returns the problem it found, if any.
///
/// Restoring rather than re-copying is what makes a shared copy usable,
/// and it is also why a worker owns its copy outright: two fixtures
/// whose targets overlap would otherwise see each other's mutation.
fn check_fixture(work: &Path, f: &Fixture) -> Option<String> {
    let Some((_, audit)) = AUDITS.iter().find(|(n, _)| *n == f.audit) else {
        return Some(format!(
            "{}/{}: no audit named `{}` is registered",
            f.audit, f.name, f.audit
        ));
    };
    let target = work.join(&f.target);
    let original = std::fs::read_to_string(&target).unwrap_or_default();
    let mutated = match f.mode.as_str() {
        "replace" => f.body.clone(),
        _ => format!("{original}\n{}", f.body),
    };
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, &mutated).unwrap();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| audit(work)));

    if original.is_empty() {
        let _ = std::fs::remove_file(&target);
    } else {
        std::fs::write(&target, &original).unwrap();
    }

    match outcome {
        Err(_) => Some(format!(
            "{}/{}: the audit panicked on the mutated tree",
            f.audit, f.name
        )),
        Ok(Ok(())) if f.expect == "reject" => Some(format!(
            "{}/{}: ACCEPTED a mutation it must reject -- {}\n    (applied to {})",
            f.audit, f.name, f.why, f.target
        )),
        Ok(Err(e)) if f.expect == "accept" => Some(format!(
            "{}/{}: REJECTED a legitimate shape -- {}\n    audit said: {e}",
            f.audit, f.name, f.why
        )),
        Ok(_) => None,
    }
}

/// Every fixture is applied to a copy of the real workspace and the
/// named audit is run against it. A `reject` fixture that passes is
/// reported with the sweep slip it reproduces, so the failure names the
/// hole rather than a fixture number.
///
/// Sharded across workers, each with its own workspace copy and its own
/// thread-local parse cache (`ast::parse_cached`). Both halves matter:
/// the audits are pure functions of a directory, so an audit run costs
/// one full workspace analysis, and there are 136 of them. Together with
/// the parse cache this took the corpus from 191 s to the figure in
/// `.oh/sessions/2026-09-22-detector-root-cursors.md`. Neither changes
/// what any fixture asserts -- the sharding only decides which worker
/// runs which fixture, and the cache is keyed on the file's exact
/// contents (`the_parse_cache_never_changes_an_audit_verdict`).
#[test]
fn every_mutation_fixture_is_rejected_by_its_audit() {
    let fixtures = fixtures();
    assert!(
        !fixtures.is_empty(),
        "the mutation corpus is empty; every audit needs at least three rejection fixtures \
         including one alias/rename and one discarded-result variant"
    );
    // Each worker holds one workspace's parsed ASTs, so the worker count
    // is bounded by memory as much as by cores.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 12)
        .min(fixtures.len());
    let tmp = tempfile::tempdir().expect("tempdir");
    let shards: Vec<Vec<&Fixture>> = (0..workers)
        .map(|w| {
            fixtures
                .iter()
                .enumerate()
                .filter(|(i, _)| i % workers == w)
                .map(|(_, f)| f)
                .collect()
        })
        .collect();

    let problems: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = shards
            .into_iter()
            .enumerate()
            .map(|(w, shard)| {
                let dir = tmp.path().join(format!("w{w}"));
                scope.spawn(move || {
                    std::fs::create_dir_all(&dir).unwrap();
                    let work = fresh_copy(&dir);
                    shard
                        .into_iter()
                        .filter_map(|f| check_fixture(&work, f))
                        .collect::<Vec<String>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker"))
            .collect()
    });

    let mut problems = problems;
    problems.sort();
    assert!(
        problems.is_empty(),
        "{} of {} mutation fixtures behaved wrongly:\n{}",
        problems.len(),
        fixtures.len(),
        problems.join("\n")
    );
}

/// The parse cache is an optimisation, so it has to be invisible: the
/// same fixture must get the same verdict from a cold cache and a warm
/// one.
///
/// Three fixtures, cold-then-warm and warm-then-cold, because a cache
/// that only ever answers one way round is not being exercised. The
/// warm run is warmed by a *different* fixture's mutation of the same
/// file, which is the state the sharded run above actually creates.
#[test]
fn the_parse_cache_never_changes_an_audit_verdict() {
    let fixtures = fixtures();
    let tmp = tempfile::tempdir().expect("tempdir");
    let sample: Vec<&Fixture> = fixtures
        .iter()
        .step_by(fixtures.len().max(1) / 3)
        .take(3)
        .collect();
    assert_eq!(sample.len(), 3, "three fixtures, from across the corpus");

    std::thread::scope(|scope| {
        for (i, f) in sample.into_iter().enumerate() {
            let dir = tmp.path().join(format!("v{i}"));
            scope.spawn(move || check_cache_is_invisible(&dir, f));
        }
    });
}

/// One fixture's cold/warm comparison, on a workspace copy of its own.
fn check_cache_is_invisible(dir: &Path, f: &Fixture) {
    {
        let Some((_, audit)) = AUDITS.iter().find(|(n, _)| *n == f.audit) else {
            return;
        };
        std::fs::create_dir_all(dir).unwrap();
        let work = fresh_copy(dir);
        let target = work.join(&f.target);
        let original = std::fs::read_to_string(&target).unwrap_or_default();
        let mutated = match f.mode.as_str() {
            "replace" => f.body.clone(),
            _ => format!("{original}\n{}", f.body),
        };

        swamp_source_audit::ast::clear_parse_cache();
        std::fs::write(&target, &mutated).unwrap();
        let cold = audit(&work).is_ok();

        // Warm the cache with the *unmutated* file, then re-run the
        // mutated one: if the cache were keyed on anything weaker than
        // the contents -- a path, a length, a stamp -- this is where it
        // would answer for the wrong text.
        std::fs::write(&target, &original).unwrap();
        let _ = audit(&work);
        std::fs::write(&target, &mutated).unwrap();
        let warm = audit(&work).is_ok();

        std::fs::write(&target, &original).unwrap();
        let restored = audit(&work).is_ok();

        assert_eq!(
            cold, warm,
            "{}/{}: the parse cache changed the verdict (cold={cold}, warm={warm})",
            f.audit, f.name
        );
        assert!(
            restored,
            "{}/{}: the restored file must pass its own audit again, or the cache is holding \
             a mutated parse for unmutated text",
            f.audit, f.name
        );
    }
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
    // One audit per worker, since each audit reads the whole workspace
    // twice here and there are thirty of them. Read-only on both trees,
    // so no worker needs a copy of its own.
    let real = repo_root();
    let problems: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = AUDITS
            .iter()
            .map(|(name, audit)| {
                let work = work.clone();
                let real = real.clone();
                scope.spawn(move || {
                    let on_copy = audit(&work);
                    let on_real = audit(&real);
                    (on_copy.is_ok() != on_real.is_ok()).then(|| {
                        format!(
                            "audit `{name}` disagrees between the real tree and its copy: \
                             copy={on_copy:?} real={on_real:?}"
                        )
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().expect("worker"))
            .collect()
    });
    assert!(problems.is_empty(), "{}", problems.join("\n"));
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
