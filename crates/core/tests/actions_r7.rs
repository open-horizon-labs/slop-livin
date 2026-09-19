//! R7 (#26): the agent action layer as a consumer of report rows.
//! Plans name project/worktree/kind/evidence/recovery; only a human at the
//! CLI writes grants; the sink re-derives; Trash; ledger; measured space.

mod fixture;

use std::fs;
use std::path::{Path, PathBuf};
use swamp_core::actions::{
    self, PlanStatus, add_standing_grant, approve, execute_with_trash, list_grants, load_plan,
    propose, save_plan,
};
use swamp_core::report::{ArtifactKind, report_full_mode};

fn report_for(root: &Path, store: &Path) -> swamp_core::Report {
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
    .expect("report")
}

fn set_trash(dir: &Path) -> PathBuf {
    let t = dir.join("trash");
    fs::create_dir_all(&t).unwrap();
    t
}

fn row_path(r: &swamp_core::Report, kind: ArtifactKind, ends: &str) -> PathBuf {
    r.projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|w| w.artifacts.iter())
        .find(|a| a.kind == kind && a.path.display().to_string().ends_with(ends))
        .map(|a| a.path.clone())
        .unwrap_or_else(|| panic!("no {kind:?} row ending in {ends}"))
}

#[test]
fn propose_plans_any_path_and_names_evidence_and_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let plan = propose(&r, None, &[], "test").expect("plan");
    assert!(!plan.units.is_empty());
    for u in &plan.units {
        assert!(
            !matches!(
                u.kind,
                ArtifactKind::DockerImage
                    | ArtifactKind::DockerBuildCache
                    | ArtifactKind::DockerVolume
            ),
            "docker objects are never planned, got {:?}",
            u.kind
        );
        assert!(!u.project.is_empty() && !u.worktree_id.is_empty());
        assert!(!u.recovery.is_empty());
        assert_eq!(u.verb, "delete");
    }
    // Anything with a path is plannable — a .git store and a Source tree
    // included — carrying the warning a human weighs instead of a refusal.
    let git = row_path(&r, ArtifactKind::Git, "/.git");
    let source = row_path(&r, ArtifactKind::Source, "/checkout");
    let plan = propose(&r, None, &[git.clone(), source.clone()], "test").unwrap();
    assert_eq!(plan.units.len(), 2);
    let git_unit = plan.units.iter().find(|u| u.path == git).unwrap();
    assert!(
        git_unit
            .warnings
            .iter()
            .any(|w| w.contains("git object store")),
        "{:?}",
        git_unit.warnings
    );
    // The checkout root path is the whole checkout: the archive verb, with
    // the checkout's facts (the fixture has untracked loose files and no
    // remote) as warnings rather than a refusal.
    let src_unit = plan.units.iter().find(|u| u.path == source).unwrap();
    assert_eq!(src_unit.verb, "archive");
    assert!(
        src_unit
            .warnings
            .iter()
            .any(|w| w.contains("untracked") || w.contains("no remote")),
        "{:?}",
        src_unit.warnings
    );
    // A path that is not in the report at all is refused, naming why.
    let err = propose(
        &r,
        None,
        &[PathBuf::from("/nonexistent/prune-docker")],
        "test",
    )
    .expect_err("no such row");
    assert!(
        err.to_string().contains("not a path in this report"),
        "{err}"
    );
    // Mixed: one plannable + one unknown -> plan with a refused entry.
    let nm = row_path(&r, ArtifactKind::DependencyTree, "/node_modules");
    let plan = propose(&r, None, &[nm.clone(), PathBuf::from("/nope")], "test").unwrap();
    assert_eq!(plan.units.len(), 1);
    assert_eq!(plan.refused.len(), 1);
}

#[test]
fn a_worktree_path_plans_as_remove_worktree_and_a_checkout_as_archive_with_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());
    let (checkout, linked) = {
        let p = r
            .projects
            .iter()
            .find(|p| p.worktrees.len() >= 2)
            .expect("fixture project with a linked worktree");
        let main = p
            .worktrees
            .iter()
            .find(|w| w.kind == swamp_core::report::WorktreeKind::Main)
            .unwrap();
        let link = p
            .worktrees
            .iter()
            .find(|w| w.kind == swamp_core::report::WorktreeKind::Linked)
            .unwrap();
        (main.path.clone(), link.path.clone())
    };
    let plan = propose(&r, None, &[checkout.clone(), linked.clone()], "agent").unwrap();
    let c = plan.units.iter().find(|u| u.path == checkout).unwrap();
    let l = plan.units.iter().find(|u| u.path == linked).unwrap();
    assert_eq!(c.verb, "archive");
    assert_eq!(l.verb, "remove-worktree");
    assert!(c.bytes > 0 && l.bytes > 0);
    // The fixture checkout has untracked loose files and a remote: the
    // warnings say so, and nothing here is a refusal.
    assert!(
        c.warnings.iter().any(|w| w.contains("untracked"))
            || c.warnings.iter().any(|w| w.contains("no remote")),
        "{:?}",
        c.warnings
    );
    assert!(plan.refused.is_empty());
}

#[test]
fn execute_without_grant_is_awaiting_authorization_and_deletes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let trash = set_trash(store.path());
    let r = report_for(&fx.root, store.path());
    let nm = row_path(&r, ArtifactKind::DependencyTree, "/node_modules");
    let plan = propose(&r, None, std::slice::from_ref(&nm), "agent:test").unwrap();
    save_plan(store.path(), &plan).unwrap();

    let res = execute_with_trash(store.path(), &plan.id, "agent:test", &trash).unwrap();
    assert_eq!(res.state, "awaiting-authorization");
    assert!(
        res.next_step
            .unwrap()
            .contains(&actions::approve_command(&plan.id))
    );
    assert!(nm.exists(), "nothing may be touched without a grant");
    assert!(res.outcomes.is_empty());
    assert_eq!(
        load_plan(store.path(), &plan.id).unwrap().status,
        PlanStatus::Proposed
    );
}

#[test]
fn approve_then_execute_trashes_records_ledger_and_is_single_use() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let trash = set_trash(store.path());
    let r = report_for(&fx.root, store.path());
    let nm = row_path(&r, ArtifactKind::DependencyTree, "/node_modules");
    let plan = propose(&r, None, std::slice::from_ref(&nm), "agent:test").unwrap();
    save_plan(store.path(), &plan).unwrap();

    let g = approve(store.path(), &plan.id, "human:test").unwrap();
    assert_eq!(g.plan_id.as_deref(), Some(plan.id.as_str()));

    let res = execute_with_trash(store.path(), &plan.id, "agent:test", &trash).unwrap();
    assert_eq!(res.state, "executed", "{res:?}");
    assert_eq!(res.outcomes.len(), 1);
    assert_eq!(
        res.outcomes[0].status, "completed",
        "{:?}",
        res.outcomes[0].cause
    );
    assert!(!nm.exists(), "artifact moved to Trash");
    let dest = res.outcomes[0].recovery_location.clone().unwrap();
    assert!(
        dest.starts_with(&trash) && dest.exists(),
        "recoverable from {}",
        dest.display()
    );
    assert_eq!(res.trashed_bytes, plan.planned_bytes());

    // Ledger: actor and evidence recorded, independent of the index.
    let ledger = swamp_core::ledger::Ledger::open(store.path().join("ledger.jsonl")).unwrap();
    let recs = ledger.all().unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].actor, "agent:test");
    assert_eq!(recs[0].grant_id, g.id);
    assert_eq!(recs[0].evidence["kind"], "DependencyTree");
    assert!(recs[0].evidence["project"].as_str().is_some());

    // Single use: a second execute does nothing.
    let again = execute_with_trash(store.path(), &plan.id, "agent:test", &trash).unwrap();
    assert_eq!(again.state, "already-executed");
    // The one-shot grant's budget is spent.
    let gs = list_grants(store.path()).unwrap();
    assert_eq!(gs[0].spent_bytes, plan.planned_bytes());
}

#[test]
fn stale_unique_bytes_do_not_spend_a_standing_grant() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let trash = set_trash(store.path());
    let mut r = report_for(&fx.root, store.path());
    let nm = row_path(&r, ArtifactKind::DependencyTree, "/node_modules");
    for a in r
        .projects
        .iter_mut()
        .flat_map(|p| &mut p.worktrees)
        .flat_map(|w| &mut w.artifacts)
    {
        if a.path == nm {
            a.dedup_stale = true;
        }
    }
    let plan = propose(&r, None, std::slice::from_ref(&nm), "test").unwrap();
    assert!(plan.units[0].dedup_stale);
    assert!(plan.units[0].warnings.iter().any(|w| w.contains("stale")));
    save_plan(store.path(), &plan).unwrap();
    add_standing_grant(
        store.path(),
        "kind:DependencyTree",
        1 << 30,
        None,
        3600,
        "human:test",
    )
    .unwrap();
    let result = execute_with_trash(store.path(), &plan.id, "test", &trash).unwrap();
    assert_eq!(result.state, "awaiting-authorization");
    assert!(nm.exists());
    assert_eq!(list_grants(store.path()).unwrap()[0].spent_bytes, 0);
}

#[test]
fn standing_grant_covers_by_predicate_and_budget_refuses_overrun() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let trash = set_trash(store.path());
    let r = report_for(&fx.root, store.path());
    // grant predicates must be about the unit
    assert!(
        add_standing_grant(store.path(), "growth > 1MB in 7d", 1 << 30, None, 3600, "h").is_err()
    );
    assert!(add_standing_grant(store.path(), "", 1 << 30, None, 3600, "h").is_err());
    assert!(add_standing_grant(store.path(), "kind:BuildOutput", 0, None, 3600, "h").is_err());

    // Budget smaller than the target row: refused with the budget fact.
    let target = row_path(&r, ArtifactKind::BuildOutput, "/checkout/target");
    let target_bytes = r
        .projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|w| w.artifacts.iter())
        .find(|a| a.path == target)
        .unwrap()
        .bytes;
    add_standing_grant(
        store.path(),
        "kind:BuildOutput",
        target_bytes - 1,
        None,
        3600,
        "human:test",
    )
    .unwrap();
    let plan = propose(&r, None, std::slice::from_ref(&target), "agent:test").unwrap();
    save_plan(store.path(), &plan).unwrap();
    let res = execute_with_trash(store.path(), &plan.id, "agent:test", &trash).unwrap();
    assert_eq!(res.state, "executed");
    assert_eq!(res.outcomes[0].status, "refused");
    assert!(
        res.outcomes[0].cause.as_ref().unwrap().contains("budget"),
        "{:?}",
        res.outcomes[0].cause
    );
    assert!(target.exists());

    // A kind the grant does not name is not covered even with budget.
    let nm = row_path(&r, ArtifactKind::DependencyTree, "/node_modules");
    let plan2 = propose(&r, None, std::slice::from_ref(&nm), "agent:test").unwrap();
    save_plan(store.path(), &plan2).unwrap();
    let res2 = execute_with_trash(store.path(), &plan2.id, "agent:test", &trash).unwrap();
    assert_eq!(res2.state, "awaiting-authorization");
    assert!(nm.exists());

    // Adequate standing grant for DependencyTree in this project: executes.
    let project = plan2.units[0].project.clone();
    add_standing_grant(
        store.path(),
        &format!("kind:DependencyTree project:{project}"),
        1 << 40,
        Some(1),
        3600,
        "human:test",
    )
    .unwrap();
    let plan3 = propose(&r, None, std::slice::from_ref(&nm), "agent:test").unwrap();
    save_plan(store.path(), &plan3).unwrap();
    let res3 = execute_with_trash(store.path(), &plan3.id, "agent:test", &trash).unwrap();
    assert_eq!(
        res3.outcomes[0].status, "completed",
        "{:?}",
        res3.outcomes[0].cause
    );
    assert!(!nm.exists());
}

#[test]
fn activity_after_the_plan_refuses_at_the_sink() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let trash = set_trash(store.path());
    let r = report_for(&fx.root, store.path());
    let dist = row_path(&r, ArtifactKind::BuildOutput, "/checkout/dist");
    let mut plan = propose(&r, None, std::slice::from_ref(&dist), "agent:test").unwrap();
    // Make the plan look older than the write below.
    plan.created_at -= 10;
    save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:test").unwrap();
    fs::write(dist.join("fresh.bin"), b"activity").unwrap();
    let res = execute_with_trash(store.path(), &plan.id, "agent:test", &trash).unwrap();
    assert_eq!(res.outcomes[0].status, "refused");
    assert!(
        res.outcomes[0]
            .cause
            .as_ref()
            .unwrap()
            .contains("activity changed since plan")
    );
    assert!(dist.exists());
}

#[test]
fn expired_plan_cannot_be_approved_or_executed() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let trash = set_trash(store.path());
    let r = report_for(&fx.root, store.path());
    let dist = row_path(&r, ArtifactKind::BuildOutput, "/checkout/dist");
    let mut plan = propose(&r, None, std::slice::from_ref(&dist), "agent:test").unwrap();
    plan.expires_at = plan.created_at - 1;
    save_plan(store.path(), &plan).unwrap();
    assert!(approve(store.path(), &plan.id, "human:test").is_err());
    let res = execute_with_trash(store.path(), &plan.id, "agent:test", &trash).unwrap();
    assert_eq!(res.state, "expired");
    assert!(dist.exists());
}
