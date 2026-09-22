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

/// How many `projects/<encoded-cwd>/` container directories the
/// synthetic home has. The unit of unchanged work after the 2026-09-22
/// container-level reuse: every cost assertion below is a multiple of
/// this, never of `SESSIONS`.
const CONTAINERS: usize = 5;

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
    for bucket in 0..CONTAINERS {
        let dir = projects.join(format!("-bucket-{bucket}"));
        fs::create_dir_all(&dir).unwrap();
        for i in 0..(SESSIONS / CONTAINERS) {
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
        "agent home, {SESSIONS} sessions in {CONTAINERS} containers: first pass \
         {first_elapsed:?} ({} header bytes, {} dirs listed, {} files statted, {} containers \
         identified); unchanged second pass {second_elapsed:?} ({} header bytes, {} dirs \
         listed, {} files statted, {} containers reused)",
        first.header_bytes_read,
        first.dirs_listed,
        first.files_statted,
        first.containers_identified,
        second.header_bytes_read,
        second.dirs_listed,
        second.files_statted,
        second.containers_reused,
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
    // THE SECOND CLOSED GAP (2026-09-22, stack/12). Zero header bytes was
    // never the whole cost: the per-file identification cache's validity
    // key is each session file's own `(len, mtime_ns, ctime_ns, inode)`,
    // so *knowing* a session was unchanged still cost one `stat` per
    // session and one listing per container. The reviewer's cost report
    // measured 10,580 stats and 46 listings on an unchanged pass.
    //
    // Container-level reuse (`crate::agents::ContainerCache`) replays a
    // `projects/<encoded-cwd>/` whose watched directories all carry the
    // stamps they did last pass. The assertions below are the handoff's
    // words turned into numbers: unchanged work scales with *containers*,
    // not with files. Both are strict multiples of the container count --
    // "fewer than last time" would pass with a reuse that worked for four
    // containers in five.
    assert_eq!(
        second.containers_reused, CONTAINERS as u64,
        "every unchanged container must be replayed from the store: {} of {CONTAINERS}",
        second.containers_reused
    );
    assert_eq!(
        second.containers_identified, 0,
        "no container may be re-identified when nothing changed: {}",
        second.containers_identified
    );
    assert!(
        second.dirs_listed <= CONTAINERS as u64,
        "an unchanged agent home must not list more directories than it has containers: {} \
         listings over {CONTAINERS} containers and {SESSIONS} sessions",
        second.dirs_listed
    );
    // One `stat` per watched directory per container, plus the entries of
    // the handful of home-level listings above. The bound that matters is
    // that it does not grow with `SESSIONS`: 10 x CONTAINERS is still two
    // orders of magnitude below one stat per session.
    assert!(
        second.files_statted <= 10 * CONTAINERS as u64,
        "an unchanged agent home must cost stats per container, not per session: {} stats \
         over {CONTAINERS} containers and {SESSIONS} sessions (first pass: {})",
        second.files_statted,
        first.files_statted
    );
    assert_eq!(
        second.identification_cache_hits, 0,
        "a replayed container makes no per-file derivation at all, so there is nothing for \
         the derivation cache to answer: {} hits",
        second.identification_cache_hits
    );
}

/// One appended session re-lists exactly its own container and leaves
/// every other container replayed. The counterpart to the test above:
/// reuse that cannot notice a change is not reuse, and reuse that
/// notices a change by re-identifying *everything* is not container
/// reuse.
#[test]
fn appending_one_session_re_identifies_exactly_one_container() {
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
    let (before, _) = measure(|| observe_agents(&scope, store.path(), 1_000));

    let repo = root.join("repo");
    fs::write(
        home.join("projects/-bucket-3/aaaa-99999999-4000-8000-000000000000.jsonl"),
        format!("{{\"type\":\"user\",\"cwd\":\"{}\"}}\n", repo.display()),
    )
    .unwrap();

    let (after, third) = measure(|| observe_agents(&scope, store.path(), 3_000));
    println!(
        "one appended session: {} containers identified, {} reused, {} dirs listed, {} files \
         statted, {} header bytes",
        third.containers_identified,
        third.containers_reused,
        third.dirs_listed,
        third.files_statted,
        third.header_bytes_read,
    );
    assert_eq!(after, before + 1, "the appended session must be reported");
    assert_eq!(
        third.containers_identified, 1,
        "exactly the changed container may be re-identified: {}",
        third.containers_identified
    );
    assert_eq!(
        third.containers_reused,
        CONTAINERS as u64 - 1,
        "every other container must still be replayed: {}",
        third.containers_reused
    );
    assert_eq!(
        third.identification_cache_misses, 1,
        "exactly one derivation may miss the per-file cache: the session that is actually new"
    );
}

/// The limit this reuse has, asserted rather than only written down: a
/// session rewritten **in place** does not move its container's stamp,
/// so its new byte total is not seen until something else in that
/// container changes. Stated on `crate::agents::ContainerCache` and in
/// `.oh/guardrails/no-second-traversal-on-report-path.md`; measured here,
/// because a documented limit nobody tests is a documented guess.
#[test]
fn a_session_rewritten_in_place_is_not_seen_until_its_container_moves() {
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
    let bytes = |at: u64| -> u64 {
        swamp_core::agents::discover_and_measure(
            &scope,
            &[],
            Some(store.path()),
            true,
            at,
            30,
            3600,
        )
        .expect("agent discovery")
        .iter()
        .map(|u| u.bytes)
        .sum()
    };
    let first = bytes(1_000);

    // Same name, same directory: the file grows, its parent's mtime and
    // ctime do not move.
    let victim = home.join("projects/-bucket-1/0001-00000000-4000-8000-000000000000.jsonl");
    let grown = format!(
        "{{\"type\":\"user\",\"cwd\":\"{}\"}}\n{}\n",
        root.join("repo").display(),
        "x".repeat(50_000)
    );
    fs::write(&victim, &grown).unwrap();
    let second = bytes(2_000);
    assert_eq!(
        second, first,
        "recorded limit: an in-place rewrite does not move its container's stamp, so the \
         stored byte total stands until the container changes"
    );

    // Anything that changes the container's shape re-identifies it, and
    // the rewritten file's real size is reported then.
    fs::write(
        home.join("projects/-bucket-1/zzzz-00000000-4000-8000-000000000000.jsonl"),
        b"{}\n",
    )
    .unwrap();
    let third = bytes(3_000);
    assert!(
        third > first + 40_000,
        "once the container is re-identified the rewritten session's real size is reported: \
         {third} vs {first}"
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
