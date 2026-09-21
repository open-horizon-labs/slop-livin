//! #43 adversarial tests: external/shared storage units, modeled and
//! persisted without a fabricated project owner.

use std::collections::HashMap;
use std::fs;

use swamp_core::external::{
    ExternalUnit, associate_consumer, discover_and_measure, dissociate_consumer, total_bytes,
    unit_key,
};
use swamp_core::locations::{Environment, Platform, Registry, StorageCategory};
use swamp_core::scope::{ScanConfig, resolve_effective_scope};

fn fixture_env(home: &std::path::Path, extra: &[(&str, &str)]) -> Environment {
    let mut env: HashMap<String, String> = HashMap::new();
    for (k, v) in extra {
        env.insert((*k).to_string(), (*v).to_string());
    }
    Environment::fixture(home.to_path_buf(), env, Platform::MacOS)
}

fn only_cargo_home_config() -> ScanConfig {
    ScanConfig {
        defaults: false,
        include: Vec::new(),
        exclude: Vec::new(),
        disabled_detectors: vec!["rustup".into(), "homebrew".into()],
    }
}

fn write_pattern(path: &std::path::Path, bytes: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes as usize]).unwrap();
}

fn cargo_home_unit(units: &[ExternalUnit]) -> &ExternalUnit {
    units
        .iter()
        .find(|u| u.detector_id == "cargo-home" && u.category == StorageCategory::Installation)
        .expect("cargo home's own unit (not registry/git) is present")
}

/// An external root with no containing project (there is none -- a
/// manager home is not inside any checkout) is measured as one unit with
/// a real category and provenance, not folded into a project or dropped.
#[test]
fn external_only_root_is_measured_as_one_unit() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 1_000);
    write_pattern(&cargo_home.join("config.toml"), 200);

    let env = fixture_env(
        home.path(),
        &[("CARGO_HOME", &cargo_home.display().to_string())],
    );
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_cargo_home_config(), &[], &registry, 1);

    let store = tempfile::tempdir().unwrap();
    let units = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    let unit = cargo_home_unit(&units);
    // Allocated bytes (disk blocks), not logical file size -- a real
    // measurement, so only a lower/upper sanity bound on the two small
    // files written above, not an exact byte count.
    assert!(
        unit.bytes >= 1_200 && unit.bytes < 100_000,
        "{}",
        unit.bytes
    );
    assert_eq!(unit.category, StorageCategory::Installation);
    assert!(unit.consumers.is_empty(), "no consumer declared yet");
    // First-ever observation: unknown baseline, never a synthetic zero.
    assert_eq!(unit.growth_bytes, None);
}

/// Zero consumers is the ordinary starting state, distinct from an error
/// or an omitted field.
#[test]
fn empty_consumer_set_is_empty_not_missing() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 500);
    let env = fixture_env(
        home.path(),
        &[("CARGO_HOME", &cargo_home.display().to_string())],
    );
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_cargo_home_config(), &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();
    let units = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    assert_eq!(cargo_home_unit(&units).consumers, Vec::new());
}

/// Two declared consumers of the same unit: the unit is still counted
/// once in totals, and both consumers are visible (not overwritten).
#[test]
fn shared_consumers_are_counted_once_in_totals() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 3_000);
    let env = fixture_env(
        home.path(),
        &[("CARGO_HOME", &cargo_home.display().to_string())],
    );
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_cargo_home_config(), &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();

    let units = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    let key = unit_key(
        "cargo-home",
        StorageCategory::Installation,
        0,
        &fs::canonicalize(&cargo_home).unwrap(),
    );
    // Device is baked into the real key by `discover_and_measure`
    // internally; recover the *actual* key from the unit itself instead
    // of recomputing the device by hand.
    let unit = cargo_home_unit(&units);
    let real_key = unit_key(
        &unit.detector_id,
        unit.category,
        real_device(&unit.path),
        &unit.path,
    );
    let _ = key; // illustrative only; `real_key` is what associate_consumer needs

    associate_consumer(store.path(), &real_key, "project-a", None).unwrap();
    associate_consumer(
        store.path(),
        &real_key,
        "project-b",
        Some("declared in config"),
    )
    .unwrap();

    let units2 = discover_and_measure(&scope, Some(store.path()), true, 2_000, 30, 3600).unwrap();
    let unit2 = cargo_home_unit(&units2);
    assert_eq!(unit2.consumers.len(), 2, "{:?}", unit2.consumers);
    let labels: std::collections::BTreeSet<_> =
        unit2.consumers.iter().map(|c| c.label.clone()).collect();
    assert_eq!(
        labels,
        ["project-a", "project-b"]
            .into_iter()
            .map(String::from)
            .collect()
    );
    // The unit itself is still exactly one row: total_bytes counts its
    // bytes once, not once per consumer.
    assert_eq!(total_bytes(&units2), unit2.bytes);
}

#[cfg(unix)]
fn real_device(path: &std::path::Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).unwrap().dev()
}

/// Associating and later dissociating a consumer must never duplicate
/// the unit or perturb its byte history: growth/regrowth before and
/// after the association churn must match exactly.
#[test]
fn association_changes_never_duplicate_the_unit_or_reset_history() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 4_000);
    let env = fixture_env(
        home.path(),
        &[("CARGO_HOME", &cargo_home.display().to_string())],
    );
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_cargo_home_config(), &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();

    let before = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    let before_count = before.len();
    let unit = cargo_home_unit(&before);
    let key = unit_key(
        &unit.detector_id,
        unit.category,
        real_device(&unit.path),
        &unit.path,
    );
    let bytes_before = unit.bytes;
    let regrowth_before = unit.regrowth_count;

    for i in 0..3 {
        associate_consumer(store.path(), &key, &format!("consumer-{i}"), None).unwrap();
    }
    for i in 0..3 {
        dissociate_consumer(store.path(), &key, &format!("consumer-{i}")).unwrap();
    }
    associate_consumer(store.path(), &key, "kept", None).unwrap();

    let after = discover_and_measure(&scope, Some(store.path()), true, 2_000, 30, 3600).unwrap();
    assert_eq!(
        after.len(),
        before_count,
        "association churn must not create or drop unit rows"
    );
    let unit_after = cargo_home_unit(&after);
    assert_eq!(unit_after.bytes, bytes_before, "bytes must be unaffected");
    assert_eq!(
        unit_after.regrowth_count, regrowth_before,
        "regrowth_count must be unaffected by consumer association changes"
    );
    assert_eq!(
        unit_after
            .consumers
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        vec!["kept"]
    );
}

/// A tool's storage is measured from its conventional/overridden path
/// alone -- never gated on finding the tool's own executable. Homebrew's
/// detector proposes its conventional prefixes even with no working
/// `brew` query (`Environment::fixture`'s default `NullCommandRunner`
/// simulates exactly "the tool binary is gone"), so a leftover install
/// prefix is still measured after the tool itself was uninstalled.
#[test]
fn tool_executable_removed_but_storage_remains_still_measures_it() {
    let home = tempfile::tempdir().unwrap();
    let prefix = home.path().join("fixture-homebrew-prefix");
    write_pattern(&prefix.join("Cellar/somepkg/1.0/bin/tool"), 6_000);
    let env = fixture_env(
        home.path(),
        &[("HOMEBREW_PREFIX", &prefix.display().to_string())],
    );
    let registry = Registry::with_builtins();
    let cfg = ScanConfig {
        defaults: false,
        include: Vec::new(),
        exclude: Vec::new(),
        disabled_detectors: vec!["cargo-home".into(), "rustup".into()],
    };
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();

    // No command runner was ever configured on this fixture Environment
    // (`Environment::fixture` defaults to `NullCommandRunner`): every
    // `brew --prefix` query this detector might attempt fails, exactly
    // as it would with `brew` missing from `PATH`.
    let units = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    // #49 refined Homebrew into Cellar/Caskroom as their own units,
    // separate from the bare prefix -- the fixture's file lives under
    // Cellar, so *that* unit (not the prefix unit, which now correctly
    // excludes it) is what still measures it with no working `brew`.
    let cellar = units
        .iter()
        .find(|u| {
            u.detector_id == "homebrew"
                && u.path == fs::canonicalize(&prefix).unwrap().join("Cellar")
        })
        .expect("Cellar is still measured with no working brew binary");
    assert!(cellar.bytes > 0, "{}", cellar.bytes);
    // The bare prefix is still measured too, just correctly excluding
    // Cellar/Caskroom's separately-counted bytes (the external-location
    // double-measurement fix, same chunk).
    let prefix_unit = units
        .iter()
        .find(|u| u.detector_id == "homebrew" && u.path == fs::canonicalize(&prefix).unwrap())
        .expect("the conventional prefix is still measured with no working brew binary");
    assert_eq!(
        prefix_unit.bytes, 0,
        "the prefix's own measurement must exclude Cellar's separately-counted bytes"
    );
}

/// A unit whose storage is unreadable this pass (simulated the same way
/// as #42's worktree-access-loss protection) is not tombstoned, and
/// reappears with unaffected history once access returns.
#[test]
fn incomplete_coverage_preserves_unknown_not_absent() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 2_500);
    let env = fixture_env(
        home.path(),
        &[("CARGO_HOME", &cargo_home.display().to_string())],
    );
    let registry = Registry::with_builtins();
    let scope = resolve_effective_scope(&env, &only_cargo_home_config(), &[], &registry, 1);
    let store = tempfile::tempdir().unwrap();

    let first = discover_and_measure(&scope, Some(store.path()), true, 1_000, 30, 3600).unwrap();
    let bytes_first = cargo_home_unit(&first).bytes;
    assert!(bytes_first > 0);

    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&cargo_home, fs::Permissions::from_mode(0o000)).unwrap();
    let during = discover_and_measure(&scope, Some(store.path()), true, 2_000, 30, 3600).unwrap();
    fs::set_permissions(&cargo_home, fs::Permissions::from_mode(0o755)).unwrap();
    let protected = during
        .iter()
        .find(|u| u.detector_id == "cargo-home" && u.category == StorageCategory::Installation)
        .expect("a coverage row is still emitted while access is lost");
    assert!(protected.note.is_some(), "{:?}", protected.note);

    let after = discover_and_measure(&scope, Some(store.path()), true, 3_000, 30, 3600).unwrap();
    let unit_after = cargo_home_unit(&after);
    assert_eq!(unit_after.bytes, bytes_first);
    assert_eq!(
        unit_after.regrowth_count, 0,
        "losing and regaining read access to an external unit must never count as regrowth"
    );
}
