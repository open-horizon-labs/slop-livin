//! What an *unchanged* observation actually costs
//! (`.oh/guardrails/no-second-traversal-on-report-path.md`).
//!
//! The handoff requires unchanged work to scale with roots and changed
//! containers rather than with all files, and the 2026-09-21 review's P1
//! was that it does not: external roots were re-sized recursively on
//! every call, and every adapter re-read every session header. "Bounded
//! reads are enough" is not a measurement, so this file measures --
//! through `swamp_core::work_counters`, which counts directory listings,
//! stats and header bytes.
//!
//! These tests run single-threaded: the counters are process-global, and
//! two tests measuring at once would measure each other.
//!
//! Fixtures are disposable `tempfile` trees with synthetic content. No
//! real tool home, transcript or credential is read; the "session
//! headers" here are one-line JSON objects this test wrote itself.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use swamp_core::locations::{Environment, Platform, Registry};
use swamp_core::scope::{EffectiveScope, ScanConfig, resolve_effective_scope};
use swamp_core::work_counters::{self, WorkCounters};

/// Enough sessions that a per-session cost is unmistakable in the
/// counters. The review's objection to the existing evidence was
/// precisely that 300 sessions is not a large tool home.
const SESSIONS: usize = 5_000;

/// Enough files that a recursive re-measurement of an unchanged external
/// root would be obvious.
const EXTERNAL_FILES: usize = 20_000;

/// The work counters are process-global, so two tests measuring at once
/// would measure each other. Serializing the file on one lock makes the
/// numbers trustworthy under the default harness rather than relying on
/// every caller passing `--test-threads=1`.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn only(keep: &[&str]) -> ScanConfig {
    let registry = Registry::with_builtins();
    ScanConfig {
        defaults: false,
        disabled_detectors: registry
            .detectors()
            .iter()
            .map(|d| d.id().to_string())
            .filter(|id| !keep.contains(&id.as_str()))
            .collect(),
        ..Default::default()
    }
}

fn scope_with(env_vars: HashMap<String, String>, home: &Path, keep: &[&str]) -> EffectiveScope {
    let env = Environment::fixture(home.to_path_buf(), env_vars, Platform::MacOS);
    resolve_effective_scope(&env, &only(keep), &[], &Registry::with_builtins(), 1_000)
}

/// A synthetic Claude Code home: `SESSIONS` transcripts across a handful
/// of project directories, each a single JSON header line.
fn synthetic_claude_home(root: &Path) -> PathBuf {
    let home = root.join("claude");
    let projects = home.join("projects");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    // A few containers, so "re-list only the changed container" is
    // measurable rather than vacuous.
    for bucket in 0..5 {
        let dir = projects.join(format!("-bucket-{bucket}"));
        fs::create_dir_all(&dir).unwrap();
        for i in 0..(SESSIONS / 5) {
            let id = format!("{bucket:04}-{i:08}-4000-8000-000000000000");
            fs::write(
                dir.join(format!("{id}.jsonl")),
                format!(
                    "{{\"type\":\"user\",\"sessionId\":\"{id}\",\"cwd\":\"{}\"}}\n",
                    repo.display()
                ),
            )
            .unwrap();
        }
    }
    home
}

fn measure<T>(f: impl FnOnce() -> T) -> (T, WorkCounters) {
    let before = work_counters::snapshot();
    let out = f();
    (out, work_counters::since(before))
}

fn observe_agents(scope: &EffectiveScope, store: &Path, at: u64) -> usize {
    swamp_core::agents::discover_and_measure(scope, &[], Some(store), true, at, 30, 3600)
        .expect("agent discovery")
        .len()
}

#[test]
fn a_large_agent_home_reads_headers_once_and_not_again() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let home = synthetic_claude_home(&root);
    let scope = scope_with(
        HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string())]),
        &root,
        &["claude-code"],
    );
    let store = tempfile::tempdir().unwrap();

    let started = std::time::Instant::now();
    let (units, first) = measure(|| observe_agents(&scope, store.path(), 1_000));
    let first_elapsed = started.elapsed();
    assert!(
        units > 0,
        "precondition: the synthetic home must produce units"
    );
    assert!(
        first.header_bytes_read > 0,
        "the first observation has to read the session headers it identifies from"
    );

    let started = std::time::Instant::now();
    let (_, second) = measure(|| observe_agents(&scope, store.path(), 2_000));
    let second_elapsed = started.elapsed();

    // Reported rather than only asserted: a number in the session note
    // is worth more than a pass/fail here.
    println!(
        "agent home, {SESSIONS} sessions: first pass {first_elapsed:?} \
         ({} header bytes, {} dirs listed); unchanged second pass {second_elapsed:?} \
         ({} header bytes, {} dirs listed)",
        first.header_bytes_read, first.dirs_listed, second.header_bytes_read, second.dirs_listed
    );

    // THE CLOSED GAP. The previous repair measured, and recorded,
    // 740 KB of header reads on an *unchanged* second pass over this
    // fixture -- every one of the 5,000 sessions re-read to re-derive a
    // `cwd` that had not moved. The identification cache arrived with
    // the `AgentAdapter` trait, keyed on each session file's own
    // `(len, mtime_ns, ctime_ns, inode)` plus the adapter version, so an
    // unchanged session is a table lookup.
    //
    // This assertion is strict on purpose. "Fewer" would pass with a
    // cache that worked for four sessions in five, and the number this
    // replaces was exactly the kind of "bounded reads are enough" claim
    // the review rejected.
    assert_eq!(
        second.header_bytes_read, 0,
        "an unchanged agent home must cost zero header bytes: {} sessions, {} bytes on the \
         first pass, {} on the second",
        SESSIONS, first.header_bytes_read, second.header_bytes_read
    );
    assert!(
        second.identification_cache_hits >= SESSIONS as u64,
        "every session must be answered from the cache, not merely skipped: {} hits over {} \
         sessions",
        second.identification_cache_hits,
        SESSIONS
    );
}

#[test]
fn appending_one_session_reads_exactly_one_header() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let home = synthetic_claude_home(&root);
    let scope = scope_with(
        HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string())]),
        &root,
        &["claude-code"],
    );
    let store = tempfile::tempdir().unwrap();
    let _ = measure(|| observe_agents(&scope, store.path(), 1_000));

    let repo = root.join("repo");
    fs::write(
        home.join("projects/-bucket-0/aaaa-99999999-4000-8000-000000000000.jsonl"),
        format!("{{\"type\":\"user\",\"cwd\":\"{}\"}}\n", repo.display()),
    )
    .unwrap();

    let (_, third) = measure(|| observe_agents(&scope, store.path(), 3_000));
    println!(
        "one appended session: {} header bytes, {} dirs listed",
        third.header_bytes_read, third.dirs_listed
    );
    assert!(
        third.header_bytes_read > 0,
        "the new session's header must be read"
    );
    // Exactly one header read: the appended session, and nothing else.
    // The bound is one capped read, not `SESSIONS + 1` of them -- the
    // weaker form would have passed while every unchanged session was
    // still being re-read.
    assert!(
        third.header_bytes_read <= swamp_core::agents::bounded_io::MAX_HEADER_BYTES as u64,
        "one appended session costs one capped header read; {} bytes means the unchanged \
         sessions were re-read too",
        third.header_bytes_read
    );
    assert_eq!(
        third.identification_cache_misses, 1,
        "exactly one derivation may miss the cache: the session that is actually new"
    );
}

#[test]
fn an_unchanged_external_cache_root_is_not_re_traversed() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let cargo_home = root.join("cargo-home");
    let registry = cargo_home.join("registry/cache/index.crates.io-abc");
    fs::create_dir_all(&registry).unwrap();
    for i in 0..EXTERNAL_FILES {
        fs::write(registry.join(format!("crate-{i}.crate")), b"x").unwrap();
    }
    let scope = scope_with(
        HashMap::from([("CARGO_HOME".to_string(), cargo_home.display().to_string())]),
        &root,
        &["cargo-home"],
    );
    let store = tempfile::tempdir().unwrap();

    let started = std::time::Instant::now();
    let (units, first) = measure(|| {
        swamp_core::external::discover_and_measure(
            &scope,
            Some(store.path()),
            true,
            1_000,
            30,
            3600,
        )
        .expect("external discovery")
        .len()
    });
    let first_elapsed = started.elapsed();
    assert!(units > 0, "precondition: the Cargo home must be measured");

    let started = std::time::Instant::now();
    let (_, second) = measure(|| {
        swamp_core::external::discover_and_measure(
            &scope,
            Some(store.path()),
            true,
            2_000,
            30,
            3600,
        )
        .expect("external discovery")
        .len()
    });
    let second_elapsed = started.elapsed();

    println!(
        "external cache root, {EXTERNAL_FILES} files: first pass {first_elapsed:?} \
         ({} dirs listed, {} files statted); second pass {second_elapsed:?} \
         ({} dirs listed, {} files statted)",
        first.dirs_listed, first.files_statted, second.dirs_listed, second.files_statted
    );

    assert!(
        first.dirs_listed > 0,
        "the first pass must be counted, or the counters are not wired"
    );
    // THE CLOSED GAP (2026-09-22). External unit bytes used to come from
    // a fresh recursive folded measurement on every pass; this file's
    // previous version asserted only that the work was *counted*,
    // deliberately, so the gap stayed visible. It is now closed:
    // `folded_measurement::reuse_folded_measurement` answers an
    // unchanged unit from the folded rows the previous pass persisted,
    // paying one `stat` per directory and no listing at all.
    //
    // Strict on purpose. "Fewer" would pass with a reuse that worked for
    // the small directories and re-walked the big one.
    assert_eq!(
        second.dirs_listed, 0,
        "an unchanged external root must not re-list a single directory: {} listings over {} \
         files",
        second.dirs_listed, EXTERNAL_FILES
    );
    // The stat cost is the directory count, not the file count: that is
    // the whole claim ("scales with roots and changed containers, not
    // all files"), and a bound of "fewer than the first pass" would not
    // show it.
    assert!(
        second.files_statted < 100,
        "an unchanged external root costs one stat per directory, not per file: {} stats over \
         {} files (first pass: {})",
        second.files_statted,
        EXTERNAL_FILES,
        first.files_statted
    );
}

/// The reuse must not survive a change it cannot see the inside of: a
/// file added under the unit costs a real re-measurement, and the new
/// bytes are reported.
#[test]
fn a_changed_external_cache_root_is_measured_again() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let cargo_home = root.join("cargo-home");
    let registry = cargo_home.join("registry/cache/index.crates.io-abc");
    fs::create_dir_all(&registry).unwrap();
    for i in 0..50 {
        fs::write(registry.join(format!("crate-{i}.crate")), b"x").unwrap();
    }
    let scope = scope_with(
        HashMap::from([("CARGO_HOME".to_string(), cargo_home.display().to_string())]),
        &root,
        &["cargo-home"],
    );
    let store = tempfile::tempdir().unwrap();
    let measure_pass = |at: u64| {
        swamp_core::external::discover_and_measure(&scope, Some(store.path()), true, at, 30, 3600)
            .expect("external discovery")
            .into_iter()
            .map(|u| u.bytes)
            .sum::<u64>()
    };
    let (first_bytes, _) = measure(|| measure_pass(1_000));
    let (_, second) = measure(|| measure_pass(2_000));
    assert_eq!(
        second.dirs_listed, 0,
        "precondition: the unchanged pass reuses"
    );

    fs::write(registry.join("added.crate"), vec![b'x'; 200_000]).unwrap();
    let (third_bytes, third) = measure(|| measure_pass(3_000));
    assert!(
        third.dirs_listed > 0,
        "a directory whose contents changed must be listed again"
    );
    assert!(
        third_bytes > first_bytes,
        "the added file's bytes must be reported: {third_bytes} vs {first_bytes}"
    );
}

#[test]
fn the_work_counters_themselves_are_wired() {
    let _serial = serial();
    // A guard against the whole file passing vacuously: if nothing
    // increments the counters, every assertion above is meaningless.
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("a"), b"x").unwrap();
    let (_, counted) = measure(|| swamp_core::locations::shallow_list(tmp.path()));
    assert_eq!(counted.dirs_listed, 1);
    assert_eq!(counted.files_statted, 1);
}
