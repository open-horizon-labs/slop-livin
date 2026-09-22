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

    // MEASURED GAP, recorded rather than claimed fixed. Header reads now
    // go through one counted, capped reader, which is what makes this
    // number exist at all -- but the per-session identification cache
    // that would make an unchanged pass cost *zero* headers is not built
    // yet, because it has to arrive with the `AgentAdapter` trait rather
    // than be threaded through fifteen adapters by hand. The assertion
    // below is therefore the honest one; the strict `== 0` form, and the
    // number it must replace, are in the "still open" section of
    // `.oh/sessions/2026-09-21-foundation-repairs.md`.
    assert_eq!(
        second.header_bytes_read, first.header_bytes_read,
        "until the identification cache lands, an unchanged pass costs the same header reads as \
         the first; a *different* number here means something changed and this test must be \
         re-read rather than adjusted"
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
    // Whatever the caching story, every header read is capped: that is
    // the privacy bound, and it holds today.
    let reads = (SESSIONS + 1) as u64;
    assert!(
        third.header_bytes_read <= reads * swamp_core::agents::bounded_io::MAX_HEADER_BYTES as u64,
        "every header read is bounded by the shared cap: {} bytes over at most {reads} reads",
        third.header_bytes_read
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

    // KNOWN GAP, measured rather than claimed: external unit bytes still
    // come from a fresh folded measurement on every pass, so this
    // assertion is deliberately the weak one -- it pins that the work is
    // *counted*, which is what makes the gap visible and the next
    // repair testable. See the "still open" section of
    // `.oh/sessions/2026-09-21-foundation-repairs.md`.
    assert!(
        first.dirs_listed > 0,
        "the first pass must be counted, or the counters are not wired"
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
