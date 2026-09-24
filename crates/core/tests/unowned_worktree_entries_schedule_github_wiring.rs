//! R18a-3 wiring: `observe_scope` writes `unowned_summary.parquet` (+
//! `unowned_summary_lists.parquet`), `worktree_entries.parquet` and
//! `github_enrichment.parquet` from the same pass's already-assembled
//! `Report`, and `report_scope_from_store` rebuilds `Report.unowned`/
//! `.dirs_by_worktree`/`.files_by_worktree`/`.github_enrichment`/
//! `.schedule_line` from them.
//!
//! Field-exhaustive round-trip/tamper coverage for each table lives in
//! `growth::tests` (`unowned_summary_table_round_trips_every_field_and_
//! keyed_by_seq_not_path`, `worktree_entries_table_round_trips_every_
//! field_and_is_none_together_when_empty`,
//! `github_enrichment_table_round_trips_and_is_none_when_absent`,
//! `schedule_line_round_trips_through_summary_table`, and each one's
//! `..._rebuild_reflects_a_direct_tamper_not_the_original_value`
//! sibling); this file's job is proving the *wiring* through a real
//! `observe_scope` pass and `report_scope_from_store` read, the same
//! division `units_nested_evidence_tables.rs` uses for R16/R18a-2.
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

struct Fx {
    _tmp: tempfile::TempDir,
    store: PathBuf,
    scope: EffectiveScope,
}

fn build() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = swamp_core::fs_gate::canonicalize(tmp.path()).unwrap_or(tmp.path().to_path_buf());
    let src = root.join("src");
    let rust = src.join("rust-app");
    std::fs::create_dir_all(&rust).unwrap();
    run_git(&rust, &["init", "-q", "-b", "main"]);
    std::fs::write(rust.join("Cargo.toml"), b"[package]\nname=\"a\"\n").unwrap();
    std::fs::write(rust.join(".gitignore"), b"target/\n").unwrap();
    run_git(&rust, &["add", "Cargo.toml", ".gitignore"]);
    run_git(&rust, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(rust.join("target/debug")).unwrap();
    std::fs::write(rust.join("target/debug/blob"), vec![b'r'; 8192]).unwrap();

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
        // include_dirs: true -- populates dirs_by_worktree/files_by_worktree.
        true,
        false,
        false,
        swamp_core::fs_events::platform_source().as_ref(),
        30,
        24 * 3600,
    )
    .expect("observe_scope")
}

#[test]
fn observe_writes_the_four_tables_and_report_rebuilds_unowned_worktree_entries_and_github() {
    let fx = build();
    let observation = observe(&fx);
    assert!(
        observation.merged.dirs_by_worktree.is_some(),
        "include_dirs=true must populate dirs_by_worktree on the live observation"
    );

    for table in [
        "unowned_summary.parquet",
        "unowned_summary_lists.parquet",
        "worktree_entries.parquet",
        "github_enrichment.parquet",
    ] {
        assert!(
            fx.store.join(table).is_file(),
            "observe_scope must write {table} at the top of the store"
        );
    }

    let rebuilt = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");
    assert_eq!(
        serde_json::to_value(&rebuilt.report.unowned).unwrap(),
        serde_json::to_value(&observation.merged.unowned).unwrap(),
        "Report.unowned must rebuild field for field from unowned_summary.parquet"
    );
    assert_eq!(
        serde_json::to_value(&rebuilt.report.dirs_by_worktree).unwrap(),
        serde_json::to_value(&observation.merged.dirs_by_worktree).unwrap(),
        "Report.dirs_by_worktree must rebuild from worktree_entries.parquet"
    );
    assert_eq!(
        serde_json::to_value(&rebuilt.report.files_by_worktree).unwrap(),
        serde_json::to_value(&observation.merged.files_by_worktree).unwrap(),
        "Report.files_by_worktree must rebuild from worktree_entries.parquet"
    );
    assert_eq!(
        serde_json::to_value(&rebuilt.report.github_enrichment).unwrap(),
        serde_json::to_value(&observation.merged.github_enrichment).unwrap(),
        "Report.github_enrichment must rebuild from github_enrichment.parquet (None here: \
         this fixture never asked for live enrichment)"
    );
}

/// A direct on-disk tamper of `unowned_summary.parquet`/
/// `unowned_summary_lists.parquet` themselves (via the public
/// `write_unowned_summary_table` `observe_scope` uses) is what
/// `report_scope_from_store` reports for `Report.unowned` -- proof this
/// is a table read reachable through the real entry point, not just the
/// lower-level `growth::tests` unit coverage.
#[test]
fn report_reads_unowned_from_the_table_reflecting_a_direct_tamper() {
    let fx = build();
    let observation = observe(&fx);
    let key = report::scope_snapshot_key(&fx.scope);

    let tampered = vec![swamp_core::report::UnownedRow {
        path_or_object: "tampered-unowned-path".into(),
        bytes: 123,
        reason: swamp_core::report::UnownedReason::OwnedByNothing,
        shared_bytes: None,
        note: None,
        docker_kind: None,
        created_at: None,
        containers: Vec::new(),
        shared_with: Vec::new(),
        dangling: false,
        evidence: Vec::new(),
    }];
    assert_ne!(
        serde_json::to_value(&tampered).unwrap(),
        serde_json::to_value(&observation.merged.unowned).unwrap()
    );
    swamp_core::growth::write_unowned_summary_table(
        &fx.store,
        &key,
        &tampered,
        observation.merged.observed_at,
    )
    .unwrap();

    let rebuilt = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");
    assert_eq!(rebuilt.report.unowned.len(), 1);
    assert_eq!(
        rebuilt.report.unowned[0].path_or_object,
        "tampered-unowned-path"
    );
    assert_eq!(rebuilt.report.unowned[0].bytes, 123);
}
