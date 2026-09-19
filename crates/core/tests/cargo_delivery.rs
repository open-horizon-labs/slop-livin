use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use swamp_core::{
    actions,
    artifact::ArtifactRole,
    cargo_artifacts::{inspect_target, inspect_target_incremental},
    report::report_full_mode,
};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "target/\n").unwrap();
    let target = root.join("target");
    fs::create_dir_all(target.join("debug/deps")).unwrap();
    fs::create_dir_all(target.join("debug/.fingerprint/fixture-aaa")).unwrap();
    fs::write(target.join("debug/.cargo-lock"), b"").unwrap();
    fs::write(
        target.join("debug/.fingerprint/fixture-aaa/test-lib-fixture.json"),
        r#"{"rustc":42,"features":"[]"}"#,
    )
    .unwrap();
    for name in [
        "fixture-aaa",
        "fixture-aaa.d",
        "libdependency.rlib",
        "newer-bbb",
    ] {
        fs::write(target.join("debug/deps").join(name), vec![1u8; 8192]).unwrap();
    }
    fs::set_permissions(
        target.join("debug/deps/fixture-aaa"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    (tmp, root, target)
}
fn report(root: &Path, store: &Path) -> swamp_core::Report {
    report_full_mode(
        root,
        None,
        false,
        Some(store),
        Some("1h"),
        true,
        false,
        false,
        true,
    )
    .unwrap()
}
#[test]
fn normal_report_identifies_tests_and_exact_cleanup_preserves_neighbors() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let r = report(&root, store.path());
    let top_bytes: u64 = r
        .projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .flat_map(|w| &w.artifacts)
        .map(|a| a.bytes)
        .sum();
    assert_eq!(
        r.total_series.last().copied().flatten(),
        Some(top_bytes),
        "nested rows must not inflate root history"
    );
    let selected = target.join("debug/deps/fixture-aaa");
    assert_eq!(
        r.nested_artifacts
            .iter()
            .find(|u| u.path == selected)
            .unwrap()
            .role,
        ArtifactRole::TestExecutable
    );
    let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
    assert_eq!(plan.units[0].cargo_group.as_ref().unwrap().members.len(), 2);
    actions::save_plan(store.path(), &plan).unwrap();
    let trash = store.path().join("trash");
    assert_eq!(
        actions::execute_with_trash(store.path(), &plan.id, "test", &trash)
            .unwrap()
            .state,
        "awaiting-authorization"
    );
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let result = actions::execute_with_trash(store.path(), &plan.id, "test", &trash).unwrap();
    assert_eq!(
        result.outcomes[0].status, "completed",
        "{:?}",
        result.outcomes
    );
    assert!(!selected.exists());
    assert!(!selected.with_extension("d").exists());
    assert!(target.join("debug/deps/newer-bbb").exists());
    assert!(target.join("debug/deps/libdependency.rlib").exists());
    assert!(
        result.outcomes[0]
            .recovery_location
            .as_ref()
            .unwrap()
            .join("restore.json")
            .exists()
    );
    assert_eq!(
        actions::execute_with_trash(store.path(), &plan.id, "test", &trash)
            .unwrap()
            .state,
        "already-executed"
    );
}
#[test]
fn stale_same_size_member_refuses_entire_group() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let r = report(&root, store.path());
    let selected = target.join("debug/deps/fixture-aaa");
    let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    fs::write(selected.with_extension("d"), vec![2u8; 8192]).unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_ne!(result.outcomes[0].status, "completed");
    assert!(result.outcomes[0].cause.as_ref().unwrap().contains("stale"));
    assert!(selected.exists());
    assert!(selected.with_extension("d").exists());
}
#[test]
fn cargo_lock_and_overlapping_selection_refuse() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let r = report(&root, store.path());
    let selected = target.join("debug/deps/fixture-aaa");
    assert!(
        actions::propose(&r, None, &[target.clone(), selected.clone()], "test")
            .unwrap_err()
            .to_string()
            .contains("overlapping")
    );
    let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let lock = fs::File::open(target.join("debug/.cargo-lock")).unwrap();
    lock.try_lock().unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert!(result.outcomes[0].cause.as_ref().unwrap().contains("busy"));
    assert!(selected.exists());
}
#[test]
fn shared_hardlink_never_becomes_cleanup_candidate() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let selected = target.join("debug/deps/fixture-aaa");
    fs::hard_link(&selected, target.join("retained-alias")).unwrap();
    let r = report(&root, store.path());
    assert!(
        actions::propose(&r, None, &[selected], "test")
            .unwrap_err()
            .to_string()
            .contains("hardlink")
    );
}
#[test]
fn incremental_same_size_rename_and_metadata_change_match_full() {
    let (_tmp, root, target) = fixture();
    let first = inspect_target(&target, Some(&root));
    let from = target.join("debug/deps/newer-bbb");
    let to = target.join("debug/deps/renamed-ccc");
    fs::rename(from, &to).unwrap();
    fs::remove_file(target.join("debug/.fingerprint/fixture-aaa/test-lib-fixture.json")).unwrap();
    let changes = vec![
        target.join("debug/deps"),
        target.join("debug/.fingerprint/fixture-aaa"),
    ];
    let inc = inspect_target_incremental(&target, Some(&root), &first.units, Some(&changes));
    let full = inspect_target(&target, Some(&root));
    let facts = |r: &swamp_core::cargo_artifacts::CargoInspection| {
        r.units
            .iter()
            .map(|u| {
                (
                    u.id.clone(),
                    u.path.clone(),
                    u.role.clone(),
                    u.bytes,
                    u.physical_bytes,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(facts(&inc), facts(&full));
    assert!(inc.units.iter().any(|u| u.path == to));
    assert!(
        !inc.units
            .iter()
            .any(|u| u.role == ArtifactRole::TestExecutable)
    );
}

#[test]
fn event_pipeline_replaces_renames_and_drops_deleted_roots() {
    use swamp_core::fs_events::{FsEventsPlan, FsEventsRequest, FsEventsSource};
    struct Live(Vec<PathBuf>);
    impl FsEventsSource for Live {
        fn replay(&self, _: &FsEventsRequest) -> FsEventsPlan {
            FsEventsPlan::from_live(self.0.clone(), 1000, None)
        }
    }
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let first = report(&root, store.path());
    assert!(!first.nested_artifacts.is_empty());
    let from = target.join("debug/deps/newer-bbb");
    let to = target.join("debug/deps/renamed-ccc");
    fs::rename(&from, &to).unwrap();
    let run = |changed| {
        swamp_core::report::report_full_mode_with_source(
            &root,
            None,
            false,
            Some(store.path()),
            Some("1h"),
            true,
            false,
            false,
            false,
            &Live(changed),
        )
        .unwrap()
    };
    let next = run(vec![target.join("debug/deps")]);
    assert!(!next.nested_artifacts.iter().any(|u| u.path == to));
    assert!(!next.nested_artifacts.iter().any(|u| u.path == from));
    let size = |r: &swamp_core::Report| {
        r.nested_artifacts
            .iter()
            .find(|u| u.path == target.join("debug/deps"))
            .unwrap()
            .bytes
    };
    assert_eq!(size(&first), size(&next));
    fs::remove_dir_all(&target).unwrap(); // disposable fixture only
    let next = run(vec![root.clone(), target]);
    assert!(next.nested_artifacts.is_empty());
}

#[test]
fn new_companion_and_symlink_substitution_refuse() {
    for symlink in [false, true] {
        let (_tmp, root, target) = fixture();
        let store = tempfile::tempdir().unwrap();
        let selected = target.join("debug/deps/fixture-aaa");
        fs::remove_file(selected.with_extension("d")).unwrap();
        let r = report(&root, store.path());
        let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
        actions::save_plan(store.path(), &plan).unwrap();
        actions::approve(store.path(), &plan.id, "human:test").unwrap();
        if symlink {
            fs::remove_file(&selected).unwrap();
            std::os::unix::fs::symlink(target.join("debug/deps/newer-bbb"), &selected).unwrap();
        } else {
            fs::write(selected.with_extension("d"), "new companion").unwrap();
        }
        let result = actions::execute_with_trash(
            store.path(),
            &plan.id,
            "test",
            &store.path().join("trash"),
        )
        .unwrap();
        assert_ne!(result.outcomes[0].status, "completed");
        assert!(selected.exists());
        assert!(target.join("debug/deps/newer-bbb").exists());
    }
}

#[test]
fn absent_lock_and_unsupported_role_are_inspection_only() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let r = report(&root, store.path());
    assert!(
        actions::propose(
            &r,
            None,
            &[target.join("debug/deps/libdependency.rlib")],
            "test"
        )
        .is_err()
    );
    fs::remove_file(target.join("debug/.cargo-lock")).unwrap();
    assert!(
        actions::propose(&r, None, &[target.join("debug/deps/fixture-aaa")], "test")
            .unwrap_err()
            .to_string()
            .contains("lock")
    );
}

#[test]
fn incremental_and_build_script_entries_are_exact_directory_groups() {
    for kind in ["incremental", "build"] {
        let (_tmp, root, target) = fixture();
        let store = tempfile::tempdir().unwrap();
        let selected = target.join("debug").join(kind).join("old-aaa");
        let kept = selected.with_file_name("new-bbb");
        for path in [&selected, &kept] {
            fs::create_dir_all(path).unwrap();
            fs::write(path.join("cache"), vec![1u8; 4096]).unwrap();
        }
        let r = report(&root, store.path());
        let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
        actions::save_plan(store.path(), &plan).unwrap();
        actions::approve(store.path(), &plan.id, "human:test").unwrap();
        let result = actions::execute_with_trash(
            store.path(),
            &plan.id,
            "test",
            &store.path().join("trash"),
        )
        .unwrap();
        assert_eq!(
            result.outcomes[0].status, "completed",
            "{:?}",
            result.outcomes
        );
        assert!(!selected.exists());
        assert!(kept.join("cache").exists());
    }
}

#[test]
fn directory_group_new_member_is_stale_not_silently_included() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let selected = target.join("debug/incremental/old-aaa");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("a"), "a").unwrap();
    let r = report(&root, store.path());
    let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    fs::write(selected.join("b"), "new").unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_ne!(result.outcomes[0].status, "completed");
    assert!(selected.join("b").exists());
}

#[test]
fn thousands_of_compiler_files_stay_folded_in_reports_and_history() {
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let group = target.join("debug/incremental/fixture-aaa");
    let interior = group.join("session/objects");
    fs::create_dir_all(&interior).unwrap();
    fs::write(interior.join("initial.o"), vec![1u8; 4096]).unwrap();
    fs::File::open(&group)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(120)),
        )
        .unwrap();
    let first = report(&root, store.path());
    assert!(
        first
            .nested_artifacts
            .iter()
            .find(|u| u.path == group)
            .unwrap()
            .mtime_max
            > 120
    );
    let size = |r: &swamp_core::Report| {
        r.nested_artifacts
            .iter()
            .find(|u| u.path == group)
            .unwrap()
            .bytes
    };
    for i in 0..5000 {
        fs::write(
            interior.join(format!("compiler-internal-{i}.o")),
            vec![1u8; 4096],
        )
        .unwrap();
    }
    let next = report(&root, store.path());
    assert_eq!(first.nested_artifacts.len(), next.nested_artifacts.len());
    assert!(size(&next) >= size(&first) + 5000 * 4096);
    assert!(
        !next
            .nested_artifacts
            .iter()
            .any(|u| u.path.starts_with(&interior))
    );
    assert_eq!(first.series_by_key.len(), next.series_by_key.len());
    let cache = swamp_core::report::load_last_report(store.path(), &root).unwrap();
    assert_eq!(cache.nested_artifacts.len(), next.nested_artifacts.len());
    assert!(serde_json::to_vec(&cache.nested_artifacts).unwrap().len() < 20_000);
    // The persisted column store must not hide the discarded file inventory.
    fn assert_no_file_inventory(dir: &Path) {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                assert_no_file_inventory(&path);
            } else if path.extension().is_some_and(|s| s == "parquet") {
                let reader =
                    ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap())
                        .unwrap()
                        .build()
                        .unwrap();
                for batch in reader {
                    let batch = batch.unwrap();
                    assert!(batch.num_rows() < 100, "file-count-sized persisted batch");
                    for col in batch.columns() {
                        if let Some(col) = col.as_any().downcast_ref::<arrow_array::StringArray>() {
                            assert!(
                                !col.iter()
                                    .flatten()
                                    .any(|s| s.contains("compiler-internal-"))
                            );
                        }
                    }
                }
            }
        }
    }
    assert_no_file_inventory(store.path());
}

#[test]
fn cleanup_rechecks_fingerprint_but_not_unrelated_build_trees() {
    for changed_evidence in [false, true] {
        let (_tmp, root, target) = fixture();
        let store = tempfile::tempdir().unwrap();
        let selected = target.join("debug/deps/fixture-aaa");
        let r = report(&root, store.path());
        let plan = actions::propose(&r, None, &[selected.clone()], "test").unwrap();
        actions::save_plan(store.path(), &plan).unwrap();
        actions::approve(store.path(), &plan.id, "human:test").unwrap();
        // An unrelated unreadable tree must not block scoped cleanup.
        let unrelated = target.join("unrelated");
        fs::create_dir(&unrelated).unwrap();
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o000)).unwrap();
        if changed_evidence {
            fs::write(
                target.join("debug/.fingerprint/fixture-aaa/test-lib-fixture.json"),
                r#"{"rustc":43}"#,
            )
            .unwrap();
        }
        let result = actions::execute_with_trash(
            store.path(),
            &plan.id,
            "test",
            &store.path().join("trash"),
        );
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o755)).unwrap();
        let result = result.unwrap();
        assert_eq!(
            result.outcomes[0].status == "completed",
            !changed_evidence,
            "{:?}",
            result.outcomes
        );
        assert_eq!(selected.exists(), changed_evidence);
    }
}

#[test]
fn trusted_unchanged_container_reuses_units_without_reading_fingerprints() {
    use swamp_core::fs_events::{FsEventsPlan, FsEventsRequest, FsEventsSource};
    struct Unchanged;
    impl FsEventsSource for Unchanged {
        fn replay(&self, _: &FsEventsRequest) -> FsEventsPlan {
            FsEventsPlan::from_live(vec![], 1000, None)
        }
    }
    let (_tmp, root, target) = fixture();
    let store = tempfile::tempdir().unwrap();
    let first = report(&root, store.path());
    let fingerprints = target.join("debug/.fingerprint/fixture-aaa");
    fs::set_permissions(&fingerprints, fs::Permissions::from_mode(0o000)).unwrap();
    // Deliberately supply trusted no-change coverage: this tests the consumer's
    // reuse contract, not the OS event source's ability to notice chmod.
    let next = swamp_core::report::report_full_mode_with_source(
        &root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        false,
        &Unchanged,
    );
    fs::set_permissions(&fingerprints, fs::Permissions::from_mode(0o755)).unwrap();
    let next = next.unwrap();
    assert_eq!(first.nested_artifacts.len(), next.nested_artifacts.len());
    for (old, new) in first.nested_artifacts.iter().zip(&next.nested_artifacts) {
        assert_eq!(
            (&old.id, old.bytes, &old.role),
            (&new.id, new.bytes, &new.role)
        );
    }
}
