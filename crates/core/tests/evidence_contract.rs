//! #53 end-to-end acceptance: the evidence contract is populated
//! through the real report pipeline (not just exercised by
//! `evidence.rs`'s own unit tests), and refreshing it never manufactures
//! a byte-history delta.

mod fixture;

use std::path::Path;
use swamp_core::evidence::{Evidence, FactKind};
use swamp_core::report::{ArtifactKind, report_full_mode};

fn report_for(root: &Path, docker_facts: &Path, store: &Path) -> swamp_core::Report {
    report_full_mode(
        root,
        Some(docker_facts),
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

fn all_evidence(r: &swamp_core::Report) -> Vec<&Evidence> {
    r.projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|w| w.artifacts.iter())
        .flat_map(|a| a.evidence.iter())
        .collect()
}

/// A real fixture report (build output, dependency tree, a Docker
/// join -- see `crates/core/tests/fixture`) carries at least one real
/// fact of every `FactKind` the contract distinguishes, populated by
/// the actual pipeline (`report::attach_decision_evidence`,
/// `actions::propose`'s current-use snapshot), not merely declared and
/// left unused.
#[test]
fn fixture_report_carries_at_least_one_real_fact_of_each_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, &fx.docker_facts, store.path());

    let evidence = all_evidence(&r);
    assert!(!evidence.is_empty(), "report produced no evidence at all");

    assert!(
        evidence.iter().any(|e| e.kind == FactKind::Activity),
        "no Activity fact in a real report"
    );
    assert!(
        evidence.iter().any(|e| e.kind == FactKind::Reclaimability),
        "no Reclaimability fact in a real report"
    );
    assert!(
        evidence.iter().any(|e| e.kind == FactKind::Recovery),
        "no Recovery fact in a real report"
    );
    // Consumer evidence flows through the Docker join site
    // (report.rs's join_one match) and external::discover_and_measure;
    // this fixture's Docker facts exercise the former.
    assert!(
        evidence.iter().any(|e| e.kind == FactKind::Consumer),
        "no Consumer fact in a real report (expected from the Docker join)"
    );

    // CurrentUse is deliberately not attached during a passive report
    // (it is short-lived and only taken fresh at proposal/action
    // boundaries -- see actions::plan_unit_evidence); confirm that
    // seam produces a real CurrentUse fact instead.
    let plan = swamp_core::actions::propose(&r, None, &[], "test").expect("plan");
    let plan_evidence: Vec<&Evidence> = plan.units.iter().flat_map(|u| u.evidence.iter()).collect();
    assert!(
        plan_evidence.iter().any(|e| e.kind == FactKind::CurrentUse),
        "no CurrentUse fact anywhere in the proposal path"
    );
}

/// Attaching decision evidence is a pure post-pass over already-known
/// numbers: it must never change `bytes`, `growth_bytes` or
/// `regrowth_count` on any row, and calling it again (e.g. a second
/// `report` with nothing changed on disk) must not create a new
/// byte-history observation. This is the literal "tempting shortcut"
/// #53 names: evidence collection quietly writing to the growth store.
#[test]
fn refreshing_evidence_never_writes_byte_history_delta() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();

    fn snapshot(
        r: &swamp_core::Report,
    ) -> Vec<(ArtifactKind, std::path::PathBuf, u64, Option<i64>, u32)> {
        let mut rows: Vec<_> = r
            .projects
            .iter()
            .flat_map(|p| p.worktrees.iter())
            .flat_map(|w| w.artifacts.iter())
            .map(|a| {
                (
                    a.kind.clone(),
                    a.path.clone(),
                    a.bytes,
                    a.growth_bytes,
                    a.regrowth_count,
                )
            })
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1));
        rows
    }

    // The very first observation establishes the growth-store baseline
    // (no prior history: `growth_bytes` is legitimately `None`, not yet
    // comparable to a later call). Discard it as a warm-up, then compare
    // two *subsequent* observations -- both past that one-time
    // None -> Some(0) transition -- with nothing changed on disk between
    // them.
    let _warm_up = report_for(&fx.root, &fx.docker_facts, store.path());
    let second = report_for(&fx.root, &fx.docker_facts, store.path());
    let third = report_for(&fx.root, &fx.docker_facts, store.path());

    assert_eq!(
        snapshot(&second),
        snapshot(&third),
        "byte/growth/regrowth accounting must be identical across two \
         observations with nothing changed on disk, evidence or not"
    );

    // A direct, pure-function guarantee independent of the store: calling
    // attach_decision_evidence again on an already-annotated report must
    // not touch bytes/growth_bytes/regrowth_count either.
    let mut replay = third.clone();
    let before: Vec<(u64, Option<i64>, u32)> = replay
        .projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|w| w.artifacts.iter())
        .map(|a| (a.bytes, a.growth_bytes, a.regrowth_count))
        .collect();
    swamp_core::report::attach_decision_evidence(&mut replay);
    let after: Vec<(u64, Option<i64>, u32)> = replay
        .projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|w| w.artifacts.iter())
        .map(|a| (a.bytes, a.growth_bytes, a.regrowth_count))
        .collect();
    assert_eq!(before, after);
}

/// Missing/stale/conflicting facts round-trip through JSON exactly as
/// `evidence.rs`'s own unit tests assert in isolation -- this exercises
/// the same guarantee through a real report row's serialized form, the
/// shape a CLI/skill caller actually receives.
#[test]
fn missing_stale_and_conflicting_facts_round_trip_through_a_real_report() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture::build(tmp.path());
    let store = tempfile::tempdir().unwrap();
    let r = report_for(&fx.root, &fx.docker_facts, store.path());

    let json = swamp_core::report::to_json(&r).expect("serialize report");
    let back: swamp_core::Report = serde_json::from_str(&json).expect("deserialize report");

    let original = all_evidence(&r);
    let restored = all_evidence(&back);
    assert_eq!(original.len(), restored.len());
    for (a, b) in original.iter().zip(restored.iter()) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.status, b.status);
        assert_eq!(a.source, b.source);
    }

    // At least one Unknown-status fact exists (a cache/dependency row
    // this pass could not fully source) and survives the round trip
    // rather than being coerced into a false "known" value.
    let has_unknown = restored
        .iter()
        .any(|e| matches!(e.status, swamp_core::evidence::FactStatus::Unknown { .. }));
    assert!(
        has_unknown,
        "expected at least one Unknown fact in a real report"
    );
}
