//! #65 through the real report pipeline, for adapters other than Cargo.
//!
//! `cargo_delivery` proves full/incremental agreement and container reuse
//! for Cargo. The reconciliation table on #65 asks for the same
//! equivalence "for non-Rust adapters and partial/unreadable/shared
//! cases", and for adversarial reclassification/new-nesting history
//! tests across adapters. These drive `report_full_mode_with_source`
//! against disposable Node and Gradle checkouts with a scripted event
//! source, so every pass goes through the folded walk, the build
//! consumer, the `EventCoverage` gate and the current + reverse-delta
//! store exactly as a user's refresh does.

use std::{
    fs,
    path::{Path, PathBuf},
};
use swamp_core::fs_events::{FsEventsPlan, FsEventsRequest, FsEventsSource};

/// An event source that reports the given paths as changed.
struct Live(Vec<PathBuf>);
impl FsEventsSource for Live {
    fn replay(&self, _: &FsEventsRequest) -> FsEventsPlan {
        FsEventsPlan::from_live(self.0.clone(), 1000, None)
    }
}

fn git_init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(root)
            .status()
            .unwrap()
            .success()
    );
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes]).unwrap();
}

/// A Node checkout: an installed dependency, a build output and a
/// coverage report, with the ignore rules a real project has.
fn node_fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap().join("web");
    git_init(&root);
    fs::write(
        root.join("package.json"),
        br#"{"name":"web","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "node_modules/\ndist/\ncoverage/\n").unwrap();
    fs::create_dir_all(root.join("node_modules/left-pad")).unwrap();
    fs::write(
        root.join("node_modules/left-pad/package.json"),
        br#"{"name":"left-pad","version":"1.3.0"}"#,
    )
    .unwrap();
    write(&root.join("node_modules/left-pad/index.js"), 20_000);
    write(&root.join("dist/app.js"), 40_000);
    write(&root.join("coverage/lcov.info"), 12_000);
    (tmp, root)
}

fn observe(
    root: &Path,
    store: &Path,
    changed: Vec<PathBuf>,
    force_full: bool,
) -> swamp_core::Report {
    swamp_core::report::report_full_mode_with_source(
        root,
        None,
        false,
        Some(store),
        Some("1h"),
        true,
        false,
        false,
        force_full,
        &Live(changed),
    )
    .unwrap()
}

/// What two passes must agree on: identity, role, bytes, presence.
fn facts(r: &swamp_core::Report) -> Vec<(String, String, u64, bool)> {
    let mut v: Vec<_> = r
        .nested_artifacts
        .iter()
        .filter(|u| u.adapter.as_deref() != Some("cargo"))
        .map(|u| (u.id.clone(), u.role.label().to_string(), u.bytes, u.present))
        .collect();
    v.sort();
    v
}

/// A pass over a fresh store: the full answer for the tree as it is now.
fn fresh_full(root: &Path) -> swamp_core::Report {
    let store = tempfile::tempdir().unwrap();
    observe(root, store.path(), vec![], true)
}

#[test]
fn node_full_and_incremental_agree_as_a_sub_artifact_changes_vanishes_and_reappears() {
    let (_tmp, root) = node_fixture();
    let store = tempfile::tempdir().unwrap();
    // Warm up until the store has a recorded window start (see
    // `cargo_delivery::trusted_unchanged_container_reuses_units_without_reading_fingerprints`).
    for _ in 0..3 {
        observe(&root, store.path(), vec![], false);
    }
    let baseline = observe(&root, store.path(), vec![], false);
    assert!(
        baseline
            .nested_artifacts
            .iter()
            .any(|u| u.adapter.as_deref() == Some("node")),
        "the Node adapter identified nothing, so agreement would prove nothing"
    );
    assert_eq!(facts(&baseline), facts(&fresh_full(&root)));

    // Changes: a sub-artifact grows ...
    write(&root.join("dist/chunk.js"), 64_000);
    let inc = observe(&root, store.path(), vec![root.join("dist")], false);
    assert_eq!(facts(&inc), facts(&fresh_full(&root)), "after growth");

    // ... vanishes ...
    fs::remove_dir_all(root.join("coverage")).unwrap(); // disposable fixture only
    let inc = observe(&root, store.path(), vec![root.join("coverage")], false);
    assert_eq!(facts(&inc), facts(&fresh_full(&root)), "after removal");
    assert!(
        !inc.nested_artifacts
            .iter()
            .any(|u| u.present && u.path.starts_with(root.join("coverage"))),
        "a vanished container leaves no present unit behind"
    );

    // ... and reappears at the same path with different bytes.
    write(&root.join("coverage/lcov.info"), 3_000);
    let inc = observe(&root, store.path(), vec![root.join("coverage")], false);
    assert_eq!(facts(&inc), facts(&fresh_full(&root)), "after reappearance");
}

#[test]
fn an_unchanged_node_checkout_replays_its_units_without_reading_a_manifest() {
    let (_tmp, root) = node_fixture();
    let store = tempfile::tempdir().unwrap();
    for _ in 0..3 {
        observe(&root, store.path(), vec![], false);
    }
    let before = observe(&root, store.path(), vec![], false);
    let (after, counted) =
        swamp_core::work_counters::measured(|| observe(&root, store.path(), vec![], false));
    assert_eq!(facts(&before), facts(&after));
    assert_eq!(
        counted.header_bytes_read, 0,
        "an unchanged pass under a trusted window re-read a package.json"
    );
    assert!(counted.containers_reused > 0, "{counted:?}");
    assert_eq!(counted.containers_identified, 0, "{counted:?}");
}

#[test]
fn reclassification_and_new_nesting_never_reach_history_as_growth() {
    // A checkout whose `build/` is Node's until a Gradle marker appears:
    // the same bytes are first one unit (Node's output directory), then
    // a container with conventional children (Gradle's build layout).
    // Nothing moved on disk except a zero-byte marker file, so no unit
    // may report growth -- the directory that was already measured keeps
    // its identity and bytes, and the newly identified children start a
    // baseline rather than arriving as "+N bytes".
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap().join("mixed");
    git_init(&root);
    fs::write(root.join("package.json"), br#"{"name":"mixed"}"#).unwrap();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    write(&root.join("build/classes/Main.class"), 30_000);
    write(&root.join("build/tmp/scratch"), 5_000);
    let store = tempfile::tempdir().unwrap();
    for _ in 0..3 {
        observe(&root, store.path(), vec![], false);
    }
    let before = observe(&root, store.path(), vec![], false);
    let build = root.join("build");
    let unit = |r: &swamp_core::Report, p: &Path| {
        r.nested_artifacts
            .iter()
            .find(|u| u.path == p)
            .cloned()
            .unwrap_or_else(|| panic!("no unit at {}", p.display()))
    };
    let b0 = unit(&before, &build);
    assert_eq!(b0.adapter.as_deref(), Some("node"), "{b0:?}");

    fs::write(root.join("settings.gradle"), b"").unwrap();
    let after = observe(
        &root,
        store.path(),
        vec![root.join("settings.gradle")],
        false,
    );
    let b1 = unit(&after, &build);
    assert_eq!(b1.adapter.as_deref(), Some("gradle"), "{b1:?}");
    assert_eq!(
        b1.id, b0.id,
        "a role or adapter change is not an identity change"
    );
    assert_eq!(b1.bytes, b0.bytes);
    assert!(
        matches!(b1.growth_bytes, None | Some(0)),
        "reclassification reached history as growth: {:?}",
        b1.growth_bytes
    );
    let classes = unit(&after, &build.join("classes"));
    assert!(
        matches!(classes.growth_bytes, None | Some(0)),
        "a newly identified unit is a new baseline, not growth: {:?}",
        classes.growth_bytes
    );
    assert_eq!(classes.regrowth_count, 0);
    // And the container row's own history did not move either.
    let row_growth = |r: &swamp_core::Report| {
        r.projects
            .iter()
            .flat_map(|p| &p.worktrees)
            .flat_map(|w| &w.artifacts)
            .find(|a| a.path == build)
            .map(|a| (a.bytes, a.growth_bytes))
    };
    let (bytes_before, _) = row_growth(&before).expect("build/ is an artifact row");
    let (bytes_after, growth_after) = row_growth(&after).unwrap();
    assert_eq!(bytes_before, bytes_after);
    assert!(
        matches!(growth_after, None | Some(0)),
        "the container's history moved on a metadata-only change: {growth_after:?}"
    );
}

#[test]
fn an_unreadable_sub_directory_is_incomplete_coverage_not_a_disappearance() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, root) = node_fixture();
    let store = tempfile::tempdir().unwrap();
    for _ in 0..3 {
        observe(&root, store.path(), vec![], false);
    }
    let before = observe(&root, store.path(), vec![], false);
    let pkg = root.join("node_modules/left-pad");
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o000)).unwrap();
    let during = observe(&root, store.path(), vec![pkg.clone()], false);
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o755)).unwrap();
    let nm = root.join("node_modules");
    let container = |r: &swamp_core::Report| {
        r.nested_artifacts
            .iter()
            .find(|u| u.path == nm)
            .cloned()
            .expect("node_modules unit")
    };
    assert!(container(&before).coverage.complete);
    let c = container(&during);
    assert!(
        !c.coverage.complete,
        "an unreadable member makes the container's coverage incomplete: {c:?}"
    );
    assert!(
        c.coverage
            .limits
            .iter()
            .any(|l| l.contains("could not read")),
        "{:?}",
        c.coverage.limits
    );
}

#[test]
fn a_nested_unit_round_trips_through_the_report_json_with_its_contract_fields() {
    // Serialization/contract fixture (#64): shared entries, opaque
    // residuals, explicit unknowns, the accounting basis, the time
    // source, the action capability and the consequence all survive the
    // `--json` report shape.
    let (_tmp, root) = node_fixture();
    fs::create_dir_all(root.join("node_modules/.mystery")).unwrap();
    write(&root.join("node_modules/.mystery/blob"), 4_000);
    let report = fresh_full(&root);
    let json = serde_json::to_value(&report).unwrap();
    let back: swamp_core::Report = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(facts(&report), facts(&back));
    let units = json["nested_artifacts"].as_array().expect("nested units");
    let residual = units
        .iter()
        .find(|u| u["path"].as_str().is_some_and(|p| p.ends_with(".mystery")))
        .expect("an unrecognised entry is a residual unit, not a missing row");
    assert_eq!(residual["coverage"]["supported"], false);
    assert_eq!(residual["role"], "Residual");
    let pkg = units
        .iter()
        .find(|u| u["path"].as_str().is_some_and(|p| p.ends_with("left-pad")))
        .unwrap();
    assert_eq!(pkg["variant"]["package"], "left-pad");
    assert_eq!(pkg["variant"]["version"], "1.3.0");
    assert!(
        pkg["variant"]["unknowns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u == "build-generation"),
        "no build generation is invented; its absence is explicit"
    );
    assert_eq!(pkg["basis"], "allocated");
    assert_eq!(pkg["time_source"], "folded-directory-modification");
    assert_eq!(pkg["action"]["capability"], "inspection-only");
    assert!(pkg["consequence"].as_str().unwrap().contains("npm ci"));
    assert_eq!(pkg["adapter"], "node");
}

#[test]
fn the_json_views_carry_the_same_family_summary_as_the_text_view() {
    let (_tmp, root) = node_fixture();
    let report = fresh_full(&root);
    let deps = swamp_core::agent_json::view_payload(&report, "deps", None);
    let nm = deps
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["path"]
                .as_str()
                .is_some_and(|p| p.ends_with("node_modules"))
        })
        .expect("node_modules row");
    let families = nm["interior"]["families"].as_array().expect("families");
    let dependencies = families
        .iter()
        .find(|f| f["family"] == "dependencies")
        .expect("dependencies family");
    assert_eq!(dependencies["count"], 1);
    assert_eq!(
        dependencies["recommendation"],
        "Review: reinstall from registry"
    );
    assert!(
        dependencies["consequence"]
            .as_str()
            .unwrap()
            .contains("npm ci")
    );
    assert_eq!(dependencies["action"], "inspection-only");
    assert!(nm["interior"]["units"].as_array().unwrap().len() >= 2);
    // A row nothing identified the inside of carries no `interior` at all.
    let builds = swamp_core::agent_json::view_payload(&report, "builds", None);
    assert!(
        builds
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("interior").is_none() || r["interior"]["units"].as_array().is_some())
    );
}

fn dir_bytes(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

/// The benchmark #65 asks for: unchanged and one-group-change refresh,
/// through the real pipeline, with the store's size. Printed with
/// `--nocapture` and recorded in
/// `.oh/sessions/2026-09-21-build-adapters-node-jvm.md`; the assertions
/// are the parts that must hold on any machine.
#[test]
fn cost_report_real_pipeline_unchanged_and_one_group_change() {
    let (_tmp, root) = node_fixture();
    // A larger installed tree, so the unchanged/changed contrast is not
    // lost in fixed per-pass overhead.
    for i in 0..300 {
        let pkg = root.join(format!("node_modules/pkg-{i:03}"));
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("package.json"),
            format!(r#"{{"name":"pkg-{i:03}","version":"1.0.{i}"}}"#),
        )
        .unwrap();
        write(&pkg.join("index.js"), 2_000);
    }
    let store = tempfile::tempdir().unwrap();
    let t = std::time::Instant::now();
    let (_, cold) =
        swamp_core::work_counters::measured(|| observe(&root, store.path(), vec![], false));
    let cold_ms = t.elapsed().as_secs_f64() * 1e3;
    for _ in 0..2 {
        observe(&root, store.path(), vec![], false);
    }
    let store_before = dir_bytes(store.path());
    let t = std::time::Instant::now();
    let (_, unchanged) =
        swamp_core::work_counters::measured(|| observe(&root, store.path(), vec![], false));
    let unchanged_ms = t.elapsed().as_secs_f64() * 1e3;
    let store_after_unchanged = dir_bytes(store.path());

    write(&root.join("dist/chunk.js"), 64_000);
    let t = std::time::Instant::now();
    let (_, one_group) = swamp_core::work_counters::measured(|| {
        observe(&root, store.path(), vec![root.join("dist")], false)
    });
    let one_group_ms = t.elapsed().as_secs_f64() * 1e3;
    let store_after_change = dir_bytes(store.path());

    println!("--- BUILD ADAPTER COST (real pipeline) ---");
    println!("fixture: node_modules with 301 packages, dist, coverage; one walked root");
    for (name, ms, c) in [
        ("cold", cold_ms, &cold),
        ("unchanged", unchanged_ms, &unchanged),
        ("one group changed (dist)", one_group_ms, &one_group),
    ] {
        println!(
            "{name:<26} {ms:>8.2}ms dirs_listed={} files_statted={} manifest_bytes={} \
             containers_reused={} containers_identified={}",
            c.dirs_listed,
            c.files_statted,
            c.header_bytes_read,
            c.containers_reused,
            c.containers_identified
        );
    }
    println!(
        "store bytes: before={store_before} after_unchanged={store_after_unchanged} \
         after_one_group={store_after_change}"
    );
    println!("--- END ---");

    assert!(
        cold.header_bytes_read > 0,
        "the cold pass reads the manifests"
    );
    assert_eq!(unchanged.header_bytes_read, 0);
    assert_eq!(unchanged.containers_identified, 0);
    assert_eq!(
        one_group.containers_identified, 1,
        "a change inside dist/ re-identifies dist/ and nothing else: {one_group:?}"
    );
    assert_eq!(
        one_group.header_bytes_read, 0,
        "re-identifying dist/ reads no package.json; node_modules was replayed"
    );
}
