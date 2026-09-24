//! R17 item 1 of the JSON-in-the-store decomposition:
//! `coverage.parquet`, `series.parquet`, `summary.parquet` and
//! `notes.parquet` are what `swamp report` builds a `Report`'s
//! `coverage`/`series_by_key`/`total_series`/`series_window_secs`/
//! `summary`/`reconciliation`/`notes` from -- not `report_rows.parquet`'s
//! `report_json` cell (R16 already moved `coverage_json`/
//! `external_units_json`/`agent_units_json`/`store_interiors_json` off
//! that row entirely; this slice removes the remaining duplication for
//! the fields named above).
//!
//! Adversarial claims, not a happy path:
//!
//! 1. After `observe_scope`, all four tables exist under the store.
//! 2. `report_scope_from_store` reads coverage/series/summary/
//!    reconciliation/notes from these tables: a snapshot whose
//!    `report_json` has been tampered with in every one of those fields
//!    still reports the observed values. The tempting shortcut this
//!    fails is "keep deserializing `report_json` and merely also write
//!    the tables".
//! 3. `report_scope_from_store` does this without a second discovery
//!    pass or any extra directory listing (`report_is_a_pure_read.rs`
//!    already pins the zero-syscalls contract; this only adds the new
//!    tables to what that pass reads).
//!
//! Disposable `tempfile` fixtures only.

use std::path::{Path, PathBuf};
use std::process::Command;
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::report::{self, ObservationParts};
use swamp_core::scope::{EffectiveScope, ScanConfig, resolve_effective_scope};

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()
        .unwrap_or_else(|e| panic!("run git {args:?}: {e}"));
    assert!(out.status.success(), "git {args:?} failed");
}

/// Two Rust checkouts (a Cargo home would need a real detector wiring,
/// which is not this slice's subject): enough for real `by_type`
/// summary rows, more than one project's worth of reconciliation totals,
/// and a coverage row per root.
fn make_fixture(src: &Path) {
    for name in ["app-a", "app-b"] {
        let dir = src.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("Cargo.toml"), b"[package]\nname=\"a\"\n").unwrap();
        std::fs::write(dir.join(".gitignore"), b"target/\n").unwrap();
        run_git(&dir, &["add", "Cargo.toml", ".gitignore"]);
        run_git(&dir, &["commit", "-q", "-m", "init"]);
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("target/debug/blob"), vec![b'r'; 8192]).unwrap();
    }
}

struct Fx {
    _tmp: tempfile::TempDir,
    store: PathBuf,
    scope: EffectiveScope,
}

fn observe(fx: &Fx) -> report::ScopeObservation {
    report::observe_scope(
        &fx.scope,
        ObservationParts::ALL,
        None,
        None,
        false,
        Some(&fx.store),
        None,
        true,
        true,
        false,
        false,
        swamp_core::fs_events::platform_source().as_ref(),
        30,
        24 * 3600,
    )
    .expect("observe_scope")
}

fn build() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = swamp_core::fs_gate::canonicalize(tmp.path()).unwrap_or(tmp.path().to_path_buf());
    let src = root.join("src");
    make_fixture(&src);
    let store = root.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let env = Environment::fixture(root.clone(), Default::default(), Platform::MacOS);
    let registry = Registry::with_builtins();
    let cfg = ScanConfig {
        defaults: false,
        include: vec![src.display().to_string()],
        disabled_detectors: registry
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .collect(),
        ..Default::default()
    };
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1_000);
    Fx {
        _tmp: tmp,
        store,
        scope,
    }
}

#[test]
fn observe_writes_the_four_tables_and_report_rebuilds_coverage_series_summary_reconciliation_and_notes()
 {
    let fx = build();
    let observation = observe(&fx);
    assert!(
        observation.merged.projects.len() >= 2,
        "fixture must discover both checkouts"
    );
    assert!(
        !observation.coverage.is_empty(),
        "fixture must produce at least one coverage row"
    );
    assert!(
        !observation.merged.summary.by_type.is_empty(),
        "fixture must produce a non-empty by_type summary"
    );
    assert!(
        observation.merged.reconciliation.walked_total > 0,
        "fixture must produce a non-zero walked_total"
    );

    for table in [
        "coverage.parquet",
        "series.parquet",
        "summary.parquet",
        "notes.parquet",
    ] {
        assert!(
            fx.store.join(table).exists(),
            "{table} must exist after observe_scope"
        );
    }

    let rebuilt = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");

    assert_eq!(
        serde_json::to_value(&rebuilt.coverage).unwrap(),
        serde_json::to_value(&observation.coverage).unwrap(),
        "coverage must round-trip through coverage.parquet"
    );
    assert_eq!(
        rebuilt.report.summary.projects,
        observation.merged.summary.projects
    );
    assert_eq!(
        rebuilt.report.summary.worktrees,
        observation.merged.summary.worktrees
    );
    assert_eq!(
        rebuilt.report.summary.artifacts,
        observation.merged.summary.artifacts
    );
    assert_eq!(
        rebuilt.report.summary.by_type.keys().collect::<Vec<_>>(),
        observation
            .merged
            .summary
            .by_type
            .keys()
            .collect::<Vec<_>>(),
        "by_type tags must round-trip"
    );
    for (tag, expected) in &observation.merged.summary.by_type {
        let got = &rebuilt.report.summary.by_type[tag];
        assert_eq!(got.bytes, expected.bytes, "{tag} bytes");
        assert_eq!(got.artifacts, expected.artifacts, "{tag} artifacts");
        assert_eq!(
            got.growth_bytes, expected.growth_bytes,
            "{tag} growth_bytes"
        );
    }
    assert_eq!(
        serde_json::to_value(&rebuilt.report.reconciliation).unwrap(),
        serde_json::to_value(&observation.merged.reconciliation).unwrap(),
        "reconciliation must round-trip through summary.parquet"
    );
    assert_eq!(rebuilt.report.notes, observation.merged.notes);
    // Series content depends on retention history existing yet, which a
    // single fresh observation may not have -- the window/shape must
    // still round-trip regardless.
    assert_eq!(
        rebuilt.report.series_window_secs,
        observation.merged.series_window_secs
    );
    assert_eq!(
        rebuilt.report.total_series.len(),
        observation.merged.total_series.len()
    );
    assert_eq!(
        rebuilt.report.series_by_key.len(),
        observation.merged.series_by_key.len()
    );
}

/// A snapshot whose `report_json` cell has been tampered with in every
/// coverage/series/summary/reconciliation/notes field still reports the
/// values `coverage.parquet`/`series.parquet`/`summary.parquet`/
/// `notes.parquet` hold, not the tampered JSON. Fails a rebuild that
/// merely writes the new tables without also using them to replace what
/// `report_json` said.
#[test]
fn report_reads_coverage_series_summary_and_notes_from_the_tables_not_the_snapshot_json() {
    let fx = build();
    let observation = observe(&fx);
    let key = report::scope_snapshot_key(&fx.scope);

    let mut tampered = report::snapshot_from_observation(&observation);
    tampered.report.notes = vec!["TAMPERED-NOTE".to_string()];
    tampered.report.summary.projects = 999_999;
    tampered.report.summary.worktrees = 999_999;
    tampered.report.summary.artifacts = 999_999;
    for t in tampered.report.summary.by_type.values_mut() {
        t.bytes += 1;
        t.artifacts += 1;
        t.growth_bytes = Some(t.growth_bytes.unwrap_or(0) + 1);
    }
    tampered.report.reconciliation.attributed += 1;
    tampered.report.reconciliation.unowned += 1;
    tampered.report.reconciliation.walked_total += 1;
    tampered.report.reconciliation.docker_attributed += 1;
    tampered.report.reconciliation.docker_unowned += 1;
    tampered.report.reconciliation.du_total = Some(
        tampered
            .report
            .reconciliation
            .du_total
            .map(|v| v + 1)
            .unwrap_or(1),
    );
    tampered.report.series_window_secs += 1;
    tampered.report.total_series = vec![Some(999_999)];
    tampered.coverage = Vec::new();
    swamp_core::growth::write_report_snapshot(&fx.store, &key, &tampered).unwrap();
    assert_ne!(
        serde_json::to_value(&tampered.report).unwrap(),
        serde_json::to_value(&observation.merged).unwrap(),
        "the tamper must actually change the snapshot"
    );

    let rebuilt = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");
    assert_eq!(rebuilt.report.notes, observation.merged.notes);
    assert_eq!(
        rebuilt.report.summary.projects,
        observation.merged.summary.projects
    );
    assert_eq!(
        serde_json::to_value(&rebuilt.report.reconciliation).unwrap(),
        serde_json::to_value(&observation.merged.reconciliation).unwrap()
    );
    assert_eq!(
        rebuilt.report.series_window_secs,
        observation.merged.series_window_secs
    );
    assert_eq!(
        serde_json::to_value(&rebuilt.coverage).unwrap(),
        serde_json::to_value(&observation.coverage).unwrap(),
        "coverage must come back from coverage.parquet even though the snapshot's own coverage list was emptied"
    );
}
