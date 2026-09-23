//! Live-use/occupancy evidence (#55): bounded, read-only checks for
//! whether something outside this process currently has a path open, a
//! Docker container running against it, a simulator booted on it, or a
//! manager lock held on it. Every check here answers a narrow question
//! ("does `lsof` show an open handle right now") -- never "is this safe
//! to remove". A negative result is evidence, not proof of no consumer
//! (`.oh/guardrails/activity-and-consumer-evidence-have-limits.md`).
//!
//! [`occupied`] is the original boolean gate (`execution.rs`'s protection
//! check, `agents::is_active`) and is unchanged. The functions below
//! build the structured [`crate::evidence::Evidence`] the report/action
//! layers attach; they are the same underlying signal, offered with
//! provenance and freshness rather than a bare bool.

use crate::evidence::{Evidence, EvidenceSource, FactKind, FactSubtype, FactValue, Freshness};
use std::{path::Path, time::Duration};

/// Short-lived: a process/lock/container state observed now says nothing
/// about five minutes from now. Existing action boundaries (plan
/// proposal, execute) recheck rather than trust an older observation
/// past this window.
pub const CURRENT_USE_EXPIRY_SECS: u64 = 60;

/// How long an occupancy probe may run before it is killed and reported
/// as [`OccupancyState::Unknown`]. `lsof +D` over a large directory is
/// bounded by this, never by patience.
const OCCUPANCY_TIMEOUT: Duration = Duration::from_secs(10);

/// The answer to "does anything outside this process hold this path (or
/// anything under it) open right now". Deliberately three-valued: a
/// failed, timed-out or permission-denied probe is **not** "nothing is
/// open" (`.oh/guardrails/occupancy-is-tristate-at-sinks.md`). Every
/// destructive sink matches on this and refuses on `Unknown`; nothing
/// that moves user data may consume the boolean [`occupied`] instead.
#[must_use = "an occupancy answer that is not matched on is a recheck that did not happen"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OccupancyState {
    /// The probe ran to completion and found no open handle.
    Free,
    /// The probe found at least one open handle; the payload names the
    /// member that was open (the anchor itself for a file probe).
    Occupied(std::path::PathBuf),
    /// The probe could not answer (binary missing, timed out, permission
    /// denied, unreadable directory). Treated as a refusal at every sink.
    Unknown(String),
}

impl OccupancyState {
    /// A one-line refusal cause for a sink's outcome/ledger record.
    pub fn refusal(&self) -> Option<String> {
        match self {
            Self::Free => None,
            Self::Occupied(p) => Some(format!(
                "refused: an open file handle was found on {} just now, so an active process is \
                 using it — propose again once it is closed (for a directory this answer covers \
                 everything under it)",
                p.display()
            )),
            Self::Unknown(why) => Some(format!(
                "refused: occupancy could not be determined ({why}); refusing rather than guessing that nothing is open"
            )),
        }
    }
}

/// The one bounded `lsof` probe every occupancy question in this crate
/// goes through. Directories are probed with `+D` so **every descendant**
/// is covered, not just the anchor (the review's open-cache-member
/// counterexample); files are probed directly. `stdout` and `stderr` are
/// captured separately so an ordinary `lsof` warning is not mistaken for
/// an open handle, while a permission error still becomes `Unknown`.
pub(crate) fn probe_path(path: &Path) -> OccupancyState {
    let is_dir = match crate::fs_gate::symlink_metadata(path) {
        Ok(m) => m.is_dir() && !m.file_type().is_symlink(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Nothing there to hold open. The caller's own identity
            // recheck is what refuses a vanished path.
            return OccupancyState::Free;
        }
        Err(e) => return OccupancyState::Unknown(format!("cannot stat {}: {e}", path.display())),
    };
    let flag = if is_dir { "+D" } else { "--" };
    let out = match crate::fs_gate::spawn::run(
        crate::fs_gate::spawn::Program::Lsof,
        [std::ffi::OsStr::new(flag), path.as_os_str()],
        OCCUPANCY_TIMEOUT,
    ) {
        Ok(out) => out,
        Err(e) => return OccupancyState::Unknown(format!("lsof could not be started: {e}")),
    };
    if out.timed_out {
        return OccupancyState::Unknown(format!(
            "lsof did not answer within {}s for {}",
            OCCUPANCY_TIMEOUT.as_secs(),
            path.display()
        ));
    }
    classify_lsof_exit(out.code, &out.stdout_lossy(), &out.stderr_lossy(), path)
}

/// Pure classification of a completed `lsof` run, factored out so the
/// fail-closed rules are unit-testable without spawning a process.
///
/// The distinction that matters: exit 1 with no output means "looked,
/// found nothing"; exit 1 with a permission complaint means "could not
/// look", which is `Unknown` and refuses. Treating the second as the
/// first is how a permission-denied probe came to authorize a removal.
fn classify_lsof_exit(
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
    path: &Path,
) -> OccupancyState {
    let found = !stdout.trim().is_empty();
    let denied = stderr.to_lowercase().contains("permission denied");
    match code {
        // `lsof` exits 0 when it found at least one open handle.
        Some(0) if found => OccupancyState::Occupied(path.to_path_buf()),
        Some(0) => OccupancyState::Free,
        Some(1) if found => OccupancyState::Occupied(path.to_path_buf()),
        Some(1) if denied => OccupancyState::Unknown(format!(
            "permission denied querying open files under {}",
            path.display()
        )),
        Some(1) => OccupancyState::Free,
        other => {
            OccupancyState::Unknown(format!("lsof exited with {other:?} for {}", path.display()))
        }
    }
}

/// Structured current-use evidence for whether some process holds this
/// unit open right now.
///
/// A unit *is* the path and everything under it -- that is what an action
/// renames away -- so this asks about the whole unit, through the same
/// bounded [`probe_path`] the sinks use (`lsof +D` for a directory). The
/// PR #123 review's counterexample: `lsof -- <dir>` reports only handles
/// on the directory inode, and the old implementation turned that into a
/// published `Known(false)` fact reading "no open-file match this pass"
/// for a unit whose contents were demonstrably open. A negative answer
/// to a question that was never asked is worse than no answer.
pub fn open_file_evidence(path: &Path) -> Evidence {
    let observed_at = crate::entities::now();
    let source = EvidenceSource::ProcessQuery {
        tool: "lsof".into(),
    };
    match probe_path(path) {
        OccupancyState::Occupied(open_at) => Evidence::known(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            FactValue::Bool(true),
            source,
            observed_at,
        )
        .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS))
        .with_note(format!("open handle found on {}", open_at.display())),
        OccupancyState::Free => Evidence::known(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            FactValue::Bool(false),
            source,
            observed_at,
        )
        .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS))
        .with_note(
            "no open-file match for this path or anything under it this pass; not proof that no \
             process or consumer needs it",
        ),
        OccupancyState::Unknown(reason) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            source,
            observed_at,
            crate::evidence::Reason::carried(reason),
        ),
    }
}

/// Current-use evidence from a Docker object's already-collected
/// container references (`docker::ContainerRef`, `docker::DockerFacts`):
/// a `running` container is a live consumer; anything else (`exited`,
/// none at all) is not proof of no consumer, just no *running* one seen
/// this pass.
pub fn docker_running_container_evidence(containers: &[crate::docker::ContainerRef]) -> Evidence {
    let observed_at = crate::entities::now();
    if containers.is_empty() {
        return Evidence::unknown(
            FactKind::CurrentUse,
            FactSubtype::RunningContainer,
            EvidenceSource::DockerApi {
                detail: "no container references collected for this object".into(),
            },
            observed_at,
            crate::reason!("no container reference recorded this pass"),
        );
    }
    let running: Vec<String> = containers
        .iter()
        .filter(|c| c.state == "running")
        .map(|c| c.name.clone())
        .collect();
    let has_running = !running.is_empty();
    Evidence::known(
        FactKind::CurrentUse,
        FactSubtype::RunningContainer,
        FactValue::Bool(has_running),
        EvidenceSource::DockerApi {
            detail: format!("{} container reference(s) inspected", containers.len()),
        },
        observed_at,
    )
    // Coverage, not just expiry: the daemon reports the containers *it*
    // knows about. A consumer that is not a container (a `docker build`
    // in flight, another daemon, a `docker cp` reading a volume) is
    // outside what this question can see, so the fact carries that limit
    // alongside its recheck window rather than reading as "nothing at
    // all uses this".
    .with_freshness(Freshness::expires_after_with_coverage(
        CURRENT_USE_EXPIRY_SECS,
        "only container references this daemon reported this pass; a non-container consumer \
         (an in-flight build, another daemon, a running `docker cp`) would not appear here",
    ))
    .with_note(if has_running {
        format!("running: {}", running.join(", "))
    } else {
        "no container currently running against this object".into()
    })
}

/// Advisory-lock probe: attempts (and immediately releases) a
/// non-blocking exclusive `flock` on `lock_path`. `Ok(true)` means the
/// lock was free (this call briefly held and released it); `Ok(false)`
/// means another process currently holds it. Never writes to the file;
/// never blocks.
fn probe_flock(lock_path: &Path) -> std::io::Result<bool> {
    crate::fs_gate::sys::flock_is_free(lock_path)
}

/// Manager lock-file evidence (Cargo's `.package-cache`, a Gradle daemon
/// registry lock, ...): read-only, bounded, never held past this check.
/// Absence of the lock file is `Unknown` (most managers only create the
/// file while doing something), not "not in use".
pub fn manager_lock_evidence(tool: impl Into<String>, lock_path: &Path) -> Evidence {
    let observed_at = crate::entities::now();
    let tool = tool.into();
    let source = EvidenceSource::ManagerLock {
        tool: tool.clone(),
        path: lock_path.display().to_string(),
    };
    if !crate::fs_gate::exists(lock_path) {
        return Evidence::unknown(
            FactKind::CurrentUse,
            FactSubtype::Lock,
            source,
            observed_at,
            crate::reason!("no lock file present"),
        );
    }
    match probe_flock(lock_path) {
        Ok(free) => Evidence::known(
            FactKind::CurrentUse,
            FactSubtype::Lock,
            FactValue::Bool(!free),
            source,
            observed_at,
        )
        .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS)),
        Err(e) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::Lock,
            source,
            observed_at,
            crate::reason!("could not probe lock: {e}"),
        ),
    }
}

/// Simulator "booted" state (#55) via the bounded, allow-listed,
/// read-only `xcrun simctl list devices -j` query
/// (`locations::ALLOWED_COMMANDS`). `udid` identifies the target
/// simulator; a query failure is `Unavailable`, an unmatched udid is
/// `Unknown` (never assumed "not booted").
pub fn simulator_booted_evidence(udid: &str, env: &crate::locations::Environment) -> Evidence {
    let observed_at = crate::entities::now();
    let source = EvidenceSource::ProcessQuery {
        tool: "xcrun simctl list devices -j".into(),
    };
    match env.run_command("xcrun", &["simctl", "list", "devices", "-j"]) {
        Ok(outcome) if outcome.success => match find_simulator_state(&outcome.stdout, udid) {
            Some(state) => Evidence::known(
                FactKind::CurrentUse,
                FactSubtype::Booted,
                FactValue::Bool(state.eq_ignore_ascii_case("booted")),
                source,
                observed_at,
            )
            .with_note(format!("simctl state: {state}"))
            // `simctl list devices` answers for the devices *this user's*
            // CoreSimulator store holds. A device in another user's store,
            // or one backed by a system-wide runtime volume this user
            // cannot read, is outside the query's reach -- a coverage
            // limit, separate from the recheck window.
            .with_freshness(Freshness::expires_after_with_coverage(
                CURRENT_USE_EXPIRY_SECS,
                "only devices `simctl list devices` reports for this user; a device in another \
                 user's CoreSimulator store or on an unreadable runtime volume would not appear",
            )),
            None => Evidence::unknown(
                FactKind::CurrentUse,
                FactSubtype::Booted,
                source,
                observed_at,
                crate::reason!("udid not found in simctl device list"),
            ),
        },
        Ok(_) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::Booted,
            source,
            observed_at,
            crate::reason!("simctl query did not succeed"),
        ),
        Err(reason) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::Booted,
            source,
            observed_at,
            crate::evidence::Reason::carried(reason),
        ),
    }
}

/// Pure JSON scan for `udid`'s `"state"` field in `simctl list devices
/// -j`'s output shape (`{"devices": {"<runtime>": [{"udid":..,
/// "state":..}, ...]}}`), factored out for testability without a real
/// simulator.
fn find_simulator_state(json_text: &str, udid: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json_text).ok()?;
    let devices = value.get("devices")?.as_object()?;
    for runtime_list in devices.values() {
        if let Some(list) = runtime_list.as_array() {
            for dev in list {
                if dev.get("udid").and_then(|v| v.as_str()) == Some(udid) {
                    return dev
                        .get("state")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::FactStatus;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn probe(code: Option<i32>, stdout: &str, stderr: &str) -> OccupancyState {
        classify_lsof_exit(code, stdout, stderr, Path::new("/x"))
    }

    #[test]
    fn lsof_positive_match_is_occupied() {
        assert!(matches!(
            probe(Some(0), "COMMAND PID\nswamp 1 /x", ""),
            OccupancyState::Occupied(_)
        ));
    }

    #[test]
    fn lsof_no_match_is_free() {
        assert_eq!(probe(Some(1), "", ""), OccupancyState::Free);
    }

    #[test]
    fn lsof_permission_denied_is_unknown_not_free() {
        // The tempting shortcut this rejects: treating a query that
        // could not run the same as "checked, found nothing". A sink
        // refuses on Unknown, so this distinction is the difference
        // between refusing and deleting.
        let state = probe(Some(1), "", "lsof: WARNING: Permission denied");
        assert!(matches!(state, OccupancyState::Unknown(_)), "{state:?}");
        assert!(!matches!(state, OccupancyState::Free));
        assert!(state.refusal().is_some());
    }

    #[test]
    fn an_unexpected_lsof_exit_code_is_unknown() {
        assert!(matches!(probe(Some(9), "", ""), OccupancyState::Unknown(_)));
        assert!(matches!(probe(None, "", ""), OccupancyState::Unknown(_)));
    }

    #[test]
    fn a_free_probe_is_the_only_state_that_permits_an_action() {
        assert!(matches!(OccupancyState::Free, OccupancyState::Free));
        assert!(OccupancyState::Free.refusal().is_none());
        assert!(!matches!(
            OccupancyState::Occupied(PathBuf::from("/x")),
            OccupancyState::Free
        ));
        assert!(!matches!(
            OccupancyState::Unknown("nope".into()),
            OccupancyState::Free
        ));
    }

    #[test]
    fn docker_running_container_detected() {
        let containers = vec![crate::docker::ContainerRef {
            name: "web-1".into(),
            state: "running".into(),
            finished_at: None,
        }];
        let ev = docker_running_container_evidence(&containers);
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(true))
        ));
    }

    #[test]
    fn docker_exited_container_is_known_false_not_no_consumer() {
        let containers = vec![crate::docker::ContainerRef {
            name: "web-1".into(),
            state: "exited".into(),
            finished_at: Some("2026-01-01T00:00:00Z".into()),
        }];
        let ev = docker_running_container_evidence(&containers);
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(false))
        ));
    }

    #[test]
    fn docker_no_containers_recorded_is_unknown_not_known_false() {
        let ev = docker_running_container_evidence(&[]);
        assert!(matches!(ev.status, FactStatus::Unknown { .. }));
    }

    #[test]
    fn manager_lock_absent_file_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("does-not-exist.lock");
        let ev = manager_lock_evidence("cargo", &lock);
        assert!(matches!(ev.status, FactStatus::Unknown { .. }));
    }

    #[test]
    fn manager_lock_free_is_known_false() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("package-cache");
        std::fs::write(&lock, b"").unwrap();
        let ev = manager_lock_evidence("cargo", &lock);
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(false))
        ));
    }

    #[test]
    fn manager_lock_held_is_known_true() {
        use std::os::unix::io::AsRawFd;
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("daemon.lock");
        std::fs::write(&lock, b"").unwrap();
        let held = std::fs::File::open(&lock).unwrap();
        let rc = unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "test setup must acquire the lock first");
        let ev = manager_lock_evidence("gradle", &lock);
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(true))
        ));
        drop(held);
    }

    #[test]
    fn find_simulator_state_matches_udid() {
        let json = r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-17-0":[
            {"udid":"AAAA","state":"Booted","name":"iPhone 15"},
            {"udid":"BBBB","state":"Shutdown","name":"iPhone 14"}
        ]}}"#;
        assert_eq!(
            find_simulator_state(json, "AAAA").as_deref(),
            Some("Booted")
        );
        assert_eq!(
            find_simulator_state(json, "BBBB").as_deref(),
            Some("Shutdown")
        );
        assert_eq!(find_simulator_state(json, "ZZZZ"), None);
    }

    #[test]
    fn simulator_booted_evidence_positive_match() {
        let json = r#"{"devices":{"iOS-17":[{"udid":"AAAA","state":"Booted"}]}}"#;
        let runner = Arc::new(crate::locations::FakeCommandRunner::new().with_answer(
            "xcrun",
            &["simctl", "list", "devices", "-j"],
            json,
        ));
        let env = crate::locations::Environment::fixture(
            std::path::PathBuf::from("/home/dev"),
            std::collections::HashMap::new(),
            crate::locations::Platform::MacOS,
        )
        .with_runner(runner);
        let ev = simulator_booted_evidence("AAAA", &env);
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(true))
        ));
    }

    #[test]
    fn simulator_booted_evidence_unavailable_on_query_failure() {
        let runner = Arc::new(crate::locations::FakeCommandRunner::new().with_failure(
            "xcrun",
            &["simctl", "list", "devices", "-j"],
            "xcrun not found",
        ));
        let env = crate::locations::Environment::fixture(
            std::path::PathBuf::from("/home/dev"),
            std::collections::HashMap::new(),
            crate::locations::Platform::MacOS,
        )
        .with_runner(runner);
        let ev = simulator_booted_evidence("AAAA", &env);
        assert!(matches!(ev.status, FactStatus::Unavailable { .. }));
    }

    #[test]
    fn simulator_booted_evidence_unknown_udid_is_unknown_never_assumed_shutdown() {
        let json = r#"{"devices":{"iOS-17":[{"udid":"OTHER","state":"Booted"}]}}"#;
        let runner = Arc::new(crate::locations::FakeCommandRunner::new().with_answer(
            "xcrun",
            &["simctl", "list", "devices", "-j"],
            json,
        ));
        let env = crate::locations::Environment::fixture(
            std::path::PathBuf::from("/home/dev"),
            std::collections::HashMap::new(),
            crate::locations::Platform::MacOS,
        )
        .with_runner(runner);
        let ev = simulator_booted_evidence("AAAA", &env);
        assert!(matches!(ev.status, FactStatus::Unknown { .. }));
    }

    #[test]
    fn all_current_use_facts_expire_and_are_not_trusted_forever() {
        let ev = manager_lock_evidence("cargo", Path::new("/nowhere/does-not-exist"));
        let ev = Evidence::known(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            FactValue::Bool(true),
            EvidenceSource::ProcessQuery {
                tool: "lsof".into(),
            },
            ev.observed_at,
        )
        .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS));
        assert!(!ev.is_stale(ev.observed_at + CURRENT_USE_EXPIRY_SECS));
        assert!(ev.is_stale(ev.observed_at + CURRENT_USE_EXPIRY_SECS + 1));
    }
}
