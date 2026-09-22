//! Mandate item 4: two unchanged full observations over a multi-ecosystem
//! fixture with a 5,000-session synthetic agent home and a 20k-file
//! external cache. Disposable fixtures only.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{EffectiveScope, ScanConfig, resolve_effective_scope};
use swamp_core::work_counters::{self, WorkCounters};

const SESSIONS: usize = 5_000;
const EXTERNAL_FILES: usize = 20_000;

fn shim_dir(dir: &Path, counter: &Path, names: &[&str]) -> String {
    fs::create_dir_all(dir).unwrap();
    for n in names {
        let p = dir.join(n);
        fs::write(
            &p,
            format!("#!/bin/sh\necho \"{n}\" >> {}\nexit 1\n", counter.display()),
        )
        .unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&p, perms).unwrap();
    }
    let prev = std::env::var("PATH").unwrap_or_default();
    unsafe { std::env::set_var("PATH", format!("{}:{prev}", dir.display())) };
    prev
}

fn spawns(counter: &Path) -> Vec<String> {
    fs::read_to_string(counter)
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .collect()
}

fn fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    // Agent home: 5,000 Claude Code sessions across 5 projects.
    let claude = root.join("claude");
    for p in 0..5 {
        let proj = claude.join(format!("projects/-Users-dev-proj{p}"));
        fs::create_dir_all(&proj).unwrap();
        for s in 0..(SESSIONS / 5) {
            fs::write(
                proj.join(format!("sess-{p}-{s}.jsonl")),
                format!("{{\"cwd\":\"/Users/dev/proj{p}\",\"type\":\"user\"}}\n"),
            )
            .unwrap();
        }
    }
    fs::create_dir_all(claude.join("debug")).unwrap();
    fs::write(claude.join("debug/log.txt"), vec![b'x'; 4096]).unwrap();

    // External cache: 20,000 files in a Cargo registry cache.
    let cargo_home = root.join("cargo-home");
    let registry = cargo_home.join("registry/cache/index.crates.io-abc");
    fs::create_dir_all(&registry).unwrap();
    for i in 0..EXTERNAL_FILES {
        fs::write(registry.join(format!("crate-{i}.crate")), b"x").unwrap();
    }

    // A second ecosystem: an npm cache and a model store.
    let npm = root.join("npm-cache/_cacache/content-v2");
    fs::create_dir_all(&npm).unwrap();
    for i in 0..500 {
        fs::write(npm.join(format!("blob-{i}")), vec![b'y'; 64]).unwrap();
    }
    let hf = root.join("hf/hub/models--x--y/blobs");
    fs::create_dir_all(&hf).unwrap();
    for i in 0..200 {
        fs::write(hf.join(format!("b-{i}")), vec![b'z'; 1024]).unwrap();
    }

    // An ordinary project root, so the walker runs too.
    let src = root.join("src");
    fs::create_dir_all(src.join("p/target/debug")).unwrap();
    fs::write(src.join("p/target/debug/blob.bin"), vec![b'x'; 65536]).unwrap();
    fs::write(src.join("p/Cargo.toml"), b"[package]\nname=\"x\"\n").unwrap();
    (claude, cargo_home, src)
}

fn scope_for(root: &Path, claude: &Path, cargo_home: &Path, src: &Path) -> EffectiveScope {
    let registry = Registry::with_builtins();
    let keep = ["claude-code", "cargo-home", "npm", "huggingface"];
    let cfg = ScanConfig {
        defaults: false,
        include: vec![src.display().to_string()],
        disabled_detectors: registry
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .filter(|id| !keep.contains(&id.as_str()))
            .collect(),
        ..Default::default()
    };
    let env = Environment::fixture(
        root.to_path_buf(),
        HashMap::from([
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                claude.display().to_string(),
            ),
            ("CARGO_HOME".to_string(), cargo_home.display().to_string()),
            (
                "npm_config_cache".to_string(),
                root.join("npm-cache").display().to_string(),
            ),
            ("HF_HOME".to_string(), root.join("hf").display().to_string()),
        ]),
        Platform::MacOS,
    );
    resolve_effective_scope(&env, &cfg, &[], &registry, 1_000)
}

fn observe(scope: &EffectiveScope, store: &Path, _at: u64) -> (usize, usize) {
    let o = swamp_core::report::observe_scope(
        scope,
        swamp_core::report::ObservationParts::ALL,
        None,
        None,
        false,
        Some(store),
        None,
        true,
        true,
        false,
        false,
        swamp_core::fs_events::platform_source().as_ref(),
        30,
        24 * 3600,
    )
    .expect("scope observation");
    (o.agent_units.len(), o.external_units.len())
}

fn measured<T>(f: impl FnOnce() -> T) -> (T, WorkCounters, std::time::Duration) {
    work_counters::reset();
    let started = std::time::Instant::now();
    let out = f();
    (out, work_counters::snapshot(), started.elapsed())
}

#[test]
fn two_unchanged_full_observations_cost_report() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (claude, cargo_home, src) = fixture(&root);
    let store = tempfile::tempdir().unwrap();
    let shims = root.join("shims");
    let counter = root.join("spawns.log");
    fs::write(&counter, b"").unwrap();
    let prev_path = shim_dir(
        &shims,
        &counter,
        &["lsof", "plutil", "xcrun", "du", "docker", "simctl", "mdls"],
    );

    let scope = scope_for(&root, &claude, &cargo_home, &src);
    let ((a1, e1), first, t1) = measured(|| observe(&scope, store.path(), 1_000));
    let after_first = spawns(&counter).len();
    let ((a2, e2), second, t2) = measured(|| observe(&scope, store.path(), 2_000));
    let all_spawns = spawns(&counter);
    unsafe { std::env::set_var("PATH", prev_path) };

    println!("--- COST REPORT (mandate item 4) ---");
    println!(
        "fixture: {SESSIONS} agent sessions, {EXTERNAL_FILES}-file cargo cache, 500-file npm cache, 200-file model store, 1 walked project root"
    );
    println!("pass 1: {a1} agent units, {e1} external units, {t1:?}");
    println!(
        "  dirs_listed={} files_statted={} header_bytes={} cache_hits={} cache_misses={}",
        first.dirs_listed,
        first.files_statted,
        first.header_bytes_read,
        first.identification_cache_hits,
        first.identification_cache_misses
    );
    println!("pass 2 (nothing changed): {a2} agent units, {e2} external units, {t2:?}");
    println!(
        "  dirs_listed={} files_statted={} header_bytes={} cache_hits={} cache_misses={}",
        second.dirs_listed,
        second.files_statted,
        second.header_bytes_read,
        second.identification_cache_hits,
        second.identification_cache_misses
    );
    println!(
        "subprocess spawns: pass 1 = {after_first}, total = {} ({:?})",
        all_spawns.len(),
        all_spawns
    );
    println!("--- END COST REPORT ---");

    assert_eq!(
        second.header_bytes_read, 0,
        "an unchanged pass must read zero session-header bytes"
    );
    assert_eq!(
        all_spawns.len(),
        0,
        "an unchanged observation must spawn no subprocesses, got {all_spawns:?}"
    );
    assert_eq!(
        second.dirs_listed, 0,
        "an unchanged pass must not re-list directories beyond the events/coverage check"
    );
    assert_eq!(
        second.files_statted, 0,
        "an unchanged pass must not re-stat measured members"
    );
}
