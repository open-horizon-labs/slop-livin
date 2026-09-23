//! The one way swamp starts a subprocess.
//!
//! * **The program is a [`Program`] variant.** There is no `Command`
//!   outside this module (the `std::process` path is rejected everywhere
//!   else by the gate audit and by clippy), so "which binaries can swamp
//!   run" is this enum, and a second scheduler (`crontab`) or an
//!   uncounted probe does not compile.
//! * **Every run is counted** by `work_counters` before the process
//!   starts, so "this observation ran no subprocess" is measured by
//!   swamp's own instrumentation (re-review 3, F2).
//! * **Every run is bounded** by a timeout and returns only a
//!   [`RunOutput`]; no `Child` or `Command` escapes.
//! * **Mutating verbs are refused here.** `docker … rm`, `git worktree
//!   prune` and friends are reachable only through
//!   [`super::destroy`], which takes an
//!   [`crate::authority::Authorized`].

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Seek, SeekFrom};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Every program swamp may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Program {
    /// Occupancy probe (`lsof -- <path>`, `lsof +D <dir>`).
    Lsof,
    /// Xcode `Info.plist` → XML (`plutil -convert xml1 -o - <plist>`),
    /// read-only, output to stdout.
    Plutil,
    /// `xcrun simctl list devices -j`.
    Xcrun,
    /// `du -skPx <root>` (the `--verify-du` cross-check).
    Du,
    /// Docker daemon queries (`docker system df`, `inspect`, `ps`).
    Docker,
    /// GitHub enrichment (`gh api …`, `gh pr list …`).
    Gh,
    /// Read-only git queries. Mutating git runs through
    /// [`super::destroy::git_worktree_prune`].
    Git,
    /// Free-space measurement (`df -k <path>`).
    Df,
    /// The user's uid for the launchd domain (`id -u`).
    Id,
    /// The scheduled refresh's LaunchAgent (`launchctl bootstrap|bootout|…`).
    Launchctl,
    /// Liveness of the observation lock holder (`kill -0 <pid>`).
    Kill,
    /// `brew --prefix` (detector query).
    Brew,
    /// `defaults read com.apple.dt.Xcode …` (detector query).
    Defaults,
}

impl Program {
    /// Every variant: the PATH-shim spawn oracle in the cost tests shims
    /// exactly these, so "spawns: []" means all of them.
    pub const ALL: &'static [Program] = &[
        Program::Lsof,
        Program::Plutil,
        Program::Xcrun,
        Program::Du,
        Program::Docker,
        Program::Gh,
        Program::Git,
        Program::Df,
        Program::Id,
        Program::Launchctl,
        Program::Kill,
        Program::Brew,
        Program::Defaults,
    ];

    /// The executable name looked up on `PATH`.
    pub fn binary(self) -> &'static str {
        match self {
            Program::Lsof => "lsof",
            Program::Plutil => "plutil",
            Program::Xcrun => "xcrun",
            Program::Du => "du",
            Program::Docker => "docker",
            Program::Gh => "gh",
            Program::Git => "git",
            Program::Df => "df",
            Program::Id => "id",
            Program::Launchctl => "launchctl",
            Program::Kill => "kill",
            Program::Brew => "brew",
            Program::Defaults => "defaults",
        }
    }

    /// The program named `name`, if swamp may run it.
    pub fn named(name: &str) -> Option<Program> {
        Program::ALL.iter().copied().find(|p| p.binary() == name)
    }
}

/// What a finished (or timed-out) run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    /// The exit code; `None` when killed by a signal or by the timeout.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

impl RunOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// The verbs [`run`] refuses because they change state outside swamp's
/// store: those go through [`super::destroy`] with an authorization.
fn mutating(program: Program, args: &[OsString]) -> Option<String> {
    let words: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let first = words.first().map(String::as_str).unwrap_or("");
    let second = words.get(1).map(String::as_str).unwrap_or("");
    let refused = match program {
        Program::Docker => {
            matches!(first, "rm" | "rmi" | "kill" | "stop" | "prune")
                || matches!(
                    (first, second),
                    (
                        "image" | "volume" | "container" | "network" | "builder" | "system",
                        "rm" | "prune" | "remove"
                    )
                )
        }
        Program::Git => {
            // Skip `-C <dir>` and other leading options to the subcommand.
            let mut i = 0;
            while i < words.len() && words[i].starts_with('-') {
                i += if words[i] == "-C" { 2 } else { 1 };
            }
            let sub = words.get(i).map(String::as_str);
            let next = words.get(i + 1).map(String::as_str);
            matches!(
                sub,
                Some(
                    "worktree"
                        | "gc"
                        | "prune"
                        | "reset"
                        | "clean"
                        | "checkout"
                        | "switch"
                        | "push"
                        | "commit"
                        | "rm"
                        | "branch"
                        | "stash"
                )
            ) && !matches!(next, Some("list"))
        }
        _ => false,
    };
    refused.then(|| {
        format!(
            "{} {} is a mutating verb; it runs only through fs_gate::destroy",
            program.binary(),
            words.join(" ")
        )
    })
}

/// Runs `program args…` with stdin closed, stdout and stderr captured to
/// anonymous temp files (so a chatty program can never deadlock a full
/// pipe), and kills it after `timeout`. Counted as one spawn.
pub fn run<I, S>(program: Program, args: I, timeout: Duration) -> io::Result<RunOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    if let Some(why) = mutating(program, &args) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
    }
    run_unchecked(program, &args, timeout)
}

/// [`run`] without the mutating-verb refusal: only
/// [`super::destroy`], behind an authorization, calls this.
pub(super) fn run_unchecked(
    program: Program,
    args: &[OsString],
    timeout: Duration,
) -> io::Result<RunOutput> {
    crate::work_counters::record_spawn();
    let mut out_file = tempfile::tempfile()?;
    let mut err_file = tempfile::tempfile()?;
    let mut child = Command::new(program.binary())
        .args(args)
        .stdin(Stdio::null())
        .stdout(out_file.try_clone()?)
        .stderr(err_file.try_clone()?)
        .spawn()?;
    let started = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
    };
    let read_back = |f: &mut std::fs::File| -> Vec<u8> {
        let mut buf = Vec::new();
        let _ = f.seek(SeekFrom::Start(0));
        let _ = f.read_to_end(&mut buf);
        buf
    };
    let stdout = read_back(&mut out_file);
    let stderr = read_back(&mut err_file);
    Ok(RunOutput {
        code,
        stdout,
        stderr,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_counts_one_spawn() {
        let (_, counted) =
            crate::work_counters::measured(|| run(Program::Id, ["-u"], Duration::from_secs(5)));
        assert_eq!(counted.subprocess_spawns, 1);
    }

    #[test]
    fn mutating_verbs_are_refused_without_spawning() {
        let (r, counted) = crate::work_counters::measured(|| {
            run(
                Program::Docker,
                ["image", "rm", "x"],
                Duration::from_secs(5),
            )
        });
        assert!(r.is_err());
        assert_eq!(counted.subprocess_spawns, 0);
        let (r, counted) = crate::work_counters::measured(|| {
            run(
                Program::Git,
                ["-C", "/tmp/x", "worktree", "prune"],
                Duration::from_secs(5),
            )
        });
        assert!(r.is_err());
        assert_eq!(counted.subprocess_spawns, 0);
        assert!(
            run(
                Program::Git,
                ["-C", "/nonexistent", "worktree", "list"],
                Duration::from_secs(5)
            )
            .is_ok(),
            "a read-only worktree listing is not a mutating verb"
        );
    }

    #[test]
    fn every_program_has_a_distinct_binary() {
        let mut names: Vec<&str> = Program::ALL.iter().map(|p| p.binary()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Program::ALL.len());
        for p in Program::ALL {
            assert_eq!(Program::named(p.binary()), Some(*p));
        }
    }
}
