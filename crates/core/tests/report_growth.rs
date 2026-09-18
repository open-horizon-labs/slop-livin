//! R4: growth since previous observation, exercised end-to-end through
//! `report_with` against a temp store dir and the shared fixture.

#[path = "fixture/mod.rs"]
mod fixture;

use slop_livin_core::report::{ArtifactKind, report_with};
use std::fs;

#[test]
fn growing_one_artifact_shows_growth_there_and_zero_elsewhere() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    // First observation: persists current-state, no prior baseline.
    let first =
        report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("first report");
    let checkout = first
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .expect("checkout project");
    let main = checkout
        .worktrees
        .iter()
        .find(|w| w.path == fx.checkout)
        .expect("main worktree");
    for artifact in &main.artifacts {
        assert_eq!(
            artifact.growth_bytes, None,
            "first observation must have no growth baseline: {artifact:?}"
        );
    }

    // Grow node_modules by exactly 5 MiB.
    let grow_bytes = 5 * 1024 * 1024;
    fs::write(fx.node_modules.join("growth-probe"), vec![b'g'; grow_bytes])
        .expect("write growth probe");

    let second =
        report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("second report");
    let checkout2 = second
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .expect("checkout project");
    let main2 = checkout2
        .worktrees
        .iter()
        .find(|w| w.path == fx.checkout)
        .expect("main worktree");

    let node_modules_row = main2
        .artifacts
        .iter()
        .find(|a| a.path == fx.node_modules)
        .expect("node_modules row");
    assert_eq!(
        node_modules_row.growth_bytes,
        Some(grow_bytes as i64),
        "node_modules should show exactly the added bytes as growth"
    );

    for artifact in &main2.artifacts {
        if artifact.path == fx.node_modules {
            continue;
        }
        assert_eq!(
            artifact.growth_bytes,
            Some(0),
            "unrelated artifact must show zero growth: {artifact:?}"
        );
    }
}

#[test]
fn deleting_and_recreating_target_counts_one_regrowth() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("first report");

    fs::remove_dir_all(&fx.target_dir).expect("delete target/");
    let after_delete =
        report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("second report");
    let checkout = after_delete
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .expect("checkout project");
    let main = checkout
        .worktrees
        .iter()
        .find(|w| w.path == fx.checkout)
        .expect("main worktree");
    assert!(
        !main.artifacts.iter().any(|a| a.path == fx.target_dir),
        "deleted target/ must not appear as a report row (tombstones are store-internal)"
    );

    fs::create_dir_all(&fx.target_dir).expect("recreate target/");
    fs::write(fx.target_dir.join("rebuilt"), vec![b'x'; 4096]).expect("write rebuilt file");

    let after_recreate =
        report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("third report");
    let checkout3 = after_recreate
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .expect("checkout project");
    let main3 = checkout3
        .worktrees
        .iter()
        .find(|w| w.path == fx.checkout)
        .expect("main worktree");
    let target_row = main3
        .artifacts
        .iter()
        .find(|a| a.path == fx.target_dir && a.kind == ArtifactKind::BuildOutput)
        .expect("recreated target/ row");
    assert_eq!(
        target_row.regrowth_count, 1,
        "one absent -> present transition must count as exactly one regrowth"
    );
}

#[test]
fn observed_report_covers_every_worktree_and_leaves_unowned_rows_untouched() {
    // A second observation with no filesystem changes must show zero
    // growth everywhere -- including the linked worktree and the nested
    // repo's own build/ artifact -- and must not disturb the unowned
    // rows (shared cache, loose file), which the growth store never
    // tracks.
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("first report");
    let second =
        report_with(&fx.root, None, false, Some(store.path()), Some("1h")).expect("second report");

    let checkout = second
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .expect("checkout project");
    let main = checkout
        .worktrees
        .iter()
        .find(|w| w.path == fx.checkout)
        .expect("main worktree");
    let target_row = main
        .artifacts
        .iter()
        .find(|a| a.path == fx.target_dir)
        .expect("target row");
    assert_eq!(target_row.bytes, fx.target_bytes);
    assert_eq!(target_row.growth_bytes, Some(0));
    let dist_row = main
        .artifacts
        .iter()
        .find(|a| a.path == fx.dist_dir)
        .expect("dist row");
    assert_eq!(dist_row.bytes, fx.dist_bytes);
    assert_eq!(dist_row.growth_bytes, Some(0));
    let node_modules_row = main
        .artifacts
        .iter()
        .find(|a| a.path == fx.node_modules)
        .expect("node_modules row");
    assert_eq!(node_modules_row.bytes, fx.node_modules_bytes);

    assert!(
        checkout
            .worktrees
            .iter()
            .any(|w| w.path == fx.linked_worktree),
        "linked worktree row must still be present"
    );

    let nested_project = second
        .projects
        .iter()
        .find(|p| p.worktrees.iter().any(|w| w.path == fx.nested_repo))
        .expect("nested repo project row");
    let nested_worktree = nested_project
        .worktrees
        .iter()
        .find(|w| w.path == fx.nested_repo)
        .expect("nested repo worktree row");
    let nested_build_row = nested_worktree
        .artifacts
        .iter()
        .find(|a| a.path == fx.nested_repo_build)
        .expect("nested repo build row");
    assert_eq!(nested_build_row.bytes, fx.nested_repo_build_bytes);
    assert_eq!(nested_build_row.growth_bytes, Some(0));

    let shared_cache_row = second
        .unowned
        .iter()
        .find(|u| u.path_or_object == fx.shared_cache.display().to_string())
        .expect("shared cache stays unowned");
    assert_eq!(shared_cache_row.bytes, fx.shared_cache_bytes);

    let loose_row = second
        .unowned
        .iter()
        .find(|u| u.path_or_object == fx.loose_file.display().to_string())
        .expect("loose file stays unowned");
    assert_eq!(loose_row.bytes, fx.loose_file_bytes);
}

#[test]
fn golden_report_without_observe_flag_stays_read_only() {
    // report() (no store dir) must remain a pure read: no files created,
    // growth always None. This is the R1 golden test's contract; pinned
    // again here so a future change to report_with's defaults cannot
    // silently start writing to disk from a plain `report()` call.
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let r = slop_livin_core::report::report(&fx.root, Some(&fx.docker_facts))
        .expect("report() should not error");
    assert!(!r.projects.is_empty());
    for project in &r.projects {
        for worktree in &project.worktrees {
            for artifact in &worktree.artifacts {
                assert_eq!(artifact.growth_bytes, None);
                assert_eq!(artifact.regrowth_count, 0);
            }
        }
    }
}
