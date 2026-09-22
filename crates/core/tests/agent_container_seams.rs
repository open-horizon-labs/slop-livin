//! The container seam, for every adapter whose session storage is a
//! directory tree: Codex (`sessions/<yyyy>/<mm>/<dd>/`), OpenCode
//! (`storage/session/<project-id>/`) and Pi (`sessions/<dir>/`).
//!
//! Two properties per adapter, both adversarial:
//!
//! * a container this pass's event window vouches for is **replayed** --
//!   no listing, no `stat`, no header read -- and reports the same units
//!   it did when it was identified;
//! * a container the window names is **re-identified**, and the change
//!   is reported in the same pass. A reuse that cannot notice a change
//!   is not reuse.
//!
//! Plus the property that made the conversion possible at all: one
//! container's output does not depend on what a sibling container
//! produced. Codex's session walk used to carry a single entry budget
//! shared across `sessions/` and `archived_sessions/`
//! (`already_seen + out.len()`), so a day directory's contents depended
//! on how many files the days before it had yielded -- which means the
//! rows stored for that day could not be replayed into a pass that
//! reached it in a different order.
//!
//! Fixtures are disposable `tempfile` trees with synthetic content: the
//! "transcripts" here are one-line JSON objects this test wrote itself.

use std::fs;
use std::path::{Path, PathBuf};
use swamp_core::agents::{ContainerCache, IdentificationCache, IdentifyCtx};
use swamp_core::fs_events::EventCoverage;
use swamp_core::work_counters::{self, WorkCounters};

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn append(path: &Path, bytes: usize) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(&vec![b'x'; bytes]).unwrap();
    f.write_all(b"\n").unwrap();
}

/// One identification pass with a fresh pair of caches loaded from
/// `store`, under `coverage`, returning the units' total bytes, their
/// count, and the work the pass did.
fn pass(
    identify: &dyn Fn(&Path, &IdentifyCtx) -> Vec<swamp_core::agents::CandidateAgentUnit>,
    home: &Path,
    store: &Path,
    at: u64,
    coverage: EventCoverage,
) -> (u64, usize, WorkCounters) {
    let cache = IdentificationCache::load(store);
    let containers = ContainerCache::load(store, coverage);
    let (units, counted) = work_counters::measured(|| {
        let ctx = IdentifyCtx::with_containers(at, &cache, &containers);
        identify(home, &ctx)
    });
    cache.save(store, at).unwrap();
    containers.save(store, at).unwrap();
    let bytes = units.iter().map(|u| u.bytes).sum();
    (bytes, units.len(), counted)
}

fn quiet(root: &Path, since: u64) -> EventCoverage {
    EventCoverage::trusted(root.to_path_buf(), Vec::new(), since)
}

fn touched(root: &Path, changed: &[PathBuf], since: u64) -> EventCoverage {
    EventCoverage::trusted(root.to_path_buf(), changed.to_vec(), since)
}

/// The shared body of all three adapter tests: identify once cold,
/// replay under a quiet window, then append and re-identify under a
/// window that names the appended file.
fn seam_behaves(
    identify: &dyn Fn(&Path, &IdentifyCtx) -> Vec<swamp_core::agents::CandidateAgentUnit>,
    root: &Path,
    home: &Path,
    containers: u64,
    victim: &Path,
    victim_container: &Path,
) {
    let store = tempfile::tempdir().unwrap();
    let (cold_bytes, cold_units, cold) = pass(
        identify,
        home,
        store.path(),
        1_000,
        EventCoverage::untrusted(),
    );
    assert!(
        cold_units > 0,
        "precondition: the fixture must be identified"
    );
    assert_eq!(
        cold.containers_identified, containers,
        "every container must be identified on the cold pass: {} of {containers}",
        cold.containers_identified
    );

    let (quiet_bytes, quiet_units, replayed) =
        pass(identify, home, store.path(), 2_000, quiet(root, 1_000));
    assert_eq!(
        (quiet_bytes, quiet_units),
        (cold_bytes, cold_units),
        "a replayed home must report exactly what it reported when identified"
    );
    assert_eq!(
        replayed.containers_reused, containers,
        "every container the window vouches for must be replayed: {} of {containers}",
        replayed.containers_reused
    );
    assert_eq!(
        replayed.containers_identified, 0,
        "nothing may be re-identified when the window reports no event: {}",
        replayed.containers_identified
    );
    assert_eq!(
        replayed.header_bytes_read, 0,
        "a replayed container reads no headers: {}",
        replayed.header_bytes_read
    );

    // An in-place append: the container's own directory stamp does not
    // move, which is exactly the change the pre-2026-09-22 stamp key
    // could not see.
    let before = fs::metadata(victim).unwrap().len();
    append(victim, 4096);
    assert!(fs::metadata(victim).unwrap().len() >= before + 4096);

    let events = touched(
        root,
        &[victim.to_path_buf(), victim_container.to_path_buf()],
        2_000,
    );
    let (grown_bytes, _, cost) = pass(identify, home, store.path(), 3_000, events);
    assert_eq!(
        cost.containers_identified, 1,
        "exactly the container the window named may be re-identified: {} identified, {} reused",
        cost.containers_identified, cost.containers_reused
    );
    assert_eq!(
        cost.containers_reused,
        containers - 1,
        "every other container must still be replayed: {}",
        cost.containers_reused
    );
    assert!(
        grown_bytes >= cold_bytes + 4096,
        "the appended bytes must be reported in the same pass that sees their event: \
         {grown_bytes} vs {cold_bytes}"
    );
}

#[test]
fn codex_day_directories_are_containers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let home = root.join("codex");
    let mut victim = PathBuf::new();
    for (month, day) in [("01", "05"), ("02", "11")] {
        for i in 0..4 {
            let p = home
                .join("sessions/2026")
                .join(month)
                .join(day)
                .join(format!("rollout-{month}{day}-{i}.jsonl"));
            write(
                &p,
                &format!("{{\"cwd\":\"{}\",\"type\":\"user\"}}\n", root.display()),
            );
            if month == "01" && i == 0 {
                victim = p;
            }
        }
    }
    seam_behaves(
        &swamp_core::agents::codex::identify,
        &root,
        &home,
        2,
        &victim,
        &home.join("sessions/2026/01/05"),
    );
}

#[test]
fn opencode_project_directories_are_containers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let home = root.join("opencode");
    let mut victim = PathBuf::new();
    for project in ["p1", "p2"] {
        for i in 0..3 {
            let p = home
                .join("storage/session")
                .join(project)
                .join(format!("s{i}.json"));
            write(&p, &format!("{{\"id\":\"s{i}\"}}\n"));
            if project == "p1" && i == 0 {
                victim = p;
            }
        }
    }
    seam_behaves(
        &swamp_core::agents::opencode::identify,
        &root,
        &home,
        2,
        &victim,
        &home.join("storage/session/p1"),
    );
}

#[test]
fn pi_session_directories_are_containers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let home = root.join("pi");
    write(&home.join("settings.json"), "{}\n");
    let mut victim = PathBuf::new();
    for bucket in ["a", "b"] {
        for i in 0..3 {
            let p = home
                .join("sessions")
                .join(bucket)
                .join(format!("s{i}.jsonl"));
            write(
                &p,
                &format!("{{\"cwd\":\"{}\",\"type\":\"user\"}}\n", root.display()),
            );
            if bucket == "a" && i == 0 {
                victim = p;
            }
        }
    }
    seam_behaves(
        &swamp_core::agents::pi::identify,
        &root,
        &home,
        2,
        &victim,
        &home.join("sessions/a"),
    );
}

/// The property the shared entry bound violated, and the reason Codex
/// could not be converted before: what one container identifies must not
/// depend on what a sibling produced first.
///
/// Stated as an equality over live identifications, because that is the
/// form a replay needs: rows stored for `2026/02/11` are replayed into a
/// pass that may reach it in a different order, or after a sibling grew
/// by thousands of files, and they are only sound if the day's own
/// contents are all that decided them.
#[test]
fn a_codex_day_container_does_not_depend_on_its_siblings() {
    let day_units = |busy_sibling: usize| -> Vec<String> {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("codex");
        for i in 0..busy_sibling {
            write(
                &home.join(format!("sessions/2026/01/05/rollout-a-{i}.jsonl")),
                "{\"type\":\"user\"}\n",
            );
        }
        // The same sharing applied across `sessions/` and
        // `archived_sessions/`, so the archived tree is populated too.
        for i in 0..busy_sibling {
            write(
                &home.join(format!("archived_sessions/2026/01/05/rollout-b-{i}.jsonl")),
                "{\"type\":\"user\"}\n",
            );
        }
        for i in 0..3 {
            write(
                &home.join(format!("sessions/2026/02/11/rollout-c-{i}.jsonl")),
                "{\"type\":\"user\"}\n",
            );
        }
        let cache = IdentificationCache::disabled();
        let containers = ContainerCache::disabled();
        let ctx = IdentifyCtx::with_containers(1_000, &cache, &containers);
        let mut names: Vec<String> = swamp_core::agents::codex::identify(&home, &ctx)
            .into_iter()
            .filter(|u| u.path.to_string_lossy().contains("2026/02/11"))
            .map(|u| u.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    };
    let alone = day_units(0);
    let crowded = day_units(500);
    assert_eq!(alone.len(), 3, "precondition: the day has three sessions");
    assert_eq!(
        alone, crowded,
        "a day container's units must not depend on how many files its siblings produced"
    );
}

/// Oh My Pi has a session tree and deliberately does **not** use the
/// container seam. Recorded here rather than left as an omission a
/// reader has to notice.
///
/// Its session identification produces a home-wide aggregate that a
/// per-container replay cannot reconstruct: each session's body
/// contributes references to the shared blob store, and the reference
/// count reported for a blob is the sum across every session. A pass
/// that replayed some containers would count only the sessions it
/// identified, so the number it printed would be *wrong* rather than
/// unknown -- and "a number that is wrong" is the failure this catalog's
/// guardrails exist to prevent. The honest alternatives are to carry the
/// per-unit blob references through the container rows (a stored-shape
/// change) or to report every blob count as unknown on any reused pass
/// (a user-visible regression); neither was taken here.
#[test]
fn oh_my_pi_declares_why_it_does_not_use_the_container_seam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let home = root.join("omp");
    write(&home.join("settings.json"), "{}\n");
    for i in 0..3 {
        write(
            &home.join(format!("sessions/a/s{i}.jsonl")),
            "{\"type\":\"user\"}\n",
        );
    }
    let store = tempfile::tempdir().unwrap();
    let _ = pass(
        &swamp_core::agents::oh_my_pi::identify,
        &home,
        store.path(),
        1_000,
        EventCoverage::untrusted(),
    );
    let (_, _, second) = pass(
        &swamp_core::agents::oh_my_pi::identify,
        &home,
        store.path(),
        2_000,
        quiet(&root, 1_000),
    );
    assert_eq!(
        (second.containers_reused, second.containers_identified),
        (0, 0),
        "Oh My Pi must not silently acquire the container seam without the blob-reference \
         aggregate being carried through it: {second:?}"
    );
}
