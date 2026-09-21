//! End-to-end `swamp observe` checks against the built binary:
//! it writes the growth store and prints one machine-readable line per
//! root.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_swamp"))
}

#[test]
fn observe_on_a_fixture_root_writes_the_store_and_prints_the_line() {
    let root = tempfile::tempdir().expect("root");
    std::fs::write(root.path().join("hello.txt"), b"hi").unwrap();
    let store = tempfile::tempdir().expect("store");

    let output = Command::new(bin())
        .arg("observe")
        .arg(root.path())
        .env("SWAMP_DIR", store.path())
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run observe");

    assert!(
        output.status.success(),
        "observe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("observed_at=") && stdout.contains("mode=full"),
        "stdout did not contain a machine-readable observe line: {stdout}"
    );

    // The volume-keyed growth store must now exist under SWAMP_DIR.
    let has_store = std::fs::read_dir(store.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_dir());
    assert!(
        has_store,
        "observe must persist a volume dir under the store"
    );

    let last_run = store.path().join("last_run.json");
    assert!(last_run.exists(), "observe must persist last_run.json");
    let text = std::fs::read_to_string(&last_run).unwrap();
    assert!(text.contains("\"outcome\":\"ok\""));
}

/// #41: `observe` with no explicit root resolves the configured scope
/// (shared with `report`/`ui`/`schedule` through
/// `swamp_core::scope::resolve_effective_scope`) instead of requiring an
/// explicit root. Disables every non-builtin detector so this test's
/// outcome depends only on the fixture `HOME`'s layout, never on
/// whatever happens to exist at `/opt/homebrew`, `/usr/local`, or a real
/// `~/.cargo` on the machine running the test.
#[test]
fn observe_with_no_roots_uses_the_configured_default_scope() {
    let home = tempfile::tempdir().expect("home");
    std::fs::create_dir_all(home.path().join("src")).unwrap();
    std::fs::write(home.path().join("src/hello.txt"), b"hi").unwrap();
    let store = tempfile::tempdir().expect("store");
    std::fs::write(
        store.path().join("config.toml"),
        "[scan]\ndisabled_detectors = [\"cargo-home\", \"rustup\", \"homebrew\"]\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("observe")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run observe with no roots");

    assert!(
        output.status.success(),
        "observe with no roots should resolve the configured default scope: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&home.path().join("src").display().to_string()),
        "observe should have walked the configured ~/src default root: {stdout}"
    );

    // #41: a resolved scope is persisted so the next run can report
    // coverage changes; this is coverage bookkeeping, never byte
    // history.
    let scope_file = store.path().join("scope.json");
    assert!(
        scope_file.exists(),
        "observe must persist the effective scope for next time"
    );
}

/// #41's "empty effective scope is explicit... never a silent fallback
/// to cwd or home": with defaults off and every detector disabled and
/// nothing configured under `include`, `observe` must fail with a
/// visible, nonzero-exit error -- never quietly scan the process's
/// current directory or the fixture `HOME` itself.
#[test]
fn observe_with_empty_scope_fails_visibly_never_falls_back_to_cwd() {
    let home = tempfile::tempdir().expect("home");
    let store = tempfile::tempdir().expect("store");
    std::fs::write(
        store.path().join("config.toml"),
        "[scan]\ndefaults = false\ndisabled_detectors = [\"cargo-home\", \"rustup\", \"homebrew\", \"claude-code\", \"codex\", \"codex-desktop\", \"oh-my-pi\", \"opencode\"]\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("observe")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .expect("run observe with empty scope");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("empty"),
        "must fail with a visible, explicit message naming the empty scope, not a silent fallback: {stderr}"
    );
}

/// Invalid `[scan]` config must refuse to run rather than silently
/// falling back to the (broader) all-defaults scope -- #41's core
/// safety requirement, exercised end-to-end through the real binary.
#[test]
fn observe_with_invalid_scan_config_fails_visibly() {
    let home = tempfile::tempdir().expect("home");
    let store = tempfile::tempdir().expect("store");
    std::fs::write(
        store.path().join("config.toml"),
        "[scan]\ndefaults = \"not-a-bool\"\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("observe")
        .env("SWAMP_DIR", store.path())
        .env("HOME", home.path())
        .output()
        .expect("run observe with invalid config");

    assert!(!output.status.success());
}
