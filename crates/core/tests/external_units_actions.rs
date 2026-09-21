//! #43 acceptance: external units are inspection-only. A plan can name
//! one, but `execute` refuses it unconditionally with an explicit "no
//! supported selective action for <category>" reason -- never a generic
//! recursive delete, and never authorized merely because a grant covers
//! the plan.

use std::collections::HashMap;
use std::fs;

use swamp_core::actions;
use swamp_core::external::discover_and_measure;
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{ScanConfig, resolve_effective_scope};

fn write_pattern(path: &std::path::Path, bytes: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes as usize]).unwrap();
}

#[test]
fn external_units_are_inspection_only_and_execution_refuses() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 5_000);

    let mut env_vars: HashMap<String, String> = HashMap::new();
    env_vars.insert("CARGO_HOME".into(), cargo_home.display().to_string());
    let env = Environment::fixture(home.path().to_path_buf(), env_vars, Platform::MacOS);
    let registry = Registry::with_builtins();
    let cfg = ScanConfig {
        defaults: false,
        include: Vec::new(),
        exclude: Vec::new(),
        disabled_detectors: vec!["rustup".into(), "homebrew".into()],
    };
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();
    let units = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    let target = units
        .iter()
        .find(|u| u.detector_id == "cargo-home")
        .expect("cargo home unit present")
        .clone();

    // A plan CAN name it (identification is not blocked)...
    let plan =
        actions::propose_external(&units, std::slice::from_ref(&target.path), "test").unwrap();
    assert_eq!(plan.units.len(), 1);
    assert!(plan.units[0].external_category.is_some());

    actions::save_plan(store.path(), &plan).unwrap();
    // ...and a human can even approve the plan (approval is not the same
    // as a supported action existing)...
    actions::approve(store.path(), &plan.id, "human:test").unwrap();

    let result = actions::execute(store.path(), &plan.id, "human:test").unwrap();
    assert_eq!(result.outcomes.len(), 1);
    let outcome = &result.outcomes[0];
    assert_eq!(outcome.status, "refused");
    let cause = outcome.cause.as_deref().unwrap_or_default();
    assert!(cause.contains("no supported selective action"), "{cause}");
    assert!(cause.contains("Installation"), "{cause}");

    // Nothing was moved: the fixture storage is untouched.
    assert!(cargo_home.join("bin/cargo").exists());
    assert_eq!(result.trashed_bytes, 0);
    assert_eq!(result.removed_permanently_bytes, 0);
}

#[test]
fn propose_external_with_no_matching_path_is_a_visible_error() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 1_000);
    let mut env_vars: HashMap<String, String> = HashMap::new();
    env_vars.insert("CARGO_HOME".into(), cargo_home.display().to_string());
    let env = Environment::fixture(home.path().to_path_buf(), env_vars, Platform::MacOS);
    let registry = Registry::with_builtins();
    let cfg = ScanConfig {
        defaults: false,
        include: Vec::new(),
        exclude: Vec::new(),
        disabled_detectors: vec!["rustup".into(), "homebrew".into()],
    };
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();
    let units = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();

    let err =
        actions::propose_external(&units, &[home.path().join("not-a-unit")], "test").unwrap_err();
    assert!(err.to_string().contains("no external unit matched"));
}
