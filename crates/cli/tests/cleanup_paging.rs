use std::{fs, path::Path, process::Command};

fn run(root: &Path, store: &Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_swamp"))
        .arg("cleanup-check")
        .arg(root)
        .args(args)
        .arg("--json")
        .env("SWAMP_DIR", store)
        .env("SWAMP_TEST_MODE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn bounded_pages_expose_coverage_without_restarting_or_widening() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "target/\n").unwrap();
    let profile = root.join("target/debug");
    for name in ["a", "b", "c"] {
        let group = profile.join("incremental").join(name);
        fs::create_dir_all(&group).unwrap();
        fs::write(group.join("state.o"), [0; 4096]).unwrap();
    }
    fs::write(profile.join(".cargo-lock"), b"").unwrap();
    let store = tempfile::tempdir().unwrap();
    let first = run(
        &root,
        store.path(),
        &["--role", "incremental", "--limit", "1"],
    );
    assert_eq!(first["observed_candidate_count"], 3);
    assert_eq!(first["not_checked_in_this_run"], 2);
    assert_eq!(first["next_offset"], 1);
    let second = run(
        &root,
        store.path(),
        &["--role", "incremental", "--limit", "1", "--offset", "1"],
    );
    assert_ne!(first["results"][0]["path"], second["results"][0]["path"]);
    let empty = run(
        &root,
        store.path(),
        &["--role", "incremental", "--offset", "999"],
    );
    assert_eq!(empty["checked_count"], 0);
    assert!(empty["next_page"].is_null());
    let one = profile.join("incremental/b");
    let scoped = run(&root, store.path(), &["--within", one.to_str().unwrap()]);
    assert_eq!(scoped["observed_candidate_count"], 0);
    assert_eq!(scoped["checked_count"], 0);
    let parent_scope = profile.join("incremental");
    let scoped = run(
        &root,
        store.path(),
        &["--within", parent_scope.to_str().unwrap(), "--limit", "5"],
    );
    assert_eq!(scoped["observed_candidate_count"], 3);
    assert!(
        scoped["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["path"] != parent_scope.to_str().unwrap())
    );
    let exact = run(
        &root,
        store.path(),
        &["--path", one.to_str().unwrap(), "--limit", "1"],
    );
    assert_eq!(exact["checked_count"], 1);
    assert_eq!(exact["results"][0]["path"], one.to_str().unwrap());
    let no_groups = root.join("source-only");
    fs::create_dir_all(&no_groups).unwrap();
    let empty = run(
        &root,
        store.path(),
        &["--within", no_groups.to_str().unwrap()],
    );
    assert_eq!(empty["checked_count"], 0);
    assert!(profile.join("incremental/a/state.o").exists());
    assert!(profile.join("incremental/b/state.o").exists());
    assert!(profile.join("incremental/c/state.o").exists());
}
