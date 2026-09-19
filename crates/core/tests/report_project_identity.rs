//! Issue #32 item 1: project identity by remote. Two separate clones of
//! the same repo (two independent object stores, two independent `.git`
//! directories) must be grouped into ONE project by their shared,
//! normalized `origin` remote URL -- not reported as two projects just
//! because each clone has its own object store.

use std::fs;
use std::path::Path;
use std::process::Command;
use swamp_core::report::{WorktreeKind, report};

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap_or_else(|e| panic!("run git {args:?} in {}: {e}", dir.display()));
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn two_clones_of_the_same_remote_are_one_project_with_two_main_checkouts() {
    let tmp = tempfile::tempdir().expect("tmp root");
    let remote = "https://example.com/fixture-org/shared-repo.git";

    // First clone: a normal checkout plus a linked worktree.
    let clone_a = tmp.path().join("shared-repo");
    fs::create_dir_all(&clone_a).unwrap();
    run_git(&clone_a, &["init", "-q", "-b", "main"]);
    run_git(&clone_a, &["config", "commit.gpgsign", "false"]);
    fs::write(clone_a.join("README.md"), b"a\n").unwrap();
    run_git(&clone_a, &["add", "README.md"]);
    run_git(&clone_a, &["commit", "-q", "-m", "init"]);
    run_git(&clone_a, &["remote", "add", "origin", remote]);

    let linked = tmp.path().join("shared-repo-linked");
    run_git(
        &clone_a,
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "linked-branch",
        ],
    );

    // Second clone: an entirely separate object store, same remote.
    let clone_b = tmp.path().join("shared-repo-second-clone");
    fs::create_dir_all(&clone_b).unwrap();
    run_git(&clone_b, &["init", "-q", "-b", "main"]);
    run_git(&clone_b, &["config", "commit.gpgsign", "false"]);
    fs::write(clone_b.join("README.md"), b"b\n").unwrap();
    run_git(&clone_b, &["add", "README.md"]);
    run_git(&clone_b, &["commit", "-q", "-m", "init"]);
    run_git(&clone_b, &["remote", "add", "origin", remote]);

    let r = report(tmp.path(), None).expect("report should not error");

    let matching: Vec<_> = r
        .projects
        .iter()
        .filter(|p| {
            p.worktrees
                .iter()
                .any(|w| w.path == clone_a || w.path == clone_b || w.path == linked)
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "two clones of the same remote must be exactly one project, got: {:?}",
        r.projects.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    let project = matching[0];
    assert_eq!(
        project.remote.as_deref(),
        Some("example.com/fixture-org/shared-repo")
    );

    let main_count = project
        .worktrees
        .iter()
        .filter(|w| w.kind == WorktreeKind::Main)
        .count();
    let clone_count = project
        .worktrees
        .iter()
        .filter(|w| w.kind == WorktreeKind::Clone)
        .count();
    let linked_count = project
        .worktrees
        .iter()
        .filter(|w| w.kind == WorktreeKind::Linked)
        .count();
    assert_eq!(main_count, 1, "exactly one Main checkout: {project:?}");
    assert_eq!(
        clone_count, 1,
        "the second main checkout must be demoted to Clone: {project:?}"
    );
    assert_eq!(
        linked_count, 1,
        "the linked worktree stays Linked: {project:?}"
    );
    assert_eq!(project.worktrees.len(), 3);
}

#[test]
fn checkouts_with_no_remote_stay_separate_projects() {
    let tmp = tempfile::tempdir().expect("tmp root");

    for name in ["repo-a", "repo-b"] {
        let dir = tmp.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "-q", "-b", "main"]);
        run_git(&dir, &["config", "commit.gpgsign", "false"]);
        fs::write(dir.join("README.md"), b"x\n").unwrap();
        run_git(&dir, &["add", "README.md"]);
        run_git(&dir, &["commit", "-q", "-m", "init"]);
    }

    let r = report(tmp.path(), None).expect("report should not error");
    assert_eq!(
        r.projects.len(),
        2,
        "two unrelated no-remote checkouts must stay two projects: {:?}",
        r.projects.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    for p in &r.projects {
        assert_eq!(p.worktrees.len(), 1);
        assert_eq!(p.worktrees[0].kind, WorktreeKind::Main);
    }
}
