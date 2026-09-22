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
    probe_paths(&[path])
}

/// [`probe_path`] over several anchors at once. On Linux that is one
/// procfs pass for all of them (a process table scan per anchor would
/// multiply); on macOS it is one bounded `lsof` per anchor, as it
/// always was. The first non-`Free` answer wins.
pub fn probe_paths(paths: &[&Path]) -> OccupancyState {
    match crate::platform::OccupancyProbe::for_os(crate::platform::Os::current()) {
        crate::platform::OccupancyProbe::Lsof => {
            for p in paths {
                match lsof_probe(p) {
                    OccupancyState::Free => {}
                    other => return other,
                }
            }
            OccupancyState::Free
        }
        crate::platform::OccupancyProbe::Procfs => {
            // SAFETY: getpid cannot fail and reads no memory.
            let pid = unsafe { libc::getpid() as u32 };
            let procfs = procfs_probe(
                Path::new("/proc"),
                paths,
                &Creds::of_self(),
                pid,
                OCCUPANCY_TIMEOUT,
            );
            // procfs is the answer; `lsof`, where one is installed, is a
            // second reader of the same kernel state and can only make
            // the answer stricter: Occupied or Unknown from either wins,
            // Free needs both. With no `lsof` on PATH, procfs stands on
            // its own -- a minimal install has none, and needs none.
            if procfs != OccupancyState::Free || !lsof_on_path() {
                return procfs;
            }
            for p in paths {
                match lsof_probe(p) {
                    OccupancyState::Free => {}
                    other => return other,
                }
            }
            OccupancyState::Free
        }
    }
}

/// Whether an `lsof` is on `PATH` at all -- present, not necessarily
/// runnable. A present `lsof` that cannot be run is a second reader
/// that could not answer, which is `Unknown`, exactly as on macOS.
///
/// A `PATH` entry that cannot be examined counts as "maybe": the second
/// reader is then tried, and if it cannot run the answer is `Unknown`.
fn lsof_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        match std::fs::symlink_metadata(dir.join("lsof")) {
            Ok(_) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true,
        }
    }
    false
}

/// The name a current-use fact gives for where its answer came from.
fn probe_tool() -> &'static str {
    match crate::platform::OccupancyProbe::for_os(crate::platform::Os::current()) {
        crate::platform::OccupancyProbe::Lsof => "lsof",
        crate::platform::OccupancyProbe::Procfs => "procfs",
    }
}

fn lsof_probe(path: &Path) -> OccupancyState {
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

    crate::work_counters::record_spawn();
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
    // A capture that cannot be read back is a probe that did not
    // answer. Reading it as empty would turn "could not look" into
    // "found nothing" -- a fail-open `occupancy_gaps_are_unknown_never_free`
    // found in this function when it was written.
    let read_all = |mut f: std::fs::File| -> std::io::Result<String> {
        let mut s = String::new();
        f.seek(SeekFrom::Start(0))?;
        f.read_to_string(&mut s)?;
        Ok(s)
    };
    let stdout = match read_all(out_file) {
        Ok(s) => s,
        Err(e) => return OccupancyState::Unknown(format!("could not read lsof's output: {e}")),
    };
    let stderr = match read_all(err_file) {
        Ok(s) => s,
        Err(e) => return OccupancyState::Unknown(format!("could not read lsof's errors: {e}")),
    };
    classify_lsof_exit(status.code(), &stdout, &stderr, path)
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

// ---------------------------------------------------------------------
// Linux: procfs (#86)
// ---------------------------------------------------------------------

/// Linux occupancy from procfs, unprivileged: for every process running
/// as this user, its `cwd`, `root` and `exe` links, every open file
/// descriptor, and every file it has mapped (`maps`) are compared with
/// the anchors. Anything at or under an anchor is `Occupied`.
///
/// **What it can see, and what it answers when it cannot.** An
/// unprivileged process can read the fd table of the processes that run
/// as its own user and no others; that is the same boundary `lsof`
/// has without root, on either platform. So the question answered is
/// "does any process running as you hold this" -- the evidence says so
/// in its coverage note -- and every way *that* question can go
/// unanswered is `Unknown`, never `Free`:
///
/// * a process with exactly this user's credentials whose fd table,
///   links or maps cannot be read although its procfs entries are still
///   this user's (an LSM denial; anything the kernel's own ownership
///   rules do not explain);
/// * a procfs that belongs to another PID namespace (`/proc/self` is
///   not this process), where the processes listed are not the ones
///   that share this filesystem view;
/// * a process table or `/proc` that cannot be listed at all;
/// * the scan running past its time bound.
///
/// A process that exits mid-scan (`ENOENT`/`ESRCH` on its entries) is
/// skipped: it holds nothing any more. Processes the kernel does not let
/// this user read -- another user's, one with more privilege (a
/// capability this process lacks, a saved uid of root), or one marked
/// non-dumpable (root-owned procfs entries; measured on the Ubuntu 24.04
/// runner: `systemd --user` holds `CAP_WAKE_ALARM`, its `(sd-pam)` is
/// non-dumpable) -- are outside the question, as they are for `lsof`
/// run without root; the evidence's coverage note says so. `hidepid`
/// hides only other users' processes, so it narrows nothing the question
/// depends on.
///
/// Pure over `proc_root` so the fail-closed rules are testable against
/// a fixture tree on either platform; the real call passes `/proc`.
pub fn procfs_probe(
    proc_root: &Path,
    anchors: &[&Path],
    me: &Creds,
    self_pid: u32,
    budget: Duration,
) -> OccupancyState {
    use std::io::ErrorKind;
    let started = Instant::now();

    let mut targets: Vec<std::path::PathBuf> = Vec::new();
    for a in anchors {
        match std::fs::canonicalize(a) {
            Ok(c) => targets.push(c),
            // Nothing there to hold open; the caller's identity recheck
            // is what refuses a vanished path.
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return OccupancyState::Unknown(format!("cannot resolve {}: {e}", a.display()));
            }
        }
    }
    if targets.is_empty() {
        return OccupancyState::Free;
    }

    // The procfs must be this PID namespace's.
    match std::fs::read_link(proc_root.join("self")) {
        Ok(link) if link.to_str() == Some(self_pid.to_string().as_str()) => {}
        Ok(link) => {
            return OccupancyState::Unknown(format!(
                "{}/self is {} rather than this process ({self_pid}): the process table belongs \
                 to another PID namespace, so the processes sharing this filesystem are not the \
                 ones listed",
                proc_root.display(),
                link.display()
            ));
        }
        Err(e) => {
            return OccupancyState::Unknown(format!(
                "cannot read {}/self ({e}); procfs is not usable here",
                proc_root.display()
            ));
        }
    }

    let entries = match std::fs::read_dir(proc_root) {
        Ok(rd) => rd,
        Err(e) => {
            return OccupancyState::Unknown(format!("cannot list {} ({e})", proc_root.display()));
        }
    };
    let held = |p: &Path| -> Option<std::path::PathBuf> {
        let text = p.to_string_lossy();
        let text = text.strip_suffix(" (deleted)").unwrap_or(&text);
        let p = Path::new(text);
        targets
            .iter()
            .find(|t| p.starts_with(t))
            .map(|_| p.to_path_buf())
    };

    for entry in entries {
        if started.elapsed() > budget {
            return OccupancyState::Unknown(format!(
                "the procfs scan did not finish within {}s",
                budget.as_secs()
            ));
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                return OccupancyState::Unknown(format!(
                    "listing {} failed part-way ({e})",
                    proc_root.display()
                ));
            }
        };
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let dir = entry.path();
        let creds = match process_creds(&dir, me.uids[0]) {
            Ok(Some(c)) => c,
            // Exited between the listing and now.
            Ok(None) => continue,
            Err(why) => return OccupancyState::Unknown(format!("process {pid}: {why}")),
        };
        if !creds.same_privilege_as(me) {
            // Another user's process, or one running with more privilege
            // than this user (a setuid program, a process holding
            // capabilities): the kernel does not let this user read it,
            // and it is outside the question (see the doc comment).
            continue;
        }
        match process_holds(&dir, &held) {
            Ok(Some(member)) => return OccupancyState::Occupied(member),
            Ok(None) => {}
            Err(why) => match kernel_withholds(&dir, me.uids[0]) {
                // The kernel marks the process non-dumpable (its procfs
                // entries turn root-owned): no unprivileged tool may read
                // it, the same boundary as a more privileged process.
                Ok(true) => continue,
                Ok(false) => {
                    return OccupancyState::Unknown(format!(
                        "process {pid} ({}) runs as this user but {why}, so whether it holds \
                         anything under the selection is not known",
                        process_name(&dir)
                    ));
                }
                Err(e) if gone(&e) => continue,
                Err(e) => {
                    return OccupancyState::Unknown(format!(
                        "process {pid}: {why}, and its procfs entry cannot be examined ({e})"
                    ));
                }
            },
        }
    }
    OccupancyState::Free
}

/// Whether the kernel itself withholds a same-credential process from
/// this user: a process whose memory is marked non-dumpable (one that
/// changed credentials without an `exec`, like a PAM session holder, or
/// that asked for it with `prctl(PR_SET_DUMPABLE, 0)`) has its procfs
/// entries owned by root, and no unprivileged reader, `lsof` included,
/// can see its open files. That is the same boundary as a process with
/// more privilege than this user, and is treated the same way: outside
/// the question this probe answers, stated in the evidence's coverage.
///
/// A same-user process whose entries are still *this user's* and yet
/// cannot be read (an LSM denial, anything unexplained) is not that
/// boundary, and stays `Unknown`.
fn kernel_withholds(dir: &Path, my_uid: u32) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(dir.join("fd"))?;
    Ok(withheld_owner(m.uid(), my_uid))
}

/// The ownership rule `kernel_withholds` reads, pure so both platforms
/// test it.
fn withheld_owner(fd_dir_owner: u32, my_uid: u32) -> bool {
    fd_dir_owner != my_uid
}

/// A process's credentials as procfs states them: real, effective,
/// saved and filesystem uid and gid, and its permitted capability set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Creds {
    pub uids: [u32; 4],
    pub gids: [u32; 4],
    pub cap_prm: u64,
}

impl Creds {
    /// This process's own credentials.
    pub fn of_self() -> Creds {
        let text = std::fs::read_to_string("/proc/self/status");
        match text.as_deref().map(parse_status) {
            Ok(Some(c)) => c,
            // Without procfs there is nothing to compare against; the
            // probe itself then fails on `/proc/self` and says Unknown.
            _ => {
                // SAFETY: plain getters.
                let (u, g) = unsafe { (libc::getuid(), libc::getgid()) };
                Creds {
                    uids: [u; 4],
                    gids: [g; 4],
                    cap_prm: 0,
                }
            }
        }
    }

    /// The kernel's own test for "this user may read that process"
    /// (`ptrace_may_access` with `PTRACE_MODE_READ`): every uid and gid
    /// equal, and no capability this process lacks. A process that fails
    /// it is another user's, or runs with more privilege than this user
    /// has -- outside what an unprivileged probe can see, on either
    /// platform, and so outside the question it answers. A process that
    /// passes it and still cannot be read is the gap that is `Unknown`.
    pub fn same_privilege_as(&self, me: &Creds) -> bool {
        self.uids.iter().all(|u| *u == me.uids[0])
            && self.gids.iter().all(|g| *g == me.gids[0])
            && self.cap_prm & !me.cap_prm == 0
    }
}

/// `status`'s `Uid:`, `Gid:` and `CapPrm:` lines.
fn parse_status(text: &str) -> Option<Creds> {
    let four = |key: &str| -> Option<[u32; 4]> {
        let line = text.lines().find_map(|l| l.strip_prefix(key))?;
        let v: Vec<u32> = line
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
    };
    let cap_prm = text
        .lines()
        .find_map(|l| l.strip_prefix("CapPrm:"))
        .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
        .unwrap_or(0);
    Some(Creds {
        uids: four("Uid:")?,
        gids: four("Gid:")?,
        cap_prm,
    })
}

/// One process's credentials, from `status`. `status` stays readable
/// for a non-dumpable process, whose other entries become root-owned.
/// `Ok(None)` when the process has exited. Under `hidepid=1` another
/// user's `status` is unreadable; its directory's owner still says it is
/// not ours.
fn process_creds(dir: &Path, my_uid: u32) -> Result<Option<Creds>, String> {
    use std::io::ErrorKind;
    match std::fs::read_to_string(dir.join("status")) {
        Ok(text) => match parse_status(&text) {
            Some(c) => Ok(Some(c)),
            None => Err("its status has no readable Uid/Gid lines".into()),
        },
        Err(e) if gone(&e) => Ok(None),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            use std::os::unix::fs::MetadataExt;
            match std::fs::symlink_metadata(dir) {
                Ok(m) if m.uid() != my_uid => Ok(Some(Creds {
                    uids: [m.uid(); 4],
                    gids: [m.gid(); 4],
                    cap_prm: 0,
                })),
                Ok(_) => Err(format!("its status cannot be read ({e})")),
                Err(e) if gone(&e) => Ok(None),
                Err(e) => Err(format!("cannot stat it ({e})")),
            }
        }
        Err(e) => Err(format!("cannot read its status ({e})")),
    }
}

fn gone(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ESRCH)
}

fn process_name(dir: &Path) -> String {
    match std::fs::read_to_string(dir.join("comm")) {
        Ok(c) => c.trim().to_string(),
        Err(e) => format!("name unreadable: {e}"),
    }
}

/// Whether one process holds anything under an anchor: its `cwd`,
/// `root` and `exe` links, each fd, each mapped file. `Ok(None)` also
/// when the process exited while it was being read.
fn process_holds(
    dir: &Path,
    held: &dyn Fn(&Path) -> Option<std::path::PathBuf>,
) -> Result<Option<std::path::PathBuf>, String> {
    for link in ["cwd", "root", "exe"] {
        match std::fs::read_link(dir.join(link)) {
            Ok(t) => {
                if let Some(m) = held(&t) {
                    return Ok(Some(m));
                }
            }
            Err(e) if gone(&e) => return Ok(None),
            Err(e) => return Err(format!("its {link} link cannot be read ({e})")),
        }
    }
    let fds = match std::fs::read_dir(dir.join("fd")) {
        Ok(rd) => rd,
        Err(e) if gone(&e) => return Ok(None),
        Err(e) => return Err(format!("its open files cannot be listed ({e})")),
    };
    for fd in fds {
        let fd = match fd {
            Ok(f) => f,
            Err(e) if gone(&e) => return Ok(None),
            Err(e) => return Err(format!("its open files cannot be listed ({e})")),
        };
        match std::fs::read_link(fd.path()) {
            Ok(t) => {
                if let Some(m) = held(&t) {
                    return Ok(Some(m));
                }
            }
            // That descriptor closed since the listing.
            Err(e) if gone(&e) => {}
            Err(e) => {
                return Err(format!(
                    "its descriptor {:?} cannot be read ({e})",
                    fd.file_name()
                ));
            }
        }
    }
    match std::fs::read_to_string(dir.join("maps")) {
        Ok(maps) => {
            for line in maps.lines() {
                // address perms offset dev inode pathname
                let Some(path) = line.split_whitespace().nth(5) else {
                    continue;
                };
                if path.starts_with('/')
                    && let Some(m) = held(Path::new(path))
                {
                    return Ok(Some(m));
                }
            }
            Ok(None)
        }
        Err(e) if gone(&e) => Ok(None),
        Err(e) => Err(format!("its memory maps cannot be read ({e})")),
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
        tool: probe_tool().into(),
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
        .with_freshness(Freshness::expires_after_with_coverage(
            CURRENT_USE_EXPIRY_SECS,
            "only processes this user can inspect: those running with this user's credentials \
             and no more. Another user's process, a more privileged one (a root daemon, a \
             container runtime, a setuid program) and one the kernel marks non-dumpable (a PAM \
             session holder, an agent that hides its memory) are not visible without privileges",
        ))
        .with_note(
            "no open-file match for this path or anything under it this pass; not proof that no \
             process or consumer needs it",
        ),
        OccupancyState::Unknown(reason) => Evidence::unavailable(
            FactKind::CurrentUse,
            FactSubtype::OpenFile,
            source,
            observed_at,
            reason,
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
    use std::os::unix::fs::PermissionsExt;
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
        assert!(!state.is_free());
        assert!(state.refusal().is_some());
    }

    #[test]
    fn an_unexpected_lsof_exit_code_is_unknown() {
        assert!(matches!(probe(Some(9), "", ""), OccupancyState::Unknown(_)));
        assert!(matches!(probe(None, "", ""), OccupancyState::Unknown(_)));
    }

    #[test]
    fn a_free_probe_is_the_only_state_that_permits_an_action() {
        assert!(OccupancyState::Free.is_free());
        assert!(OccupancyState::Free.refusal().is_none());
        assert!(!OccupancyState::Occupied(PathBuf::from("/x")).is_free());
        assert!(!OccupancyState::Unknown("nope".into()).is_free());
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

    // ---- procfs (#86): the fail-closed rules against fixture trees ----

    struct FakeProc {
        _tmp: tempfile::TempDir,
        root: std::path::PathBuf,
        work: std::path::PathBuf,
    }

    /// "This user" in the fixtures is the real one: whether the kernel
    /// withholds a process is read from who owns its fixture entries,
    /// and the test's files are owned by whoever runs it.
    fn uid() -> u32 {
        unsafe { libc::getuid() }
    }
    const MY_PID: u32 = 4242;

    fn me() -> Creds {
        Creds {
            uids: [uid(); 4],
            gids: [uid(); 4],
            cap_prm: 0,
        }
    }

    fn fake_proc() -> FakeProc {
        let tmp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        let root = base.join("proc");
        let work = base.join("work");
        std::fs::create_dir_all(work.join("target/debug")).unwrap();
        std::fs::write(work.join("target/debug/app"), b"x").unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(MY_PID.to_string(), root.join("self")).unwrap();
        FakeProc {
            _tmp: tmp,
            root,
            work,
        }
    }

    fn process(
        fp: &FakeProc,
        pid: u32,
        owner: u32,
        cwd: &Path,
        fds: &[&Path],
    ) -> std::path::PathBuf {
        let (uid, me) = (owner, uid());
        let d = fp.root.join(pid.to_string());
        std::fs::create_dir_all(d.join("fd")).unwrap();
        std::fs::write(
            d.join("status"),
            format!(
                "Name:\tx\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nGid:\t{me}\t{me}\t{me}\t{me}\nCapPrm:\t0000000000000000\n"
            ),
        )
        .unwrap();
        std::fs::write(d.join("comm"), "fixture\n").unwrap();
        std::os::unix::fs::symlink(cwd, d.join("cwd")).unwrap();
        std::os::unix::fs::symlink("/", d.join("root")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/fixture", d.join("exe")).unwrap();
        for (i, f) in fds.iter().enumerate() {
            std::os::unix::fs::symlink(f, d.join("fd").join(i.to_string())).unwrap();
        }
        std::fs::write(d.join("maps"), "").unwrap();
        d
    }

    fn proc_probe(fp: &FakeProc) -> OccupancyState {
        let target = fp.work.join("target");
        procfs_probe(&fp.root, &[&target], &me(), MY_PID, Duration::from_secs(10))
    }

    #[test]
    fn procfs_a_process_holding_a_descendant_file_or_cwd_is_occupied() {
        let fp = fake_proc();
        process(&fp, 10, uid(), Path::new("/"), &[Path::new("/dev/null")]);
        assert_eq!(proc_probe(&fp), OccupancyState::Free);

        let app = fp.work.join("target/debug/app");
        process(&fp, 11, uid(), Path::new("/"), &[&app]);
        assert_eq!(proc_probe(&fp), OccupancyState::Occupied(app));

        let fp = fake_proc();
        let cwd = fp.work.join("target/debug");
        process(&fp, 12, uid(), &cwd, &[]);
        assert_eq!(proc_probe(&fp), OccupancyState::Occupied(cwd));

        let fp = fake_proc();
        let d = process(&fp, 13, uid(), Path::new("/"), &[]);
        let app = fp.work.join("target/debug/app");
        std::fs::write(
            d.join("maps"),
            format!("7f00-7f01 r-xp 00000000 08:01 1234  {}\n", app.display()),
        )
        .unwrap();
        assert_eq!(
            proc_probe(&fp),
            OccupancyState::Occupied(app),
            "a mapped file counts"
        );
    }

    /// A deleted-but-open file under the anchor still holds its space.
    #[test]
    fn procfs_a_deleted_open_file_under_the_anchor_still_counts() {
        let fp = fake_proc();
        let gone = format!("{}/target/debug/old.o (deleted)", fp.work.display());
        process(&fp, 20, uid(), Path::new("/"), &[Path::new(&gone)]);
        assert!(matches!(proc_probe(&fp), OccupancyState::Occupied(_)));
    }

    /// Another user's process is outside the question; one of *ours*
    /// whose fd table cannot be read is `Unknown`, never `Free`.
    #[test]
    fn procfs_an_unreadable_process_of_this_user_is_unknown_not_free() {
        if unsafe { libc::getuid() } == 0 {
            eprintln!("SKIP procfs_an_unreadable_process: root ignores the mode bits");
            return;
        }
        let fp = fake_proc();
        let d = process(&fp, 30, uid() + 1, Path::new("/"), &[]);
        std::fs::set_permissions(d.join("fd"), std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(
            proc_probe(&fp),
            OccupancyState::Free,
            "another user's process is not read"
        );
        std::fs::set_permissions(d.join("fd"), std::fs::Permissions::from_mode(0o700)).unwrap();

        let fp = fake_proc();
        let d = process(&fp, 31, uid(), Path::new("/"), &[]);
        std::fs::set_permissions(d.join("fd"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let got = proc_probe(&fp);
        std::fs::set_permissions(d.join("fd"), std::fs::Permissions::from_mode(0o700)).unwrap();
        match got {
            OccupancyState::Unknown(why) => {
                assert!(why.contains("31") && why.contains("open files"), "{why}")
            }
            other => panic!("a same-user process we cannot read must be Unknown: {other:?}"),
        }
    }

    /// The kernel's own read rule decides who is in the question: a
    /// process running with more privilege than this user -- a saved uid
    /// of root (a setuid program), a permitted capability -- is not
    /// readable by this user and is outside the question, like another
    /// user's; one with exactly this user's credentials that still
    /// cannot be read is `Unknown`.
    #[test]
    fn procfs_privilege_not_the_uid_alone_decides_who_is_in_scope() {
        if unsafe { libc::getuid() } == 0 {
            eprintln!("SKIP procfs_privilege: root ignores the mode bits");
            return;
        }
        for status in [
            format!(
                "Uid:\t{u}\t{u}\t0\t{u}\nGid:\t{u}\t{u}\t{u}\t{u}\nCapPrm:\t0\n",
                u = uid()
            ),
            format!(
                "Uid:\t{u}\t{u}\t{u}\t{u}\nGid:\t{u}\t{u}\t{u}\t{u}\nCapPrm:\t0000000000200000\n",
                u = uid()
            ),
        ] {
            let fp = fake_proc();
            let d = process(&fp, 60, uid(), Path::new("/"), &[]);
            std::fs::write(d.join("status"), &status).unwrap();
            std::fs::set_permissions(d.join("fd"), std::fs::Permissions::from_mode(0o000)).unwrap();
            let got = proc_probe(&fp);
            std::fs::set_permissions(d.join("fd"), std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(
                got,
                OccupancyState::Free,
                "a more-privileged process is not read: {status}"
            );
        }
        assert!(
            parse_status("Uid:\t1\t2\t3\t4\nGid:\t5\t6\t7\t8\nCapPrm:\t00000000000000ff\n")
                .is_some_and(|c| c.uids == [1, 2, 3, 4]
                    && c.gids == [5, 6, 7, 8]
                    && c.cap_prm == 0xff)
        );
    }

    /// A same-credential process whose procfs entries turned root-owned
    /// (non-dumpable) is withheld by the kernel and outside the question;
    /// one whose entries are still ours and unreadable is not.
    #[test]
    fn procfs_the_kernels_non_dumpable_boundary_is_read_from_ownership() {
        assert!(
            withheld_owner(0, 1000),
            "root-owned entries: the kernel withholds it"
        );
        assert!(
            !withheld_owner(1000, 1000),
            "our own entries, unreadable: Unknown, not withheld"
        );
    }

    #[test]
    fn procfs_a_foreign_pid_namespace_or_missing_proc_is_unknown() {
        let fp = fake_proc();
        std::fs::remove_file(fp.root.join("self")).unwrap();
        std::os::unix::fs::symlink("1", fp.root.join("self")).unwrap();
        match proc_probe(&fp) {
            OccupancyState::Unknown(why) => assert!(why.contains("namespace"), "{why}"),
            other => panic!("{other:?}"),
        }
        let fp = fake_proc();
        let target = fp.work.join("target");
        let got = procfs_probe(
            &fp.root.join("missing"),
            &[&target],
            &me(),
            MY_PID,
            Duration::from_secs(10),
        );
        assert!(matches!(got, OccupancyState::Unknown(_)), "{got:?}");
    }

    /// A process that exits mid-scan holds nothing; its vanished entries
    /// are not an error and not a reason to refuse.
    #[test]
    fn procfs_a_process_that_exited_mid_scan_is_skipped() {
        let fp = fake_proc();
        let d = fp.root.join("40");
        std::fs::create_dir_all(&d).unwrap();
        // status present, everything else gone: exited after listing.
        std::fs::write(
            d.join("status"),
            format!(
                "Uid:\t{u}\t{u}\t{u}\t{u}\nGid:\t{u}\t{u}\t{u}\t{u}\n",
                u = uid()
            ),
        )
        .unwrap();
        assert_eq!(proc_probe(&fp), OccupancyState::Free);
    }

    #[test]
    fn procfs_past_its_time_bound_is_unknown() {
        let fp = fake_proc();
        process(&fp, 50, uid(), Path::new("/"), &[]);
        let target = fp.work.join("target");
        let got = procfs_probe(&fp.root, &[&target], &me(), MY_PID, Duration::ZERO);
        assert!(
            matches!(got, OccupancyState::Unknown(ref w) if w.contains("did not finish")),
            "{got:?}"
        );
    }
}
