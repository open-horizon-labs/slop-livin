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
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

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
    /// True only for [`OccupancyState::Free`]: the one state in which a
    /// destructive action may proceed.
    pub fn is_free(&self) -> bool {
        matches!(self, Self::Free)
    }

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
pub fn probe_path(path: &Path) -> OccupancyState {
    use std::io::{Read, Seek, SeekFrom};

    let is_dir = match std::fs::symlink_metadata(path) {
        Ok(m) => m.is_dir() && !m.file_type().is_symlink(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Nothing there to hold open. The caller's own identity
            // recheck is what refuses a vanished path.
            return OccupancyState::Free;
        }
        Err(e) => return OccupancyState::Unknown(format!("cannot stat {}: {e}", path.display())),
    };

    let mut cmd = Command::new("lsof");
    if is_dir {
        cmd.arg("+D").arg(path);
    } else {
        cmd.arg("--").arg(path);
    }
    let (Ok(out_file), Ok(err_file)) = (tempfile::tempfile(), tempfile::tempfile()) else {
        return OccupancyState::Unknown("could not create a buffer for the lsof probe".into());
    };
    let (Ok(stdout), Ok(stderr)) = (out_file.try_clone(), err_file.try_clone()) else {
        return OccupancyState::Unknown("could not redirect the lsof probe".into());
    };
    let Ok(mut child) = cmd.stdout(stdout).stderr(stderr).spawn() else {
        return OccupancyState::Unknown("lsof could not be started".into());
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < OCCUPANCY_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return OccupancyState::Unknown(format!(
                    "lsof did not answer within {}s for {}",
                    OCCUPANCY_TIMEOUT.as_secs(),
                    path.display()
                ));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return OccupancyState::Unknown(format!("lsof probe failed: {e}"));
            }
        }
    };
    let read_all = |mut f: std::fs::File| -> String {
        let mut s = String::new();
        let _ = f.seek(SeekFrom::Start(0));
        let _ = f.read_to_string(&mut s);
        s
    };
    let stdout_text = read_all(out_file);
    let stderr_text = read_all(err_file).to_lowercase();
    match status.code() {
        // `lsof` exits 0 when it found at least one open handle.
        Some(0) if !stdout_text.trim().is_empty() => OccupancyState::Occupied(path.to_path_buf()),
        Some(0) => OccupancyState::Free,
        // Exit 1 with no output is "nothing found"; exit 1 with a
        // permission complaint is "could not look".
        Some(1) if stdout_text.trim().is_empty() => {
            if stderr_text.contains("permission denied") {
                OccupancyState::Unknown(format!(
                    "permission denied querying open files under {}",
                    path.display()
                ))
            } else {
                OccupancyState::Free
            }
        }
        Some(1) => OccupancyState::Occupied(path.to_path_buf()),
        other => {
            OccupancyState::Unknown(format!("lsof exited with {other:?} for {}", path.display()))
        }
    }
}

/// Boolean convenience over [`probe_path`], fail-closed: anything but
/// [`OccupancyState::Free`] is `true`. **Never call this from a
/// destructive sink** -- it collapses `Unknown` into `Occupied` and so
/// cannot record *why* an action was refused; sinks use
/// `crate::recheck::member_occupancy` (audited by
/// `occupancy_is_tristate_at_sinks`).
pub fn occupied(path: &Path) -> bool {
    !probe_path(path).is_free()
}

/// Pure classification of an `lsof` invocation's outcome, factored out so
/// it is unit-testable without spawning a real process. `spawn_ok` is
/// `false` only when the process itself could not be started (missing
/// binary, permission to exec); a completed run with a non-zero exit
/// still reaches the `Some(status_success)` branch.
fn classify_lsof(spawn_ok: bool, status_success: bool, stdout: &[u8], stderr: &[u8]) -> Evidence {
    let observed_at = crate::entities::now();
    if !spawn_ok {
        return Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            EvidenceSource::ProcessQuery {
                tool: "lsof".into(),
            },
            observed_at,
            "lsof could not be started",
        );
    }
    let stderr_text = String::from_utf8_lossy(stderr).to_lowercase();
    if !status_success && stderr_text.contains("permission") {
        return Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            EvidenceSource::ProcessQuery {
                tool: "lsof".into(),
            },
            observed_at,
            "permission denied querying open files for this path",
        );
    }
    let has_match = status_success && !stdout.is_empty();
    let mut ev = Evidence::known(
        FactKind::CurrentUse,
        FactSubtype::OpenFile,
        FactValue::Bool(has_match),
        EvidenceSource::ProcessQuery {
            tool: "lsof".into(),
        },
        observed_at,
    )
    .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS));
    if !has_match {
        ev = ev.with_note(
            "no open-file match this pass; not proof that no process or consumer needs this path",
        );
    }
    ev
}

/// Structured current-use evidence for whether some process holds this
/// path open right now (the same signal [`occupied`] reduces to a bool).
pub fn open_file_evidence(path: &Path) -> Evidence {
    match Command::new("lsof").arg("--").arg(path).output() {
        Ok(o) => classify_lsof(true, o.status.success(), &o.stdout, &o.stderr),
        Err(_) => classify_lsof(false, false, &[], &[]),
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
            "no container reference recorded this pass",
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
    .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS))
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
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new().read(true).open(lock_path)?;
    let fd = file.as_raw_fd();
    let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        Ok(true)
    } else {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Ok(false)
        } else {
            Err(err)
        }
    }
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
    if !lock_path.exists() {
        return Evidence::unknown(
            FactKind::CurrentUse,
            FactSubtype::Lock,
            source,
            observed_at,
            "no lock file present",
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
            format!("could not probe lock: {e}"),
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
            .with_freshness(Freshness::expires_after(CURRENT_USE_EXPIRY_SECS)),
            None => Evidence::unknown(
                FactKind::CurrentUse,
                FactSubtype::Booted,
                source,
                observed_at,
                "udid not found in simctl device list",
            ),
        },
        Ok(_) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::Booted,
            source,
            observed_at,
            "simctl query did not succeed",
        ),
        Err(reason) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::Booted,
            source,
            observed_at,
            reason,
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
    use std::sync::Arc;

    #[test]
    fn lsof_positive_match_is_known_true() {
        let ev = classify_lsof(true, true, b"COMMAND PID\nswamp 1 /x", b"");
        assert!(ev.is_known());
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(true))
        ));
    }

    #[test]
    fn lsof_no_match_is_known_false_with_limits_note() {
        let ev = classify_lsof(true, false, b"", b"");
        assert!(matches!(
            ev.status,
            FactStatus::Known(FactValue::Bool(false))
        ));
        assert!(ev.note.as_deref().unwrap().contains("not proof"));
    }

    #[test]
    fn lsof_permission_denied_is_unavailable_not_known_false() {
        // The tempting shortcut this rejects: treating a failed query the
        // same as "checked, found nothing".
        let ev = classify_lsof(true, false, b"", b"lsof: WARNING: Permission denied");
        assert!(matches!(ev.status, FactStatus::Unavailable { .. }));
    }

    #[test]
    fn lsof_spawn_failure_is_unavailable() {
        let ev = classify_lsof(false, false, b"", b"");
        assert!(matches!(ev.status, FactStatus::Unavailable { .. }));
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
        let ev = classify_lsof(true, true, b"x", b"");
        assert!(!ev.is_stale(ev.observed_at + CURRENT_USE_EXPIRY_SECS));
        assert!(ev.is_stale(ev.observed_at + CURRENT_USE_EXPIRY_SECS + 1));
    }
}
