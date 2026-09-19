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

#[test]
fn observe_needs_at_least_one_root() {
    let output = Command::new(bin())
        .arg("observe")
        .output()
        .expect("run observe with no roots");
    assert!(!output.status.success());
}
