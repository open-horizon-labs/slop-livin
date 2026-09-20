use std::{
    fs,
    path::{Path, PathBuf},
};

use swamp_core::{actions, artifact::ArtifactRole, report::report_full_mode};

fn fixture(
    internal_alias: bool,
    external_alias: bool,
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap().join("repo");
    fs::create_dir_all(root.join("target/debug/incremental/crate-a")).unwrap();
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
    let profile = root.join("target/debug");
    fs::write(profile.join(".cargo-lock"), b"").unwrap();
    let group = profile.join("incremental/crate-a");
    let member = group.join("state.o");
    fs::write(&member, b"shared cargo state").unwrap();
    if internal_alias {
        fs::hard_link(&member, group.join("state-alias.o")).unwrap();
    }
    let external = root.join("target/external-alias.o");
    if external_alias {
        fs::hard_link(&member, &external).unwrap();
    }
    (tmp, root, group, external)
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

fn plan_for(root: &Path, store: &Path, group: &Path) -> actions::Plan {
    let r = report(root, store);
    let unit = r.nested_artifacts.iter().find(|u| u.path == group).unwrap();
    assert_eq!(unit.role, ArtifactRole::Incremental);
    actions::propose(&r, None, &[group.to_path_buf()], "hardlink-test").unwrap()
}

#[test]
fn cleanup_check_reports_shared_storage_warning_on_ready_plan() {
    let (_tmp, root, group, _external) = fixture(false, true);
    let store = tempfile::tempdir().unwrap();
    let r = report(&root, store.path());
    let results =
        swamp_core::cargo_cleanup::check(&r, store.path(), std::slice::from_ref(&group), None, 1)
            .unwrap();
    assert_eq!(results[0].check_status, "ready_for_review");
    assert!(results[0].plan_id.is_some());
    assert!(
        results[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("hardlinks") && warning.contains("unknown"))
    );
}

#[test]
fn external_alias_added_after_proposal_does_not_stale_selected_group() {
    let (_tmp, root, group, external) = fixture(false, false);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    fs::hard_link(group.join("state.o"), &external).unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.outcomes[0].status, "completed", "{result:?}");
    assert!(!group.exists());
    assert!(external.exists());
}

#[test]
fn external_alias_removed_after_proposal_does_not_stale_selected_group() {
    let (_tmp, root, group, external) = fixture(false, true);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    fs::remove_file(&external).unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.outcomes[0].status, "completed", "{result:?}");
    assert!(!group.exists());
}

#[test]
fn internal_and_external_aliases_are_reviewable_and_external_alias_survives() {
    let (_tmp, root, group, external) = fixture(true, true);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    let cargo = plan.units[0].cargo_group.as_ref().unwrap();
    assert!(cargo.shared_storage);
    assert!(cargo.hardlink_members >= 2);
    assert_eq!(cargo.reclaimable_bytes, None);
    assert!(cargo.allocated_bytes > 0);
    actions::save_plan(store.path(), &plan).unwrap();

    let before = fs::read(&external).unwrap();
    let waiting =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(waiting.state, "awaiting-authorization");
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.state, "executed");
    assert_eq!(result.outcomes[0].status, "completed", "{result:?}");
    assert!(!group.exists());
    assert_eq!(fs::read(&external).unwrap(), before);
}

#[test]
fn internal_aliases_only_can_execute_as_one_reviewed_group() {
    let (_tmp, root, group, _external) = fixture(true, false);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    let cargo = plan.units[0].cargo_group.as_ref().unwrap();
    assert!(cargo.shared_storage);
    assert_eq!(cargo.hardlink_members, 2);
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.outcomes[0].status, "completed", "{result:?}");
    assert!(!group.exists());
}

#[test]
fn alias_removal_and_replacement_after_proposal_is_stale() {
    let (_tmp, root, group, _external) = fixture(true, false);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    fs::remove_file(group.join("state-alias.o")).unwrap();
    fs::hard_link(group.join("state.o"), group.join("replacement-alias.o")).unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.outcomes[0].status, "failed");
    assert!(
        result.outcomes[0]
            .cause
            .as_deref()
            .unwrap()
            .contains("stale Cargo member")
    );
    assert!(group.exists());
    assert!(group.join("replacement-alias.o").exists());
}

#[test]
fn external_alias_content_mutation_makes_plan_stale() {
    let (_tmp, root, group, external) = fixture(false, true);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    fs::write(&external, b"changed through retained alias").unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.state, "executed");
    assert_eq!(result.outcomes[0].status, "failed");
    assert!(
        result.outcomes[0]
            .cause
            .as_deref()
            .unwrap()
            .contains("stale Cargo member")
    );
    assert!(group.exists());
    assert_eq!(
        fs::read(&external).unwrap(),
        b"changed through retained alias"
    );
}

#[test]
fn unshared_group_reports_allocation_and_trash_not_promised_free_space() {
    let (_tmp, root, group, _external) = fixture(false, false);
    let store = tempfile::tempdir().unwrap();
    let plan = plan_for(&root, store.path(), &group);
    let cargo = plan.units[0].cargo_group.as_ref().unwrap();
    assert!(!cargo.shared_storage);
    assert_eq!(cargo.hardlink_members, 0);
    assert_eq!(cargo.reclaimable_bytes, None);
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:test").unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "test", &store.path().join("trash"))
            .unwrap();
    assert_eq!(result.outcomes[0].status, "completed");
    assert_eq!(result.trashed_bytes, cargo.allocated_bytes);
    assert_eq!(result.removed_permanently_bytes, 0);
}
