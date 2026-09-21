use swamp_core::{
    cargo_artifacts::inspect_target,
    cargo_cleanup::{cleanup_order, guidance, guidance_at, recommendation},
};

#[test]
fn recommendations_explain_tradeoffs_without_inventing_disuse() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target");
    std::fs::create_dir_all(target.join("debug/incremental/crate-a")).unwrap();
    std::fs::write(target.join("debug/incremental/crate-a/state"), b"data").unwrap();
    let report = inspect_target(&target, Some(&target));
    let mut unit = report
        .units
        .iter()
        .find(|u| u.path.ends_with("crate-a"))
        .unwrap()
        .clone();
    assert_eq!(guidance(&unit).check_status, "unchecked");
    assert!(recommendation(&unit).0.contains("Start here"));
    assert!(recommendation(&unit).1.contains("slower"));
    let now = 30 * 86400;
    unit.mtime_max = now - 21 * 86400;
    let advice = guidance_at(&unit, now);
    assert!(advice.recommendation.contains("Cleanup candidate"));
    assert!(advice.recommendation.contains("21d ago"));
    assert_eq!(advice.modified_age_secs, Some(21 * 86400));
    let mut recent = unit.clone();
    recent.mtime_max = now - 3600;
    recent.bytes = unit.bytes + 1_000_000;
    assert!(
        cleanup_order(&unit, &recent, now).is_lt(),
        "older beats larger"
    );
    assert!(guidance_at(&recent, now).recommendation.contains("1h ago"));
    assert_eq!(
        guidance_at(&recent, now).check_status,
        "unchecked",
        "recent remains reviewable"
    );
    for timestamp in [0, now + 1] {
        let mut unknown = unit.clone();
        unknown.mtime_max = timestamp;
        assert_eq!(guidance_at(&unknown, now).modified_age_secs, None);
        assert!(cleanup_order(&recent, &unknown, now).is_lt());
    }
    let before = recommendation(&unit);
    unit.mtime_max = 1;
    assert_eq!(
        recommendation(&unit),
        before,
        "age is not evidence of disuse"
    );
    unit.coverage.complete = false;
    assert_eq!(recommendation(&unit).0, "Inspect coverage");
    assert_eq!(guidance(&unit).check_status, "blocked");
}
