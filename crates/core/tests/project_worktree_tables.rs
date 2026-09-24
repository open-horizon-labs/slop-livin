//! R15 (tables 2-4 of the JSON-in-the-store decomposition):
//! `projects.parquet`, `worktrees.parquet` (+ `worktree_facts.parquet`)
//! and the extended current-artifact table are what `swamp report`
//! builds a `Report`'s `projects` tree from -- not the snapshot's
//! `report_json` cell.
//!
//! Adversarial claims, not a happy path:
//!
//! 1. After `observe_scope`, the three tables exist under the store.
//! 2. `report_scope_from_store` reads a project's/worktree's own scalars
//!    and an artifact's byte/age/ecosystem facts from the tables: a
//!    snapshot whose `report_json` has been tampered with (name, branch,
//!    signals, bytes, ecosystem all changed) still reports the values the
//!    tables hold. The tempting shortcut this fails is "keep
//!    deserializing `report_json` and merely also write the tables".
//! 3. Every text and JSON view renders byte-identically from the report
//!    `observe_scope` produced and from the one `report_scope_from_store`
//!    rebuilt.
//!
//! Disposable `tempfile` fixtures only.

use std::path::{Path, PathBuf};
use std::process::Command;
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::report::{self, ObservationParts, Report};
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

/// A Rust checkout (so the `target` artifact carries an ecosystem tag)
/// with a linked worktree, plus a Node checkout: several projects,
/// several worktree kinds, artifacts with and without an ecosystem.
fn make_fixture(src: &Path) {
    let rust = src.join("rust-app");
    std::fs::create_dir_all(&rust).unwrap();
    run_git(&rust, &["init", "-q", "-b", "main"]);
    std::fs::write(rust.join("Cargo.toml"), b"[package]\nname=\"a\"\n").unwrap();
    std::fs::write(rust.join(".gitignore"), b"target/\n").unwrap();
    run_git(&rust, &["add", "Cargo.toml", ".gitignore"]);
    run_git(&rust, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(rust.join("target/debug")).unwrap();
    std::fs::write(rust.join("target/debug/blob"), vec![b'r'; 8192]).unwrap();
    let linked = src.join("rust-app-feature");
    run_git(
        &rust,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );
    std::fs::create_dir_all(linked.join("target/release")).unwrap();
    std::fs::write(linked.join("target/release/blob"), vec![b'f'; 4096]).unwrap();

    let node = src.join("node-app");
    std::fs::create_dir_all(node.join("node_modules/lodash")).unwrap();
    run_git(&node, &["init", "-q", "-b", "main"]);
    std::fs::write(node.join("package.json"), b"{\"name\":\"n\"}").unwrap();
    std::fs::write(node.join(".gitignore"), b"node_modules/\n").unwrap();
    run_git(&node, &["add", "package.json", ".gitignore"]);
    run_git(&node, &["commit", "-q", "-m", "init"]);
    std::fs::write(node.join("node_modules/lodash/index.js"), vec![b'n'; 2048]).unwrap();
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

/// Every text and JSON view the CLI can render, concatenated with a
/// separator, so one comparison covers them all.
fn render_everything(r: &Report) -> String {
    use swamp_core::agent_json as aj;
    use swamp_core::render as rd;
    let filter = swamp_core::filter::Filter::default();
    let mut out = vec![
        rd::render_text(r),
        rd::render_overview_sorted(r, true, false, false, rd::OverviewSort::Growth, false),
        rd::render_types(r),
        rd::render_kinds(r),
        rd::render_worktrees(r, &filter),
        rd::render_view_builds(r, None),
        rd::render_view_deps(r, None),
        rd::render_view_docker(r, None),
        rd::render_view_reconciliation(r),
        rd::render_view_unowned(r),
    ];
    for p in &r.projects {
        out.push(rd::render_project(r, &p.name).unwrap_or_default());
        out.push(rd::render_project_tree(r, &p.name).unwrap_or_default());
        for wt in &p.worktrees {
            out.push(rd::render_worktree_signals(r, &wt.path).unwrap_or_default());
        }
    }
    // `to_json` serializes `series_by_key` (a `HashMap`) in iteration
    // order, which differs between two `Report` values with the same
    // content; parse and re-serialize through `serde_json::Value`'s
    // ordered map so only a real difference shows.
    let full: serde_json::Value = serde_json::from_str(&report::to_json(r).unwrap()).unwrap();
    out.push(serde_json::to_string_pretty(&full).unwrap());
    for view in [
        "types",
        "kinds",
        "builds",
        "deps",
        "unowned",
        "reconciliation",
    ] {
        out.push(serde_json::to_string_pretty(&aj::view_payload(r, view, None)).unwrap());
    }
    out.push(serde_json::to_string_pretty(&aj::list_projects_payload(r, None)).unwrap());
    out.push(serde_json::to_string_pretty(&aj::list_worktrees_payload(r, &filter, None)).unwrap());
    out.push(serde_json::to_string_pretty(&aj::what_grew_payload(r, None)).unwrap());
    out.push(serde_json::to_string_pretty(&aj::docker_objects_payload(r, false, None)).unwrap());
    out.join("\n=====\n")
}

#[test]
fn observe_writes_the_three_tables_and_every_view_renders_identically_from_them() {
    let fx = build();
    let observation = observe(&fx);
    assert!(
        observation.merged.projects.len() >= 2,
        "fixture must discover both checkouts: {:?}",
        observation
            .merged
            .projects
            .iter()
            .map(|p| &p.name)
            .collect::<Vec<_>>()
    );
    let worktrees: usize = observation
        .merged
        .projects
        .iter()
        .map(|p| p.worktrees.len())
        .sum();
    assert!(
        worktrees >= 3,
        "fixture must discover the linked worktree too"
    );
    assert!(
        observation
            .merged
            .projects
            .iter()
            .any(|p| p.worktrees.iter().any(|w| w
                .artifacts
                .iter()
                .any(|a| a.ecosystem.as_deref() == Some("rs")))),
        "fixture must produce an artifact with an ecosystem tag"
    );
    assert!(
        observation
            .merged
            .projects
            .iter()
            .any(|p| p.worktrees.iter().any(|w| !w.signals.is_empty())),
        "fixture must produce at least one worktree signal, or the child table is untested"
    );

    for table in [
        "projects.parquet",
        "worktrees.parquet",
        "worktree_facts.parquet",
    ] {
        assert!(
            fx.store.join(table).is_file(),
            "observe_scope must write {table} at the top of the store"
        );
    }

    // The listing the report asks for: printed so it lands in the log.
    let mut listing: Vec<String> = Vec::new();
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(format!(
                    "{:>8}  {}",
                    std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                    p.strip_prefix(base).unwrap().display()
                ));
            }
        }
    }
    walk(&fx.store, &fx.store, &mut listing);
    listing.sort();
    println!("store after observe:\n{}", listing.join("\n"));

    let snapshot = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");
    assert_eq!(
        serde_json::to_value(&observation.merged).unwrap(),
        serde_json::to_value(&snapshot.report).unwrap(),
        "the rebuilt Report must equal the observed one field for field"
    );
    assert_eq!(
        render_everything(&observation.merged),
        render_everything(&snapshot.report),
        "every text/JSON view must render byte-identically from the tables"
    );
}

#[test]
fn report_reads_projects_worktrees_and_artifact_facts_from_the_tables_not_the_snapshot_json() {
    let fx = build();
    let observation = observe(&fx);
    let key = report::scope_snapshot_key(&fx.scope);
    let expected = serde_json::to_value(&observation.merged).unwrap();

    // Tamper with every field the tables now own, in the snapshot's
    // JSON cell only. The tables are untouched.
    let mut tampered = report::snapshot_from_observation(&observation);
    let mut tampered_fields = 0;
    for p in &mut tampered.report.projects {
        p.name.push_str("-TAMPERED");
        p.ecosystems.push("tampered".into());
        p.remote = Some("git@tampered:x/y.git".into());
        tampered_fields += 3;
        for wt in &mut p.worktrees {
            wt.branch = Some("tampered-branch".into());
            wt.idle_secs = Some(424242);
            wt.signals.clear();
            wt.kind = swamp_core::report::WorktreeKind::Clone;
            tampered_fields += 4;
            for a in &mut wt.artifacts {
                a.bytes += 1;
                a.local_bytes += 1;
                a.mtime_max += 1;
                a.ecosystem = Some("tampered".into());
                a.dedup_stale = !a.dedup_stale;
                a.hardlinked = !a.hardlinked;
                a.regrowth_count += 7;
                tampered_fields += 7;
            }
        }
    }
    assert!(
        tampered_fields > 20,
        "the fixture must give the tamper something to change"
    );
    swamp_core::growth::write_report_snapshot(&fx.store, &key, &tampered).unwrap();
    assert_ne!(
        serde_json::to_value(&tampered.report).unwrap(),
        expected,
        "the tamper must actually change the snapshot"
    );

    let rebuilt = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");
    assert_eq!(
        serde_json::to_value(&rebuilt.report).unwrap(),
        expected,
        "names, ecosystems, remotes, branches, kinds, idle time, signals and artifact \
         bytes/age/ecosystem/hardlinked/dedup/regrowth must come from the tables, not \
         from report_json"
    );
}

/// The snapshot's remaining parts (everything CHUNK_R15/R17 left for
/// later slices) still come from the snapshot: a tampered `unowned` row
/// is reported as tampered. This pins the boundary in both directions,
/// so the previous test cannot pass by ignoring the snapshot entirely.
///
/// `notes`/`reconciliation` moved to `notes.parquet`/`summary.parquet`
/// in R17 (`coverage_series_summary_notes_tables.rs` covers that
/// boundary now), so they are no longer part of this one -- tampering
/// them here would now be silently overwritten by the rebuild, which is
/// the *fixed* behavior this slice adds, not a regression.
#[test]
fn the_remaining_parts_still_come_from_the_snapshot() {
    let fx = build();
    let observation = observe(&fx);
    let key = report::scope_snapshot_key(&fx.scope);
    let mut tampered = report::snapshot_from_observation(&observation);
    tampered
        .report
        .unowned
        .push(swamp_core::report::UnownedRow {
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
        });
    swamp_core::growth::write_report_snapshot(&fx.store, &key, &tampered).unwrap();

    let rebuilt = report::report_scope_from_store(&fx.scope, &fx.store).expect("stored");
    assert!(
        rebuilt
            .report
            .unowned
            .iter()
            .any(|u| u.path_or_object == "tampered-unowned-path")
    );
    assert_eq!(
        serde_json::to_value(&rebuilt.report.projects).unwrap(),
        serde_json::to_value(&observation.merged.projects).unwrap()
    );
}
