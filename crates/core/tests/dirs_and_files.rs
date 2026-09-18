//! R4c: per-directory rows under Source, large-file rows, and the
//! folding contract that keeps folded artifacts (node_modules, target,
//! .git, ...) at exactly one row with no `DirRollup`s underneath.

#[path = "fixture/mod.rs"]
mod fixture;

use slop_livin_core::report::report_with_dirs;
use std::fs;

#[test]
fn growth_appears_on_file_row_and_every_directory_in_the_chain() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    // The directory chain and the file itself must already exist at the
    // first observation (a brand-new row has no baseline to diff
    // against, same rule as artifact rows in growth.rs) so growth can be
    // measured on the *change* between the two observations, not on the
    // row's mere appearance.
    let deep_dir = fx.checkout.join("a").join("b").join("c");
    fs::create_dir_all(&deep_dir).expect("mkdir deep");
    // Must already be >= the large-file threshold (1 MiB default) at the
    // first observation, so a `FileRow` baseline exists to diff against;
    // below that threshold the file is never tracked as a row at all.
    let big_path = deep_dir.join("big.bin");
    fs::write(&big_path, vec![b'x'; 2 * 1024 * 1024]).expect("write initial file");

    // First observation: baseline.
    let first = report_with_dirs(&fx.root, None, false, Some(store.path()), Some("1h"), true)
        .expect("first report");
    assert!(
        first.dirs_by_worktree.is_some(),
        "include_dirs=true must populate dirs_by_worktree"
    );

    // Grow the file to 50 MiB, three levels deep in Source (not inside
    // any folded artifact).
    let big_bytes = 50 * 1024 * 1024;
    fs::write(&big_path, vec![b'x'; big_bytes]).expect("write big file");

    let second = report_with_dirs(&fx.root, None, false, Some(store.path()), Some("1h"), true)
        .expect("second report");

    let worktree_id = second
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .and_then(|p| p.worktrees.iter().find(|w| w.path == fx.checkout))
        .map(|w| w.worktree_id.clone())
        .expect("main worktree");

    let files = second
        .files_by_worktree
        .as_ref()
        .expect("files_by_worktree populated")
        .get(&worktree_id)
        .expect("worktree has file rows");
    let big_file_rel = "a/b/c/big.bin";
    let big_row = files
        .iter()
        .find(|f| f.rel_path == big_file_rel)
        .unwrap_or_else(|| panic!("expected a FileRow for {big_file_rel}: {files:?}"));
    assert!(
        big_row.growth_bytes.unwrap_or(0) > (big_bytes as i64) / 2,
        "large file row must show its growth since the prior observation: {big_row:?}"
    );

    let dirs = second
        .dirs_by_worktree
        .as_ref()
        .expect("dirs_by_worktree populated")
        .get(&worktree_id)
        .expect("worktree has dir rows");

    for rel in ["a", "a/b", "a/b/c"] {
        let row = dirs
            .iter()
            .find(|d| d.rel_path == rel)
            .unwrap_or_else(|| panic!("expected a DirRollup for {rel}: {dirs:?}"));
        assert!(
            row.growth_bytes.unwrap_or(0) > (big_bytes as i64) / 2,
            "directory {rel} in the chain must show the new file's growth: {row:?}"
        );
    }
}

#[test]
fn no_dir_rollup_rows_exist_under_folded_artifact_directories() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());

    let report = report_with_dirs(&fx.root, None, false, None, None, true).expect("report");

    let worktree_id = report
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .and_then(|p| p.worktrees.iter().find(|w| w.path == fx.checkout))
        .map(|w| w.worktree_id.clone())
        .expect("main worktree");

    let dirs = report
        .dirs_by_worktree
        .as_ref()
        .expect("dirs_by_worktree populated")
        .get(&worktree_id)
        .expect("worktree has dir rows");

    for folded in ["node_modules", "target", "dist", ".git"] {
        assert!(
            !dirs
                .iter()
                .any(|d| d.rel_path == folded || d.rel_path.starts_with(&format!("{folded}/"))),
            "folding is what keeps this affordable: no DirRollup should exist \
             for or under {folded}, got: {:?}",
            dirs.iter().map(|d| &d.rel_path).collect::<Vec<_>>()
        );
    }

    // node_modules itself is still exactly one ArtifactRow.
    let main = report
        .projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .find(|w| w.path == fx.checkout)
        .expect("main worktree row");
    let node_modules_rows = main
        .artifacts
        .iter()
        .filter(|a| a.path == fx.node_modules)
        .count();
    assert_eq!(
        node_modules_rows, 1,
        "node_modules stays exactly one folded artifact row"
    );
}

#[test]
fn no_change_observation_appends_no_dir_or_file_delta() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    let _first = report_with_dirs(&fx.root, None, false, Some(store.path()), Some("1h"), true)
        .expect("first report");

    let volume_id = std::fs::metadata(&fx.root)
        .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
        .unwrap_or(0);
    let volume_dir = store.path().join(volume_id.to_string());
    let dirs_deltas = volume_dir.join("dirs_deltas");
    let files_deltas = volume_dir.join("files_deltas");
    let count_parquet = |p: &std::path::Path| {
        std::fs::read_dir(p)
            .map(|it| it.flatten().count())
            .unwrap_or(0)
    };
    let after_first_dirs = count_parquet(&dirs_deltas);
    let after_first_files = count_parquet(&files_deltas);

    // Second observation with no filesystem changes at all.
    let _second = report_with_dirs(&fx.root, None, false, Some(store.path()), Some("1h"), true)
        .expect("second report");

    assert_eq!(
        count_parquet(&dirs_deltas),
        after_first_dirs,
        "no-change observation must not append a dirs delta file"
    );
    assert_eq!(
        count_parquet(&files_deltas),
        after_first_files,
        "no-change observation must not append a files delta file"
    );
}

#[test]
fn mod_time_min_round_trips_as_i32_minutes() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());

    let report = report_with_dirs(&fx.root, None, false, None, None, true).expect("report");
    let worktree_id = report
        .projects
        .iter()
        .find(|p| p.name == fx.checkout_name)
        .and_then(|p| p.worktrees.iter().find(|w| w.path == fx.checkout))
        .map(|w| w.worktree_id.clone())
        .expect("main worktree");
    let dirs = report
        .dirs_by_worktree
        .as_ref()
        .unwrap()
        .get(&worktree_id)
        .unwrap();

    // The worktree root row ("") must exist and carry a plausible,
    // recent mod_time_min (minutes since epoch, not seconds).
    let root_row = dirs
        .iter()
        .find(|d| d.rel_path.is_empty())
        .expect("worktree root DirRollup exists");
    let now_minutes = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 60) as i32;
    assert!(
        root_row.mod_time_min > 0 && root_row.mod_time_min <= now_minutes + 1,
        "mod_time_min ({}) must be minutes-since-epoch, not seconds or zero",
        root_row.mod_time_min
    );
}
