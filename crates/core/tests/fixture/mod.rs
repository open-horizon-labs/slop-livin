//! Reusable fixture builder for report golden tests.
//!
//! Builds, under a caller-supplied tmp root, a small multi-project tree
//! with real `git` checkouts (a main checkout plus a `git worktree add`
//! linked worktree), build-output/dependency directories, a nested second
//! repository, a shared cache directory outside any repo, loose files
//! outside any repo, and a mocked `docker system df -v` JSON fixture file.
//!
//! File contents are a deterministic byte pattern so sizes are exact and
//! reproducible across runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Paths and exact byte sizes for everything the fixture wrote.
pub struct Fixture {
    pub root: PathBuf,
    pub checkout: PathBuf,
    pub checkout_name: String,
    pub linked_worktree: PathBuf,
    pub node_modules: PathBuf,
    pub node_modules_bytes: u64,
    pub target_dir: PathBuf,
    pub target_bytes: u64,
    pub dist_dir: PathBuf,
    pub dist_bytes: u64,
    pub nested_repo: PathBuf,
    pub nested_repo_build: PathBuf,
    pub nested_repo_build_bytes: u64,
    pub shared_cache: PathBuf,
    pub shared_cache_bytes: u64,
    pub loose_file: PathBuf,
    pub loose_file_bytes: u64,
    pub docker_facts: PathBuf,
}

/// Writes `size` bytes of a deterministic repeating pattern to `path`,
/// creating parent directories as needed.
fn write_pattern(path: &Path, size: u64) {
    fs::create_dir_all(path.parent().expect("path has parent")).expect("mkdir parent");
    let pattern: Vec<u8> = (0..=255u8).collect();
    let mut remaining = size as usize;
    let mut buf = Vec::with_capacity(size as usize);
    while remaining > 0 {
        let take = remaining.min(pattern.len());
        buf.extend_from_slice(&pattern[..take]);
        remaining -= take;
    }
    fs::write(path, &buf).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Writes `total_bytes` split evenly across `count` files under `dir`,
/// named `file-0`, `file-1`, ... Returns the exact total bytes written.
fn write_files(dir: &Path, count: u64, total_bytes: u64) -> u64 {
    let per_file = total_bytes / count;
    let remainder = total_bytes - per_file * count;
    let mut written = 0u64;
    for i in 0..count {
        let size = if i == count - 1 {
            per_file + remainder
        } else {
            per_file
        };
        write_pattern(&dir.join(format!("file-{i}")), size);
        written += size;
    }
    written
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .unwrap_or_else(|e| panic!("run git {args:?} in {}: {e}", dir.display()));
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Builds the full fixture tree under `tmp` and returns its paths and
/// exact byte sizes.
pub fn build(tmp: &Path) -> Fixture {
    fs::create_dir_all(tmp).expect("mkdir tmp root");

    // --- main checkout ---
    let checkout = tmp.join("checkout");
    fs::create_dir_all(&checkout).expect("mkdir checkout");
    run_git(&checkout, &["init", "-q", "-b", "main"]);
    run_git(&checkout, &["config", "commit.gpgsign", "false"]);
    fs::write(checkout.join("README.md"), b"fixture checkout\n").expect("write README");
    run_git(&checkout, &["add", "README.md"]);
    run_git(&checkout, &["commit", "-q", "-m", "initial commit"]);
    let checkout_name = checkout
        .file_name()
        .and_then(|n| n.to_str())
        .expect("checkout has a name")
        .to_string();

    // node_modules/: ~3 MB across several files
    let node_modules = checkout.join("node_modules");
    let node_modules_bytes = write_files(&node_modules, 6, 3 * 1024 * 1024);

    // target/: ~2 MB
    let target_dir = checkout.join("target");
    let target_bytes = write_files(&target_dir, 4, 2 * 1024 * 1024);

    // dist/: ~1 MB
    let dist_dir = checkout.join("dist");
    let dist_bytes = write_files(&dist_dir, 2, 1024 * 1024);

    // --- linked worktree ---
    let linked_worktree = tmp.join("checkout-linked");
    run_git(
        &checkout,
        &[
            "worktree",
            "add",
            "-q",
            linked_worktree
                .to_str()
                .expect("linked worktree path is utf8"),
            "-b",
            "linked-branch",
        ],
    );

    // --- nested second repo, with its own build/ ---
    let nested_repo = checkout.join("nested-repo");
    fs::create_dir_all(&nested_repo).expect("mkdir nested repo");
    run_git(&nested_repo, &["init", "-q", "-b", "main"]);
    run_git(&nested_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(nested_repo.join("README.md"), b"nested fixture repo\n")
        .expect("write nested README");
    run_git(&nested_repo, &["add", "README.md"]);
    run_git(&nested_repo, &["commit", "-q", "-m", "initial commit"]);
    let nested_repo_build = nested_repo.join("build");
    let nested_repo_build_bytes = write_files(&nested_repo_build, 2, 512 * 1024);

    // --- shared cache dir outside any repo ---
    let shared_cache = tmp.join("cache").join(".cargo-registry");
    let shared_cache_bytes = write_files(&shared_cache, 3, 1024 * 1024);

    // --- loose files outside any repo ---
    let loose_dir = tmp.join("loose");
    let loose_file = loose_dir.join("orphan.bin");
    let loose_file_bytes = 4096u64;
    write_pattern(&loose_file, loose_file_bytes);

    // --- mocked `docker system df -v` JSON ---
    let docker_facts = tmp.join("docker_system_df_v.json");
    let docker_json = serde_json::json!({
        "Images": [
            {
                "ID": "sha256:aaaa000000000000000000000000000000000000000000000000000000aa",
                "Repository": "fixture/with-project",
                "Tag": "latest",
                "Size": "10485760",
                "SharedSize": "2097152",
                "UniqueSize": "8388608",
                "Labels": format!("com.docker.compose.project={checkout_name}"),
            },
            {
                "ID": "sha256:bbbb000000000000000000000000000000000000000000000000000000bb",
                "Repository": "fixture/no-join",
                "Tag": "latest",
                "Size": "5242880",
                "SharedSize": "1048576",
                "UniqueSize": "4194304",
                "Labels": "",
            }
        ]
    });
    fs::write(
        &docker_facts,
        serde_json::to_vec_pretty(&docker_json).expect("serialize docker fixture"),
    )
    .expect("write docker fixture");

    Fixture {
        root: tmp.to_path_buf(),
        checkout,
        checkout_name,
        linked_worktree,
        node_modules,
        node_modules_bytes,
        target_dir,
        target_bytes,
        dist_dir,
        dist_bytes,
        nested_repo,
        nested_repo_build,
        nested_repo_build_bytes,
        shared_cache,
        shared_cache_bytes,
        loose_file,
        loose_file_bytes,
        docker_facts,
    }
}
