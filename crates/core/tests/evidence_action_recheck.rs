//! #61: proposals carry decision evidence, and `execute` re-takes
//! current-use evidence fresh rather than trusting the proposal-time
//! snapshot -- so a fact that changes between propose and execute (here,
//! occupancy) is still caught. Mirrors the existing real-lsof pattern in
//! `agent_units_actions.rs::active_session_transcript_refuses_at_proposal_and_execution`,
//! extended to the generic (non-agent) unit path.

mod fixture;

use std::fs;
use std::path::Path;
use swamp_core::actions::{self, approve, execute_with_trash, propose};
use swamp_core::report::{ArtifactKind, report_full_mode};

/// #61 table-driven check: every `StorageCategory` (the full detector
/// catalog, including ones added after this test was written) produces
/// an external unit whose plan `execute` refuses unconditionally --
/// never a path that falls through to the generic recursive-delete
/// rename. A new detector category can never silently acquire generic
/// deletion capability just by existing.
#[test]
fn every_external_storage_category_is_inspection_only_never_recursive_delete() {
    use swamp_core::locations::{Provenance, StorageCategory};

    let categories = [
        StorageCategory::Installation,
        StorageCategory::Downloads,
        StorageCategory::Cache,
        StorageCategory::LocalState,
        StorageCategory::Environments,
        StorageCategory::BuildOutput,
        StorageCategory::Models,
        StorageCategory::Unclassified,
    ];
    for category in categories {
        let unit = swamp_core::external::ExternalUnit {
            detector_id: "synthetic".into(),
            detector_name: "Synthetic".into(),
            category,
            provenance: Provenance::BuiltinConvention,
            path: "/tmp/does-not-matter".into(),
            bytes: 1,
            mtime_max: 0,
            hardlinked: false,
            growth_bytes: None,
            regrowth_count: 0,
            observed_at: 0,
            consumers: Vec::new(),
            note: None,
            evidence: Vec::new(),
        };
        let plan_unit = actions::unit_from_external(&unit);
        assert!(
            plan_unit.external_category.is_some(),
            "{category:?} must carry external_category so execute refuses it unconditionally"
        );
        assert_eq!(plan_unit.verb, "inspect");
    }
}

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

#[test]
fn proposal_carries_evidence_including_activity_reclaimability_and_current_use() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let plan = propose(&r, None, &[], "test").expect("plan");
    let build_output_unit = plan
        .units
        .iter()
        .find(|u| u.kind == ArtifactKind::BuildOutput)
        .expect("a build-output unit in the plan");

    // The tempting shortcut this rejects: an empty evidence list on a
    // proposal, or one that names only current-use and drops the
    // report row's own Activity/Reclaimability/Recovery facts.
    assert!(
        !build_output_unit.evidence.is_empty(),
        "proposal must carry evidence, not just bytes/kind"
    );
    let kinds: std::collections::HashSet<_> =
        build_output_unit.evidence.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&swamp_core::evidence::FactKind::Activity));
    assert!(kinds.contains(&swamp_core::evidence::FactKind::Reclaimability));
    assert!(kinds.contains(&swamp_core::evidence::FactKind::CurrentUse));
}

#[test]
fn occupancy_change_between_propose_and_execute_refuses_execution() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let plan = propose(&r, None, std::slice::from_ref(&fx.target_dir), "test").expect("plan");
    assert_eq!(plan.units.len(), 1, "exactly the requested unit");
    assert!(
        plan.units[0].evidence.iter().any(|e| {
            e.kind == swamp_core::evidence::FactKind::CurrentUse
                && matches!(
                    e.status,
                    swamp_core::evidence::FactStatus::Known(swamp_core::evidence::FactValue::Bool(
                        false
                    ))
                )
        }),
        "at proposal time, before anything held it open, current-use must read known-false"
    );

    actions::save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:test").unwrap();

    // Mutates the fact between propose and execute: hold the exact unit
    // path open in this process, the same real-lsof mechanism the
    // existing agent-storage test uses (no mocking).
    let _held_open = fs::File::open(&fx.target_dir).expect("open target dir to simulate new use");

    let trash_dir = tempfile::tempdir().unwrap();
    let result =
        execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path()).unwrap();
    let outcome = &result.outcomes[0];
    assert_ne!(
        outcome.status, "completed",
        "must refuse, not silently delete an actively-open unit"
    );
    assert!(
        outcome
            .cause
            .as_deref()
            .unwrap_or_default()
            .contains("open file handle"),
        "cause should name what changed: {:?}",
        outcome.cause
    );
    assert!(
        fx.target_dir.exists(),
        "the unit must remain in place, never moved to Trash while occupied"
    );
}

#[test]
fn without_new_occupancy_the_same_plan_still_executes_normally() {
    // Control: confirm the new recheck does not break the ordinary
    // succeeding path when nothing opened the unit after proposal.
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let plan = propose(&r, None, std::slice::from_ref(&fx.target_dir), "test").expect("plan");
    actions::save_plan(store.path(), &plan).unwrap();
    approve(store.path(), &plan.id, "human:test").unwrap();

    let trash_dir = tempfile::tempdir().unwrap();
    let result =
        execute_with_trash(store.path(), &plan.id, "human:test", trash_dir.path()).unwrap();
    let outcome = &result.outcomes[0];
    assert_eq!(outcome.status, "completed", "cause: {:?}", outcome.cause);
    assert!(!fx.target_dir.exists());
}

/// #60: human keep/protect intent, extended beyond agent-storage units
/// to an ordinary filesystem artifact row. A scanned project file or an
/// agent observation must never be able to add or remove this
/// protection -- only the explicit `protected` list this test builds
/// by hand (standing in for `agents::protect_add`, itself reachable
/// only from the CLI's `protect` subcommand) does.
#[test]
fn human_protected_ordinary_artifact_is_refused_at_proposal_not_silently_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    let protected = vec![fx.target_dir.clone()];
    let plan = actions::propose_checking_protection(
        &r,
        None,
        std::slice::from_ref(&fx.target_dir),
        "test",
        &protected,
    );
    let err = plan.unwrap_err();
    assert!(err.to_string().contains("human-protected"), "{err}");

    // The tempting shortcut this rejects: silently omitting the
    // protected path from the plan instead of naming it in `refused`.
    let unfiltered = actions::propose(&r, None, &[], "test").expect("broad plan");
    let plan = actions::propose_checking_protection(&r, None, &[], "test", &protected)
        .expect("other unprotected units remain plannable");
    assert!(
        plan.refused
            .iter()
            .any(|r| r.path == fx.target_dir && r.cause.contains("human-protected"))
    );
    assert!(
        plan.units.len() < unfiltered.units.len(),
        "the protected unit must be removed from the plannable set"
    );
    assert!(!plan.units.iter().any(|u| u.path == fx.target_dir));
}

/// The integration owner's 2026-09-21 mutation check, as a runtime test.
///
/// Mutating protection to one direction only -- `candidate.starts_with(p)`,
/// dropping `p.starts_with(candidate)` -- passed the
/// `protection_fails_closed` audit *and* every runtime test at the time.
/// The audit inspected `agents::protection_conflict`, while this path,
/// `actions::propose_checking_protection` for **ordinary** filesystem
/// rows, went through a second predicate (`agents::is_human_protected`,
/// since deleted). Nothing proposed an ordinary directory that
/// *contained* a protected descendant, so the surviving direction was
/// never exercised here.
///
/// This test is that exact case. Protecting one file inside an artifact
/// directory must make the directory itself unproposable: removing
/// `target/` destroys `target/keep-this.txt`, which is precisely what
/// the human asked to keep.
#[test]
fn ordinary_unit_containing_protected_descendant_is_refused_at_proposal() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();

    // A file the human protects, *inside* the artifact directory.
    let keep = fx.target_dir.join("keep-this.txt");
    fs::write(&keep, b"work I asked you to keep").unwrap();
    let r = report_for(&fx.root, store.path());
    let protected = vec![keep.clone()];

    // Naming the containing directory explicitly must fail.
    let err = actions::propose_checking_protection(
        &r,
        None,
        std::slice::from_ref(&fx.target_dir),
        "test",
        &protected,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("human-protected"),
        "removing a directory that contains a protected file must be refused: {err}"
    );

    // And a broad proposal must refuse it by name rather than quietly
    // including it.
    let plan = actions::propose_checking_protection(&r, None, &[], "test", &protected)
        .expect("other units remain plannable");
    assert!(
        plan.refused
            .iter()
            .any(|x| x.path == fx.target_dir && x.cause.contains("human-protected")),
        "the containing directory must appear in `refused` with a named cause: {:?}",
        plan.refused
    );
    assert!(
        !plan.units.iter().any(|u| u.path == fx.target_dir),
        "the containing directory must not be plannable"
    );
    assert!(keep.exists(), "nothing may have been touched at proposal");
}

/// The other direction, in the same place, so a future one-directional
/// mutation fails whichever half it drops.
#[test]
fn ordinary_unit_beneath_a_protected_ancestor_is_refused_at_proposal() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, store.path());

    // Protect an ancestor of the artifact row.
    let protected = vec![fx.checkout.clone()];
    let err = actions::propose_checking_protection(
        &r,
        None,
        std::slice::from_ref(&fx.target_dir),
        "test",
        &protected,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("human-protected"),
        "a unit beneath a protected path must be refused: {err}"
    );
}
