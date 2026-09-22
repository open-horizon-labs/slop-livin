//! The runtime half of `.oh/guardrails/coverage-changes-are-not-storage-changes.md`,
//! which until 2026-09-22 had no executable validation at all (the
//! guardrail is `severity: hard` and carried no `audit:` field either).
//!
//! The re-review's CE4 is one instance: adding one `exclude` line to the
//! config reported 64 KB of growth and, on removal, a regrowth event,
//! with zero bytes changed on disk. This file is the family: for each way
//! coverage can change -- an exclusion added, a detector disabled, a
//! location made unreadable, the invocation switched to an explicit root
//! -- three passes over an *unchanged* tree must produce zero growth,
//! zero regrowth and zero tombstones.
//!
//! "Zero tombstones" is checked at the store, not inferred: pass 3
//! restores the original coverage and every unit must come back with the
//! byte total it had in pass 1 and `regrowth_count == 0`. A tombstone
//! written in pass 2 shows up here as a regrowth in pass 3, which is
//! exactly how the bug presented.
//!
//! Disposable `tempfile` fixtures only.

use std::{collections::HashMap, fs};
use swamp_core::{
    external,
    locations::{Environment, Platform, Registry},
    scope::{ScanConfig, resolve_effective_scope},
};

fn only(ids: &[&str]) -> ScanConfig {
    ScanConfig {
        defaults: false,
        disabled_detectors: Registry::with_builtins()
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .filter(|id| !ids.contains(&id.as_str()))
            .collect(),
        ..Default::default()
    }
}

/// A Cargo home with a nested registry cache: the parent and the child
/// are separate external units, and the child is inside the parent's
/// path-prefix ownership window. That overlap is what every case below
/// attacks.
struct Fixture {
    _tmp: tempfile::TempDir,
    outer: std::path::PathBuf,
    inner: std::path::PathBuf,
    env: Environment,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let outer = tmp.path().join("cargo");
    let inner = outer.join("registry/cache");
    fs::create_dir_all(&inner).unwrap();
    fs::write(outer.join("top.bin"), vec![b'x'; 4096]).unwrap();
    fs::write(inner.join("crate.crate"), vec![b'y'; 65536]).unwrap();
    let env = Environment::fixture(
        tmp.path().to_owned(),
        HashMap::from([("CARGO_HOME".into(), outer.display().to_string())]),
        Platform::MacOS,
    );
    Fixture {
        _tmp: tmp,
        outer,
        inner,
        env,
    }
}

type Snapshot = Vec<(std::path::PathBuf, u64, Option<i64>, u32)>;

fn observe(
    env: &Environment,
    cfg: &ScanConfig,
    explicit: &[std::path::PathBuf],
    store: &std::path::Path,
    at: u64,
) -> Snapshot {
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(env, cfg, explicit, &registry, at);
    let units = external::discover_and_measure(&scope, Some(store), true, at, 30, 3600).unwrap();
    let mut out: Snapshot = units
        .into_iter()
        .map(|u| (u.path, u.bytes, u.growth_bytes, u.regrowth_count))
        .collect();
    out.sort();
    out
}

fn assert_no_invented_history(label: &str, snap: &Snapshot) {
    let grew: Vec<_> = snap
        .iter()
        .filter(|(_, _, g, _)| g.unwrap_or(0) != 0)
        .collect();
    assert!(
        grew.is_empty(),
        "{label}: nothing changed on disk, yet growth was reported: {grew:?}"
    );
    let regrew: Vec<_> = snap.iter().filter(|(_, _, _, r)| *r > 0).collect();
    assert!(
        regrew.is_empty(),
        "{label}: nothing changed on disk, yet a regrowth was recorded: {regrew:?}"
    );
}

/// Pass 1 baseline, pass 2 with coverage changed, pass 3 with the
/// original coverage restored. No byte on disk moves.
fn three_passes(label: &str, narrowed: impl Fn(&Fixture) -> (ScanConfig, Vec<std::path::PathBuf>)) {
    let f = fixture();
    let store = tempfile::tempdir().unwrap();
    let base = only(&["cargo-home"]);

    let first = observe(&f.env, &base, &[], store.path(), 1_000);
    assert!(
        first.len() >= 2,
        "{label} precondition: the parent and the nested location are both measured: {first:?}"
    );

    let (narrow_cfg, narrow_roots) = narrowed(&f);
    let second = observe(&f.env, &narrow_cfg, &narrow_roots, store.path(), 2_000);
    assert_no_invented_history(&format!("{label} pass 2 (coverage narrowed)"), &second);

    let third = observe(&f.env, &base, &[], store.path(), 3_000);
    assert_no_invented_history(&format!("{label} pass 3 (coverage restored)"), &third);
    assert_eq!(
        first.iter().map(|(p, b, _, _)| (p, b)).collect::<Vec<_>>(),
        third.iter().map(|(p, b, _, _)| (p, b)).collect::<Vec<_>>(),
        "{label}: restoring coverage did not restore the same measured units"
    );
}

#[test]
fn adding_an_exclusion_is_not_growth_or_regrowth() {
    three_passes("exclusion", |f| {
        (
            ScanConfig {
                exclude: vec![f.inner.display().to_string()],
                ..only(&["cargo-home"])
            },
            vec![],
        )
    });
}

#[test]
fn disabling_a_detector_is_not_growth_or_regrowth() {
    three_passes("disabled detector", |_| {
        let registry = Registry::with_builtins();
        (
            ScanConfig {
                defaults: false,
                disabled_detectors: registry
                    .detectors()
                    .iter()
                    .map(|d| d.id().to_string())
                    .collect(),
                ..Default::default()
            },
            vec![],
        )
    });
}

#[test]
fn switching_to_an_explicit_root_that_excludes_a_location_is_not_growth_or_regrowth() {
    three_passes("explicit root + exclusion", |f| {
        (
            ScanConfig {
                exclude: vec![f.inner.display().to_string()],
                ..only(&["cargo-home"])
            },
            vec![f.outer.clone()],
        )
    });
}

/// Lost access is the case the guardrail names first: "unobserved is not
/// deleted". A location that cannot be read this pass keeps its last
/// known value and must never be tombstoned.
#[test]
fn losing_access_to_a_location_is_not_a_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let store = tempfile::tempdir().unwrap();
    let base = only(&["cargo-home"]);
    let first = observe(&f.env, &base, &[], store.path(), 1_000);
    assert!(first.len() >= 2, "precondition: {first:?}");

    fs::set_permissions(&f.inner, fs::Permissions::from_mode(0o000)).unwrap();
    let second = observe(&f.env, &base, &[], store.path(), 2_000);
    fs::set_permissions(&f.inner, fs::Permissions::from_mode(0o755)).unwrap();
    // Running as root defeats the permission bit; only assert where the
    // platform enforces it.
    let canonical_inner = fs::canonicalize(&f.inner).unwrap();
    let unreadable = second
        .iter()
        .find(|(p, _, _, _)| *p == canonical_inner)
        .is_some_and(|(_, b, _, _)| *b == 65536);
    if !unreadable {
        assert_no_invented_history("lost access pass 2", &second);
        let third = observe(&f.env, &base, &[], store.path(), 3_000);
        assert_no_invented_history("lost access pass 3 (access restored)", &third);
    }
}
