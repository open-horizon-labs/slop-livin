use swamp_core::{
    cargo_artifacts::inspect_target,
    cargo_cleanup::{guidance, recommendation},
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
