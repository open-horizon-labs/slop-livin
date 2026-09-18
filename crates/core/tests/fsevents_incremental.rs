//! R4b (#29): FSEvents-driven incremental observation, exercised end to
//! end through `report_full_mode_with_source` with a canned
//! [`FsEventsSource`] so nothing here depends on the live `fseventsd`.

#[path = "fixture/mod.rs"]
mod fixture;

use slop_livin_core::fs_events::{
    FsEventsPlan, FsEventsRequest, FsEventsSource, RefreshRefusal, testing::CannedSource,
};
use slop_livin_core::report::{ArtifactKind, report_full_mode, report_full_mode_with_source};
use std::fs;
use std::path::PathBuf;

/// A source that always refuses with a fixed reason, ignoring the
/// request entirely -- used to exercise each refusal reason without
/// depending on real FSEvents state.
struct RefusingSource(RefreshRefusal);

impl FsEventsSource for RefusingSource {
    fn replay(&self, _request: &FsEventsRequest) -> FsEventsPlan {
        FsEventsPlan {
            incremental: false,
            refusal: Some(self.0),
            changed_dirs: Vec::new(),
            current_event_id: 999,
            device: Some(1),
        }
    }
}

fn incremental_plan(changed: Vec<PathBuf>, event_id: u64) -> FsEventsPlan {
    FsEventsPlan {
        incremental: true,
        refusal: None,
        changed_dirs: changed,
        current_event_id: event_id,
        device: Some(1),
    }
}

#[test]
fn touching_one_artifact_resizes_only_that_row_and_matches_a_full_walk() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    // First observation: no stored event id yet, so this is a full walk
    // regardless of source (observe_tracked never even reaches the
    // source's answer when there is no prior topology). Use the real
    // report_full_mode (no store dir dependency on a source) so the
    // baseline is produced exactly as a real first-ever `slop-livin
    // observe` would.
    let first = report_full_mode(
        &fx.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        false,
    )
    .expect("first (full) report");

    // The growth store's timestamps are whole-second granularity, and
    // observe_tracked refuses a replay requested within the same second
    // as its baseline (RefreshRefusal::TooSoon) -- real usage always has
    // more turnaround than a scripted test does, so bridge that gap here.
    std::thread::sleep(std::time::Duration::from_millis(3100));
    assert!(
        first
            .notes
            .iter()
            .any(|n| n.starts_with("fsevents: mode=full")),
        "first observation of a fresh store must be a full walk: {:?}",
        first.notes
    );

    // Touch one file inside node_modules: same directory, different
    // content, so only that artifact's byte total should move.
    fs::write(fx.node_modules.join("touched.bin"), vec![b't'; 4096]).expect("write touch probe");

    // Second observation: canned source reports exactly node_modules
    // (and its parent, mirroring what a real FSEvents replay would
    // report) as changed.
    let source = CannedSource(incremental_plan(vec![fx.node_modules.clone()], 1));
    let second = report_full_mode_with_source(
        &fx.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        false,
        &source,
    )
    .expect("second (incremental) report");

    assert!(
        second
            .notes
            .iter()
            .any(|n| n.starts_with("fsevents: mode=incremental")),
        "second observation must take the incremental path: {:?}",
        second.notes
    );

    // A forced full walk from the same on-disk state, for comparison.
    let full = report_full_mode(
        &fx.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        false, // read-only: don't disturb the store the incremental path just wrote.
        false,
        false,
        true,
    )
    .expect("forced full report");

    let checkout_worktree = |r: &slop_livin_core::report::Report| {
        r.projects
            .iter()
            .find(|p| p.name == fx.checkout_name)
            .expect("checkout project")
            .worktrees
            .iter()
            .find(|w| w.path == fx.checkout)
            .expect("main worktree")
            .clone()
    };

    let inc_worktree = checkout_worktree(&second);
    let full_worktree = checkout_worktree(&full);

    let inc_by_path: std::collections::HashMap<_, _> = inc_worktree
        .artifacts
        .iter()
        .map(|a| (a.path.clone(), a.bytes))
        .collect();
    let full_by_path: std::collections::HashMap<_, _> = full_worktree
        .artifacts
        .iter()
        .map(|a| (a.path.clone(), a.bytes))
        .collect();

    assert_eq!(
        inc_by_path, full_by_path,
        "incremental and full walks must agree on every artifact row's bytes"
    );

    let node_modules_bytes = inc_by_path[&fx.node_modules];
    assert!(
        node_modules_bytes > fx.node_modules_bytes,
        "node_modules must have grown from the touched file: {node_modules_bytes} vs original {}",
        fx.node_modules_bytes
    );

    // Every other artifact row in the same worktree is byte-identical to
    // the untouched original fixture value.
    let target_bytes = inc_by_path[&fx.target_dir];
    assert_eq!(
        target_bytes, fx.target_bytes,
        "target/ must carry forward unchanged"
    );
    let dist_bytes = inc_by_path[&fx.dist_dir];
    assert_eq!(
        dist_bytes, fx.dist_bytes,
        "dist/ must carry forward unchanged"
    );

    // Reconciliation still holds exactly on the incremental report.
    assert_eq!(
        second.reconciliation.attributed + second.reconciliation.unowned,
        second.reconciliation.walked_total,
        "attributed + unowned must reconcile to walked_total on an incremental report"
    );
}

#[test]
fn new_nested_repo_is_discovered_incrementally() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    report_full_mode(
        &fx.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        false,
    )
    .expect("first (full) report");

    // The growth store's timestamps are whole-second granularity, and
    // observe_tracked refuses a replay requested within the same second
    // as its baseline (RefreshRefusal::TooSoon) -- real usage always has
    // more turnaround than a scripted test does, so bridge that gap here.
    std::thread::sleep(std::time::Duration::from_millis(3100));

    // A brand-new nested checkout appears under the existing checkout,
    // in a location the first walk never saw.
    let new_repo = fx.checkout.join("second-nested-repo");
    fs::create_dir_all(&new_repo).expect("mkdir new nested repo");
    let run_git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&new_repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "{:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "commit.gpgsign", "false"]);
    fs::write(new_repo.join("README.md"), b"new nested repo\n").expect("write README");
    run_git(&["add", "README.md"]);
    run_git(&["commit", "-q", "-m", "initial commit"]);

    let source = CannedSource(incremental_plan(vec![new_repo.clone()], 1));
    let second = report_full_mode_with_source(
        &fx.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        false,
        &source,
    )
    .expect("second (incremental) report");

    assert!(
        second
            .projects
            .iter()
            .any(|p| p.worktrees.iter().any(|w| w.path == new_repo)),
        "the newly created nested repo must appear as a discovered worktree after an \
         incremental observation implicates its directory"
    );
}

#[test]
fn every_refusal_reason_falls_back_to_a_full_walk() {
    for reason in [
        RefreshRefusal::NoStoredEventId,
        RefreshRefusal::EventIdFromFuture,
        RefreshRefusal::RootMismatch,
        RefreshRefusal::FseventsdUnavailable,
        RefreshRefusal::HelperInconclusive,
        RefreshRefusal::TooManyChanges,
        RefreshRefusal::UnsupportedPlatform,
    ] {
        let tmp = tempfile::tempdir().expect("tmp root");
        let fx = fixture::build(tmp.path());
        let store = tempfile::tempdir().expect("tmp store");

        report_full_mode(
            &fx.root,
            None,
            false,
            Some(store.path()),
            Some("1h"),
            true,
            false,
            false,
            false,
        )
        .expect("first (full) report");

        // The growth store's timestamps are whole-second granularity, and
        // observe_tracked refuses a replay requested within the same second
        // as its baseline (RefreshRefusal::TooSoon) -- real usage always has
        // more turnaround than a scripted test does, so bridge that gap here.
        std::thread::sleep(std::time::Duration::from_millis(3100));

        let source = RefusingSource(reason);
        let r = report_full_mode_with_source(
            &fx.root,
            None,
            false,
            Some(store.path()),
            Some("1h"),
            true,
            false,
            false,
            false,
            &source,
        )
        .expect("refused report still succeeds as a full walk");

        let expected = format!(
            "fsevents: mode=full reason={} changed_dirs=0",
            reason.as_str()
        );
        assert!(
            r.notes.iter().any(|n| n == &expected),
            "reason {:?} must produce note {expected:?}, got {:?}",
            reason,
            r.notes
        );
        // A refused observation is still a correct one: every artifact
        // kind classified by the fixture is present.
        let checkout = r
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
            main.artifacts
                .iter()
                .any(|a| a.kind == ArtifactKind::DependencyTree),
            "a full walk must still classify node_modules"
        );
    }
}

#[test]
fn stored_event_id_is_recorded_after_an_observation() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().expect("tmp store");

    report_full_mode(
        &fx.root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        false,
    )
    .expect("report");

    let volume_id = fs::metadata(&fx.root)
        .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
        .unwrap_or(0);
    let sidecar = store
        .path()
        .join(volume_id.to_string())
        .join("fsevents.json");
    let text =
        fs::read_to_string(&sidecar).unwrap_or_else(|e| panic!("read {}: {e}", sidecar.display()));
    assert!(
        text.contains("event_id"),
        "fsevents.json must record the observed event id: {text}"
    );
}
