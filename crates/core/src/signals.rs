//! Git activity signals: facts about a worktree's git state, never a
//! verdict. Every signal carries the value observed and when it was
//! observed; a timeout or parse failure records `Unknown`, never a guess.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::entities::now;
use crate::report::Signal;

/// Bound on any single `git` shell-out used for signals. Kept short: this
/// runs once per worktree in the report's hot path.
const SIGNAL_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalValue {
    LastCommitAgeSecs(u64),
    Dirty(bool),
    UnpushedCount(u32),
    Locked(bool),
    Unknown,
}

impl SignalValue {
    fn render(&self) -> String {
        match self {
            SignalValue::LastCommitAgeSecs(secs) => human_age(*secs),
            SignalValue::Dirty(true) => "dirty".to_string(),
            SignalValue::Dirty(false) => "clean".to_string(),
            SignalValue::UnpushedCount(n) => format!("{n} unpushed"),
            SignalValue::Locked(true) => "locked".to_string(),
            SignalValue::Locked(false) => "unlocked".to_string(),
            SignalValue::Unknown => "unknown".to_string(),
        }
    }
}

fn human_age(secs: u64) -> String {
    if secs < 3600 {
        format!("last commit {}m", (secs / 60).max(1))
    } else if secs < 86_400 {
        format!("last commit {}h", secs / 3600)
    } else {
        format!("last commit {}d", secs / 86_400)
    }
}

/// Runs `cmd`/`args` in `dir`, bounded by [`SIGNAL_TIMEOUT`]. Returns
/// `None` on failure, non-zero exit, or timeout (process is killed).
fn bounded_output(dir: &Path, cmd: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) => {
                if !status.success() {
                    return None;
                }
                let mut out = child.stdout.take()?;
                use std::io::Read;
                let mut buf = String::new();
                out.read_to_string(&mut buf).ok()?;
                return Some(buf);
            }
            None => {
                if start.elapsed() >= SIGNAL_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn last_commit_age(dir: &Path, observed_at: u64) -> SignalValue {
    match bounded_output(dir, "git", &["log", "-1", "--format=%ct"]) {
        Some(text) => match text.trim().parse::<u64>() {
            Ok(commit_ts) if commit_ts <= observed_at => {
                SignalValue::LastCommitAgeSecs(observed_at - commit_ts)
            }
            Ok(_) => SignalValue::LastCommitAgeSecs(0),
            Err(_) => SignalValue::Unknown,
        },
        None => SignalValue::Unknown,
    }
}

fn dirty(dir: &Path) -> SignalValue {
    match bounded_output(dir, "git", &["status", "--porcelain"]) {
        Some(text) => SignalValue::Dirty(!text.trim().is_empty()),
        None => SignalValue::Unknown,
    }
}

fn unpushed(dir: &Path) -> SignalValue {
    // No upstream configured is a normal, expected outcome, not a
    // failure; git exits non-zero and prints to stderr in that case, so
    // `bounded_output`'s `None` covers both "unknown" and "no upstream"
    // -- both render the same way: unpushed count unknown.
    match bounded_output(dir, "git", &["rev-list", "--count", "@{u}..HEAD"]) {
        Some(text) => match text.trim().parse::<u32>() {
            Ok(n) => SignalValue::UnpushedCount(n),
            Err(_) => SignalValue::Unknown,
        },
        None => SignalValue::Unknown,
    }
}

/// A worktree is locked when its administrative `.git/worktrees/<name>/locked`
/// file exists (linked worktrees only; a main checkout is never locked this
/// way). `git_dir_hint` is the worktree's own `.git` path (file or dir).
fn locked(dir: &Path) -> SignalValue {
    let git_path = dir.join(".git");
    let Ok(meta) = std::fs::symlink_metadata(&git_path) else {
        return SignalValue::Locked(false);
    };
    if meta.is_dir() {
        // Main checkout: never locked.
        return SignalValue::Locked(false);
    }
    // Linked worktree: `.git` is a file containing `gitdir: <admin dir>`.
    let Ok(contents) = std::fs::read_to_string(&git_path) else {
        return SignalValue::Unknown;
    };
    let Some(admin) = contents.trim().strip_prefix("gitdir:").map(str::trim) else {
        return SignalValue::Unknown;
    };
    SignalValue::Locked(Path::new(admin).join("locked").exists())
}

/// Computes all git activity signals for one worktree at `dir`, bounded
/// and side-effect free. `observed_at` is the report's single observation
/// timestamp so every worktree's ages are measured from the same instant.
pub fn compute_signals(dir: &Path, observed_at: u64) -> Vec<Signal> {
    vec![
        Signal {
            name: "last_commit".to_string(),
            value: last_commit_age(dir, observed_at).render(),
        },
        Signal {
            name: "dirty".to_string(),
            value: dirty(dir).render(),
        },
        Signal {
            name: "unpushed".to_string(),
            value: unpushed(dir).render(),
        },
        Signal {
            name: "locked".to_string(),
            value: locked(dir).render(),
        },
    ]
}

#[allow(dead_code)]
fn _unused_now_import() -> u64 {
    now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as PCommand;
    use tempfile::tempdir;

    fn git(dir: &Path, args: &[&str]) {
        let status = PCommand::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn fresh_commit_has_small_age_and_is_clean_with_unknown_upstream() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);

        let observed_at = now();
        let signals = compute_signals(dir.path(), observed_at);
        let by_name = |n: &str| signals.iter().find(|s| s.name == n).unwrap().value.clone();

        assert!(by_name("last_commit").starts_with("last commit"));
        assert_eq!(by_name("dirty"), "clean");
        assert_eq!(by_name("unpushed"), "unknown");
        assert_eq!(by_name("locked"), "unlocked");
    }

    #[test]
    fn dirty_worktree_reports_dirty() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        std::fs::write(dir.path().join("f.txt"), "changed").unwrap();

        let signals = compute_signals(dir.path(), now());
        let dirty = signals.iter().find(|s| s.name == "dirty").unwrap();
        assert_eq!(dirty.value, "dirty");
    }
}
