//! Independent re-review of the whole stack (#116-#125) at 6a2c563.
//!
//! Every test here asserts **required behavior**, not the bug. All of
//! them FAIL on 6a2c563. They are variants designed to slip past the
//! shape of the repairs the stack already made: the seventeen prior
//! counterexamples all pass, so these attack the *edges* of those fixes
//! rather than repeating them.
//!
//! Safety: every fixture is a disposable `tempfile` tree. Nothing reads
//! a real agent/editor home, a real tool store or the user's own swamp
//! state, and nothing is moved or removed outside the per-test temp dir.
//!
//! Run:
//! ```sh
//! cargo test -p swamp-core --test reviewer_counterexamples_stack2 \
//!   --target-dir <scratch>/target-audit -- --test-threads=1 --nocapture
//! ```
//!
//! The TUI counterexample (CE3) needs `swamp-tui` and so lives in
//! `crates/tui/tests/reviewer_counterexamples_stack2_tui.rs`.

use std::{collections::HashMap, fs};
use swamp_core::{
    actions, agents, external,
    locations::{Environment, Platform, Registry},
    recheck,
    scope::{ScanConfig, resolve_effective_scope},
};

/// An explicit-only scope with exactly `ids` enabled, so no detector on
/// the reviewer's own machine can leak into a fixture.
fn only(ids: &[&str]) -> ScanConfig {
    ScanConfig {
        defaults: false,
        disabled_detectors: Registry::with_builtins()
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .filter(|id| !ids.contains(&id.as_str()))
            .collect(),
        ..Default::default()
    }
}

fn claude_home(tmp: &std::path::Path) -> std::path::PathBuf {
    let home = tmp.join("claude");
    fs::create_dir_all(home.join("debug")).unwrap();
    fs::write(home.join("debug/log.txt"), b"AAAAAAAA").unwrap();
    home
}

fn claude_env(tmp: &std::path::Path, home: &std::path::Path) -> Environment {
    Environment::fixture(
        tmp.to_owned(),
        HashMap::from([("CLAUDE_CONFIG_DIR".into(), home.display().to_string())]),
        Platform::MacOS,
    )
}

// ---------------------------------------------------------------------
// CE1. An `exclude` entry must still exclude under `--root`.
//
// `excluded_agent_home_must_not_be_scanned` is fixed for the
// non-explicit path only. Under an explicit command root,
// `agents::authorized_tool_homes` and `external::authorized_candidates`
// switch to `EffectiveScope::authorized_detector_paths_in_explicit_roots`
// (scope.rs:502), which tests exclusion *only* against
// `self.roots` entries whose status is `RootStatus::Excluded`. Under
// `--root <parent>` the excluded home is not a root at all -- it is a
// `PruneNote` inside the explicit root -- so nothing matches and the
// home the user excluded is discovered, measured and made actionable.
// ---------------------------------------------------------------------
#[test]
fn an_excluded_home_must_stay_excluded_under_an_explicit_root() {
    let tmp = tempfile::tempdir().unwrap();
    let home = claude_home(tmp.path());
    let env = claude_env(tmp.path(), &home);
    let cfg = ScanConfig {
        exclude: vec![home.display().to_string()],
        ..only(&["claude-code"])
    };
    // The user names the *parent* of the excluded home as the root.
    let scope = resolve_effective_scope(
        &env,
        &cfg,
        &[tmp.path().to_path_buf()],
        &Registry::with_builtins(),
        1000,
    );
    let units = agents::discover_and_measure(&scope, &[], None, false, 1000, 30, 3600).unwrap();
    let ext = external::discover_and_measure(&scope, None, false, 1000, 30, 3600).unwrap();
    assert!(
        units.is_empty() && ext.is_empty(),
        "an excluded home was scanned under --root: {} agent units {:?}, {} external units {:?}",
        units.len(),
        units.iter().map(|u| &u.path).collect::<Vec<_>>(),
        ext.len(),
        ext.iter().map(|u| &u.path).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------
// CE2. A reviewed member rewritten in place must not spend the old
// approval.
//
// `recheck.rs` fingerprints a member as `(path, bytes, mtime_secs,
// inode)`. An in-place rewrite that preserves the byte count within the
// same wall-clock second changes none of the four, so
// `reviewed_snapshot` reports the unit unchanged and
// `execute_agent_cache_trash` (actions.rs:1312) moves content no human
// reviewed. The project already knows this fingerprint is too weak:
// `agents/mod.rs:396-412` rejects exactly `(size, mtime_secs)` for the
// *identification cache* and uses `mtime_ns` + `ctime_ns` + `inode`
// there. The destructive sink uses the fingerprint that module rejects.
// (The ordinary-artifact branch has a second `newest_mtime > created_at`
// gate; the agent-cache branch has none, and that gate is itself
// second-granular.)
// ---------------------------------------------------------------------
#[test]
fn an_in_place_rewrite_of_a_reviewed_member_must_not_spend_the_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let home = claude_home(tmp.path());
    let env = claude_env(tmp.path(), &home);
    let scope = resolve_effective_scope(
        &env,
        &only(&["claude-code"]),
        &[],
        &Registry::with_builtins(),
        1000,
    );
    let store = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    let path = home.join("debug");
    let units = agents::discover_and_measure(&scope, &[], None, false, 1000, 30, 3600).unwrap();
    let plan = actions::propose_agents(&units, &[path.clone()], "reviewer").unwrap();
    actions::save_plan(store.path(), &plan).unwrap();
    actions::approve(store.path(), &plan.id, "human:reviewer").unwrap();
    // Same inode, same byte count, same second: a log rotation, a cache
    // blob overwritten, an agent rewriting a fixed-width record.
    fs::write(path.join("log.txt"), b"BBBBBBBB").unwrap();
    let result =
        actions::execute_with_trash(store.path(), &plan.id, "human:reviewer", trash.path())
            .unwrap();
    assert!(
        path.exists(),
        "content rewritten after approval was moved under the old approval: {:?}",
        result.outcomes
    );
}

/// The same hole at the layer that owns it, so the fix has an obvious
/// home: `recheck::reviewed_snapshot` must refuse an in-place rewrite.
#[test]
fn reviewed_snapshot_must_see_a_same_second_same_size_rewrite() {
    let tmp = tempfile::tempdir().unwrap();
    let unit = tmp.path().join("unit");
    fs::create_dir_all(&unit).unwrap();
    fs::write(unit.join("a.txt"), b"AAAA").unwrap();
    let reviewed = recheck::capture(&unit).unwrap();
    fs::write(unit.join("a.txt"), b"BBBB").unwrap();
    assert!(
        recheck::reviewed_snapshot(&unit, Some(&reviewed)).is_err(),
        "an in-place rewrite of a reviewed member passed the identity recheck"
    );
}

// ---------------------------------------------------------------------
// CE4. A config-only exclusion must not invent growth or regrowth.
//
// `unchanged_combined_observation_must_not_invent_regrowth` is fixed for
// the two-family case via `ObservationOwnership`. The ownership window
// is still *path-prefix* based (growth.rs:3607), and
// `external::discover_and_measure` seeds it with the paths it measured
// this pass (external.rs:342-345). Excluding a nested location therefore
// does two things at once:
//
//   * the parent's `nested_exclusions` are computed from the surviving
//     candidate list (external.rs:280-289), so the parent silently
//     absorbs the child's bytes -- reported as real `growth_bytes`; and
//   * the child's stored row still lies under the parent's covered root,
//     so the owned sweep tombstones it, and un-excluding it later scores
//     a `regrowth_count`.
//
// Zero bytes changed on disk in this test. Both numbers are fiction, and
// `.oh/guardrails/coverage-changes-are-not-storage-changes.md` is the
// guardrail they violate.
// ---------------------------------------------------------------------
#[test]
fn a_config_only_exclusion_must_not_invent_growth_or_regrowth() {
    let tmp = tempfile::tempdir().unwrap();
    let outer = tmp.path().join("cargo");
    let inner = outer.join("registry/cache");
    fs::create_dir_all(&inner).unwrap();
    fs::write(outer.join("top.bin"), vec![b'x'; 4096]).unwrap();
    fs::write(inner.join("crate.crate"), vec![b'y'; 65536]).unwrap();
    let env = Environment::fixture(
        tmp.path().to_owned(),
        HashMap::from([("CARGO_HOME".into(), outer.display().to_string())]),
        Platform::MacOS,
    );
    let store = tempfile::tempdir().unwrap();
    let registry = Registry::with_builtins();

    let base = resolve_effective_scope(&env, &only(&["cargo-home"]), &[], &registry, 1000);
    let first =
        external::discover_and_measure(&base, Some(store.path()), true, 1000, 30, 3600).unwrap();
    assert!(
        first.len() >= 2,
        "precondition: the parent and the nested location are both measured: {:?}",
        first.iter().map(|u| (&u.path, u.bytes)).collect::<Vec<_>>()
    );

    // One line of config changes. Nothing on disk does.
    let excluded = resolve_effective_scope(
        &env,
        &ScanConfig {
            exclude: vec![inner.display().to_string()],
            ..only(&["cargo-home"])
        },
        &[],
        &registry,
        2000,
    );
    let second =
        external::discover_and_measure(&excluded, Some(store.path()), true, 2000, 30, 3600)
            .unwrap();
    let invented_growth: Vec<_> = second
        .iter()
        .filter(|u| u.growth_bytes.unwrap_or(0) != 0)
        .map(|u| (u.path.clone(), u.bytes, u.growth_bytes))
        .collect();
    assert!(
        invented_growth.is_empty(),
        "excluding a nested location reported the parent as having grown: {invented_growth:?}"
    );

    // And restoring the config must not read as the child coming back.
    let third =
        external::discover_and_measure(&base, Some(store.path()), true, 3000, 30, 3600).unwrap();
    let invented_regrowth: Vec<_> = third
        .iter()
        .filter(|u| u.regrowth_count > 0)
        .map(|u| (u.path.clone(), u.regrowth_count))
        .collect();
    assert!(
        invented_regrowth.is_empty(),
        "un-excluding a nested location was recorded as regrowth: {invented_regrowth:?}"
    );
}

// ---------------------------------------------------------------------
// CE5. `swamp protect add` must not accept a path it cannot enforce.
//
// `protect_add` stores the argument verbatim (agents/mod.rs:989) and
// `protection_conflict` (agents/mod.rs:1037) compares it against
// absolute unit paths in both directions. A *relative* entry matches
// neither direction, so it protects nothing -- while the CLI prints
// "protected: debug" (cli/src/main.rs:1788) and `swamp protect list`
// keeps showing it. The protection layer is supposed to fail closed;
// here it accepts, confirms, and then does not protect.
//
// The existing "used verbatim, never canonicalized" rationale
// (agents/mod.rs:981-988) is about *symlinks*; it does not justify
// accepting a relative path. Required behavior: refuse it, or resolve it
// against the cwd before storing.
// ---------------------------------------------------------------------
#[test]
fn protect_add_must_not_accept_a_path_it_cannot_enforce() {
    let tmp = tempfile::tempdir().unwrap();
    let home = claude_home(tmp.path());
    let env = claude_env(tmp.path(), &home);
    let scope = resolve_effective_scope(
        &env,
        &only(&["claude-code"]),
        &[],
        &Registry::with_builtins(),
        1000,
    );
    let store = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    let path = home.join("debug");

    // What a user types from inside the tool home.
    let added = agents::protect_add(store.path(), std::path::Path::new("debug"));
    if added.is_ok() {
        assert_eq!(
            agents::protect_list(store.path()).unwrap(),
            vec![std::path::PathBuf::from("debug")],
            "precondition: the relative entry was stored as given"
        );
        let units =
            agents::discover_and_measure(&scope, &[], Some(store.path()), false, 1000, 30, 3600)
                .unwrap();
        if let Ok(plan) = actions::propose_agents(&units, &[path.clone()], "reviewer") {
            actions::save_plan(store.path(), &plan).unwrap();
            actions::approve(store.path(), &plan.id, "human:reviewer").unwrap();
            let _ =
                actions::execute_with_trash(store.path(), &plan.id, "human:reviewer", trash.path());
        }
    }
    assert!(
        added.is_err() || path.exists(),
        "`protect add debug` was accepted and confirmed, and the data it named was still moved"
    );
}

// ---------------------------------------------------------------------
// CE6. An unchanged observation must not spawn subprocesses, and a
// disabled detector must stop its probes.
//
// `consumers::docker::DockerConsumer` calls `docker::load_cached`
// unconditionally on every `ProjectsGrouped` event
// (consumers/docker.rs:32) and consults the effective scope nowhere. So
// `docker-desktop` being disabled does not stop swamp asking the Docker
// daemon about the user's images, volumes and containers -- and because
// `load_cached` only caches a *successful* answer (docker.rs:651-682),
// an unavailable daemon is re-probed on every single pass. Measured on
// the multi-ecosystem fixture: five `docker` spawns per observation,
// forever, with Docker out of scope.
// ---------------------------------------------------------------------
#[test]
fn a_disabled_detector_must_not_probe_its_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let src = root.join("src/p");
    fs::create_dir_all(src.join("target/debug")).unwrap();
    fs::write(src.join("target/debug/blob.bin"), vec![b'x'; 4096]).unwrap();
    fs::write(src.join("Cargo.toml"), b"[package]\nname=\"x\"\n").unwrap();
    let store = tempfile::tempdir().unwrap();

    // A PATH shim that records every spawn instead of running the tool.
    let shims = root.join("shims");
    let log = root.join("spawns.log");
    fs::create_dir_all(&shims).unwrap();
    fs::write(&log, b"").unwrap();
    for name in ["docker", "lsof", "plutil", "xcrun", "du"] {
        let p = shims.join(name);
        fs::write(
            &p,
            format!("#!/bin/sh\necho {name} >> {}\nexit 1\n", log.display()),
        )
        .unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&p, perms).unwrap();
    }
    let previous = std::env::var("PATH").unwrap_or_default();
    unsafe { std::env::set_var("PATH", format!("{}:{previous}", shims.display())) };

    let registry = Registry::with_builtins();
    let cfg = ScanConfig {
        defaults: false,
        include: vec![root.join("src").display().to_string()],
        // Every detector disabled, Docker Desktop included.
        disabled_detectors: registry
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .collect(),
        ..Default::default()
    };
    let env = Environment::fixture(root.clone(), HashMap::new(), Platform::MacOS);
    let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
    for _ in 0..2 {
        let _ = swamp_core::report::observe_scope(
            &scope,
            swamp_core::report::ObservationParts::ALL,
            None,
            None,
            false,
            Some(store.path()),
            None,
            true,
            true,
            false,
            false,
            swamp_core::fs_events::platform_source().as_ref(),
            30,
            24 * 3600,
        );
    }
    unsafe { std::env::set_var("PATH", previous) };
    let spawned: Vec<String> = fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(
        spawned.is_empty(),
        "two observations with every detector disabled spawned {} subprocesses: {spawned:?}",
        spawned.len()
    );
}
