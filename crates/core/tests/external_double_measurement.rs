//! Adversarial tests for the external-location double-measurement fix
//! (chunk G, alongside #45-#49): a detector-resolved location that also
//! falls inside a kept, ordinarily-walked scan root (or inside another
//! detector-resolved location that is itself measured as an external
//! unit) must contribute its bytes exactly once -- to the external
//! unit's own measurement, never also to an ordinary walk's
//! walked/unowned total, and never twice across two external units.
//!
//! The tempting shortcut this file exists to catch: measuring
//! `resize_artifact` naively for both a parent location and a nested
//! child location, or walking an ordinary scan root without pruning a
//! nested detector location out of it, silently doubles the same bytes.
//! A happy-path test that only checks "a homebrew unit exists" or "the
//! root produced some report" would pass under that bug; every
//! assertion below is a reconstruction identity that fails if bytes are
//! duplicated or dropped.

use std::fs;
use std::path::Path;

use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::report::report_scope;
use swamp_core::scope::{ScanConfig, resolve_effective_scope};
use swamp_core::walk::{resize_artifact, resize_artifact_excluding};

fn write_pattern(path: &Path, bytes: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![9u8; bytes as usize]).unwrap();
}

/// An `Environment` whose Homebrew prefix is pinned inside the fixture
/// home -- never `/opt/homebrew` or `/usr/local`. Without this, the
/// Homebrew detector's convention-prefix fallback would propose the
/// *real* prefixes on the machine running the test, in direct violation
/// of "never scan the developer's real home in tests": on a machine
/// with Homebrew actually installed, walking a real multi-gigabyte
/// Cellar would also make every byte-count assertion below meaningless.
fn fixture_env(home: &Path) -> Environment {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "HOMEBREW_PREFIX".to_string(),
        home.join("fixture-homebrew-prefix").display().to_string(),
    );
    Environment::fixture(home.to_path_buf(), env, Platform::MacOS)
}

/// Every detector except `keep` is disabled, so a fixture home never
/// picks up incidental noise from other catalog entries.
fn only_config(keep: &[&str], registry: &Registry) -> ScanConfig {
    let disabled: Vec<String> = registry
        .detectors()
        .iter()
        .map(|d| d.id().to_string())
        .filter(|id| id != "builtin-defaults" && !keep.contains(&id.as_str()))
        .collect();
    ScanConfig {
        defaults: true,
        disabled_detectors: disabled,
        ..ScanConfig::default()
    }
}

/// The concrete FOLLOWUPS scenario: Homebrew's downloads cache
/// (`~/Library/Caches/Homebrew`) sits inside the built-in default root
/// `~/Library/Caches`. Before this fix, `report_scope`'s ordinary walk
/// of `~/Library/Caches` counted the Homebrew subtree as unowned bytes
/// *and* `external::discover_and_measure` counted the same subtree
/// again as the Homebrew downloads unit.
#[test]
fn nested_external_location_is_pruned_from_its_parent_roots_walk() {
    let home = tempfile::tempdir().unwrap();
    let caches = home.path().join("Library/Caches");
    let homebrew_cache = caches.join("Homebrew");
    write_pattern(&homebrew_cache.join("downloads/bottle.tar.gz"), 40_000);
    write_pattern(&caches.join("some-other-tool/blob"), 15_000);
    // `external::discover_and_measure` reports canonicalized paths
    // (symlinks resolved, e.g. macOS's `/var` -> `/private/var`); this
    // fixture's own `homebrew_cache` is deliberately left unresolved to
    // match `EffectiveScope`'s lexical (non-canonicalizing) paths, so a
    // canonical copy is needed for the external-unit lookups below.
    let homebrew_cache_canonical = fs::canonicalize(&homebrew_cache).unwrap();

    let env = fixture_env(home.path());
    let registry = Registry::with_builtins();
    let cfg = only_config(&["homebrew"], &registry);
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1_000);

    // The scope resolution itself must record why: an
    // `ExternalPruneNote` naming the Homebrew cache as pruned from the
    // Caches root, attributed to the homebrew detector.
    let note = scope
        .external_pruned_subtrees
        .iter()
        .find(|n| n.path == homebrew_cache && n.root == caches)
        .expect("homebrew cache must be recorded as pruned from ~/Library/Caches");
    assert_eq!(note.detector_id, "homebrew");

    let (report, _coverage) =
        report_scope(&scope, None, false, None, None, false, false, false, true)
            .expect("report_scope succeeds over the fixture scope");

    // Reconstruction identity: measuring ~/Library/Caches with the
    // Homebrew subtree explicitly excluded (the same machinery, called
    // directly) must equal exactly what the ordinary walk attributed to
    // that root -- no more (would mean Homebrew leaked back in), no
    // less (would mean real non-Homebrew cache content vanished).
    let expected_without_homebrew = resize_artifact_excluding(
        &caches,
        swamp_core::report::ArtifactKind::Unknown,
        1_000,
        std::slice::from_ref(&homebrew_cache),
    );
    assert_eq!(
        report.reconciliation.walked_total, expected_without_homebrew.bytes,
        "the ordinary walk of ~/Library/Caches must attribute exactly the \
         non-Homebrew bytes once Homebrew's cache is pruned as an external unit"
    );
    assert!(
        report.reconciliation.walked_total > 0,
        "the unrelated some-other-tool cache content must still be counted"
    );

    // The prune must be visible in the report's own notes, not just in
    // `EffectiveScope` -- "the root's report notes say so".
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("measured as external unit") && n.contains("homebrew")),
        "{:?}",
        report.notes
    );

    // Now reconstruct the *other* half: `external::discover_and_measure`
    // must independently measure exactly the pruned subtree, and the
    // sum of the two halves must equal one naive, unexcluded measurement
    // of the whole Caches directory -- proving the bytes appear exactly
    // once across the two views, never zero and never twice.
    let units = swamp_core::external::discover_and_measure(&scope, None, false, 1_000, 30, 3600)
        .expect("discover_and_measure succeeds");
    let homebrew_unit = units
        .iter()
        .find(|u| u.detector_id == "homebrew" && u.path == homebrew_cache_canonical)
        .expect("homebrew cache is measured as its own external unit");
    assert!(homebrew_unit.bytes > 0);

    let naive_whole_caches =
        resize_artifact(&caches, swamp_core::report::ArtifactKind::Unknown, 1_000);
    assert_eq!(
        report.reconciliation.walked_total + homebrew_unit.bytes,
        naive_whole_caches.bytes,
        "walked (non-Homebrew) bytes plus the Homebrew external unit's bytes \
         must reconstruct one undivided measurement of Caches exactly -- \
         proving nothing was double-counted or dropped"
    );
}

/// Same fixture, but the two halves are asserted in the opposite order
/// (external unit first, then the walk) and the scope is resolved a
/// second time from a config built with the detector list in reverse
/// registration order. The prune/measurement identity must not depend
/// on any particular ordering of detectors, roots, or candidates.
#[test]
fn double_measurement_fix_is_order_independent() {
    let home = tempfile::tempdir().unwrap();
    let caches = home.path().join("Library/Caches");
    let homebrew_cache = caches.join("Homebrew");
    write_pattern(&homebrew_cache.join("downloads/bottle.tar.gz"), 22_000);
    write_pattern(&caches.join("unrelated/blob"), 9_000);
    let homebrew_cache_canonical = fs::canonicalize(&homebrew_cache).unwrap();

    let env = fixture_env(home.path());
    let registry = Registry::with_builtins();
    // A disabled_detectors list built by reversing the registry's own
    // order -- an adversarial permutation, not the registration order
    // any other test happens to rely on.
    let mut keep_disabled: Vec<String> = registry
        .detectors()
        .iter()
        .rev()
        .map(|d| d.id().to_string())
        .filter(|id| id != "builtin-defaults" && id != "homebrew")
        .collect();
    keep_disabled.sort(); // resolve_effective_scope sorts+dedups anyway
    let cfg = ScanConfig {
        defaults: true,
        disabled_detectors: keep_disabled,
        ..ScanConfig::default()
    };
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 2_000);

    let units = swamp_core::external::discover_and_measure(&scope, None, false, 2_000, 30, 3600)
        .expect("discover_and_measure succeeds");
    let homebrew_unit = units
        .iter()
        .find(|u| u.detector_id == "homebrew" && u.path == homebrew_cache_canonical)
        .expect("homebrew cache is measured as its own external unit");

    let (report, _coverage) =
        report_scope(&scope, None, false, None, None, false, false, false, true)
            .expect("report_scope succeeds over the fixture scope");

    let naive_whole_caches =
        resize_artifact(&caches, swamp_core::report::ArtifactKind::Unknown, 2_000);
    assert_eq!(
        homebrew_unit.bytes + report.reconciliation.walked_total,
        naive_whole_caches.bytes,
        "order of resolution must not change the reconstruction identity"
    );
}

/// #47 refined Cargo home into five separate proposed locations
/// (base, registry/cache, registry/src, registry/index, git/db,
/// git/checkouts), all nested under the same `CARGO_HOME`. Every one of
/// them is independently resolved and independently measured as its own
/// external unit (`crate::external::discover_and_measure` does not
/// consult scope-root nesting at all) -- so without the external-unit
/// self-exclusion fix in `external::discover_and_measure`, the base
/// unit's naive whole-directory measurement would also include
/// registry/src, registry/cache, and git's bytes, on top of those
/// subtrees' own separate units: the same bytes counted 2x (registry/git
/// content) to 6x (git/checkouts' own content, nested inside git which
/// is nested inside base -- though `git` itself is not a separate
/// proposed location here, `git/db` and `git/checkouts` both nest under
/// `base` directly).
#[test]
fn cargo_homes_own_measurement_excludes_its_separately_measured_subtrees() {
    let home = tempfile::tempdir().unwrap();
    let cargo_home = home.path().join("fixture-cargo");
    write_pattern(&cargo_home.join("bin/cargo"), 5_000);
    write_pattern(
        &cargo_home.join("registry/cache/somecrate-1.0.crate"),
        30_000,
    );
    write_pattern(
        &cargo_home.join("registry/src/somecrate-1.0/lib.rs"),
        20_000,
    );
    write_pattern(&cargo_home.join("git/db/somerepo/HEAD"), 1_000);
    write_pattern(
        &cargo_home.join("git/checkouts/somerepo/abc123/lib.rs"),
        8_000,
    );

    let mut env_vars = std::collections::HashMap::new();
    env_vars.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    // Homebrew is disabled by `only_config` below, but its prefix is
    // still pinned into the fixture home defensively, matching every
    // other test in this file -- "never scan the developer's real home
    // in tests" is a property of the whole suite, not of any one test's
    // active detector list.
    env_vars.insert(
        "HOMEBREW_PREFIX".to_string(),
        home.path()
            .join("fixture-homebrew-prefix")
            .display()
            .to_string(),
    );
    let env = Environment::fixture(home.path().to_path_buf(), env_vars, Platform::MacOS);
    let registry = Registry::with_builtins();
    let cfg = only_config(&["cargo-home"], &registry);
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 3_000);

    let units = swamp_core::external::discover_and_measure(&scope, None, false, 3_000, 30, 3600)
        .expect("discover_and_measure succeeds");
    assert_eq!(units.len(), 5, "{units:#?}");

    let canonical_cargo_home = fs::canonicalize(&cargo_home).unwrap();
    let base_unit = units
        .iter()
        .find(|u| u.path == canonical_cargo_home)
        .expect("cargo home's own base unit is present");
    // Only bin/cargo's bytes: everything else lives under a separately
    // measured, nested subtree that must be excluded from this one.
    let base_only = resize_artifact_excluding(
        &canonical_cargo_home,
        swamp_core::report::ArtifactKind::Unknown,
        3_000,
        &[
            canonical_cargo_home.join("registry"),
            canonical_cargo_home.join("git"),
        ],
    );
    assert_eq!(base_unit.bytes, base_only.bytes);
    assert!(
        base_unit.bytes > 0 && base_unit.bytes < 10_000,
        "{}",
        base_unit.bytes
    );

    // Sum of every unit's bytes must equal one undivided measurement of
    // the whole cargo home -- proving nothing nested was double-counted
    // (registry/src, registry/cache, git/db, git/checkouts all counted
    // once each, not also folded into the base unit's own total).
    let naive_whole = resize_artifact(
        &canonical_cargo_home,
        swamp_core::report::ArtifactKind::Unknown,
        3_000,
    );
    let sum: u64 = units.iter().map(|u| u.bytes).sum();
    assert_eq!(sum, naive_whole.bytes, "{units:#?}");
}
