//! Git activity signals: facts about a worktree's git state, never a
//! verdict. Every signal carries the value observed; a timeout, missing
//! upstream, or parse failure records `Unknown`, never a guess.
//!
//! Computed with `gix` (gitoxide) directly against the on-disk object
//! store during discovery, in the same thread pool as the rest of the
//! walk (see `walk.rs`). No `git` subprocess is spawned here.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::report::Signal;

/// Bound on the `status` (dirty) walk: if a repo's status has more
/// entries than this, or takes longer than [`STATUS_TIMEOUT`], the
/// result is `Unknown` rather than a partial/guessed answer.
const STATUS_ENTRY_CAP: usize = 4_000;
const STATUS_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalValue {
    LastCommitAgeSecs(u64),
    Dirty(bool),
    UnpushedCount(u32),
    UnpushedUnknownNoUpstream,
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
            SignalValue::UnpushedUnknownNoUpstream => "unknown (no upstream)".to_string(),
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

/// HEAD commit time, read straight from the commit object. `Unknown` on
/// an unborn HEAD, a corrupt object, or a commit timestamp we cannot
/// trust (negative/unparsable).
fn last_commit_age(repo: &gix::Repository, observed_at: u64) -> SignalValue {
    let Ok(commit) = repo.head_commit() else {
        return SignalValue::Unknown;
    };
    let Ok(time) = commit.time() else {
        return SignalValue::Unknown;
    };
    if time.seconds < 0 {
        return SignalValue::Unknown;
    }
    let commit_ts = time.seconds as u64;
    if commit_ts <= observed_at {
        SignalValue::LastCommitAgeSecs(observed_at - commit_ts)
    } else {
        SignalValue::LastCommitAgeSecs(0)
    }
}

/// Dirty = any index/worktree change, from `gix`'s status walk (covers
/// modified/added/removed/untracked, mirroring `git status --porcelain`).
/// Bounded by both an entry cap and a wall-clock timeout: a pathological
/// worktree (huge untracked tree, slow filesystem) degrades to `Unknown`
/// instead of blocking the report.
fn dirty(repo: &gix::Repository) -> SignalValue {
    let start = Instant::now();
    let platform = match repo.status(gix::progress::Discard) {
        Ok(p) => p,
        Err(_) => return SignalValue::Unknown,
    };
    let iter = match platform.into_iter(None) {
        Ok(iter) => iter,
        Err(_) => return SignalValue::Unknown,
    };
    let mut count = 0usize;
    let mut any = false;
    for item in iter {
        if start.elapsed() >= STATUS_TIMEOUT {
            return SignalValue::Unknown;
        }
        count += 1;
        if count > STATUS_ENTRY_CAP {
            return SignalValue::Unknown;
        }
        match item {
            Ok(_) => any = true,
            Err(_) => return SignalValue::Unknown,
        }
    }
    SignalValue::Dirty(any)
}

/// Ahead-of-upstream commit count, via a revwalk from HEAD hidden behind
/// the upstream tracking ref. `UnpushedUnknownNoUpstream` (not `Unknown`)
/// when no upstream is configured for the current branch -- this is a
/// normal, expected state, not a failure, and the renderer labels it
/// distinctly (`unpushed: unknown (no upstream)`).
fn unpushed(repo: &gix::Repository) -> SignalValue {
    let Ok(head) = repo.head() else {
        return SignalValue::Unknown;
    };
    let Some(branch_name) = head.referent_name().map(|n| n.to_owned()) else {
        // Detached HEAD: no branch, so no upstream concept applies.
        return SignalValue::UnpushedUnknownNoUpstream;
    };
    let tracking_name = match repo
        .branch_remote_tracking_ref_name(branch_name.as_ref(), gix::remote::Direction::Fetch)
    {
        Some(Ok(name)) => name,
        Some(Err(_)) | None => return SignalValue::UnpushedUnknownNoUpstream,
    };
    let Ok(mut tracking_ref) = repo.find_reference(tracking_name.as_ref()) else {
        return SignalValue::UnpushedUnknownNoUpstream;
    };
    let Ok(tracking_id) = tracking_ref.peel_to_id() else {
        return SignalValue::Unknown;
    };
    let Ok(head_id) = repo.head_id() else {
        return SignalValue::Unknown;
    };
    let walk = repo
        .rev_walk([head_id.detach()])
        .with_hidden([tracking_id.detach()])
        .all();
    match walk {
        Ok(iter) => {
            let mut n: u32 = 0;
            for item in iter {
                if item.is_err() {
                    return SignalValue::Unknown;
                }
                n += 1;
                if n == u32::MAX {
                    break;
                }
            }
            SignalValue::UnpushedCount(n)
        }
        Err(_) => SignalValue::Unknown,
    }
}

/// A worktree is locked when its administrative `<gitdir>/locked` file
/// exists (linked worktrees only; a main checkout is never locked this
/// way). Pure filesystem check via `gix`'s worktree proxy -- no object
/// database access, so it never depends on `repo` having opened cleanly.
fn locked(repo: &gix::Repository) -> SignalValue {
    match repo.worktree() {
        Some(wt) => SignalValue::Locked(wt.is_locked()),
        None => SignalValue::Locked(false),
    }
}

/// Computes all git activity signals for one worktree at `dir`, bounded
/// and side-effect free. `observed_at` is the report's single observation
/// timestamp so every worktree's ages are measured from the same instant.
pub fn compute_signals(dir: &Path, observed_at: u64) -> Vec<Signal> {
    let Ok(repo) = gix::open(dir) else {
        let unknown = SignalValue::Unknown.render();
        return vec![
            Signal {
                name: "last_commit".to_string(),
                value: unknown.clone(),
            },
            Signal {
                name: "dirty".to_string(),
                value: unknown.clone(),
            },
            Signal {
                name: "unpushed".to_string(),
                value: unknown,
            },
            Signal {
                name: "locked".to_string(),
                value: SignalValue::Locked(false).render(),
            },
        ];
    };

    vec![
        Signal {
            name: "last_commit".to_string(),
            value: last_commit_age(&repo, observed_at).render(),
        },
        Signal {
            name: "dirty".to_string(),
            value: dirty(&repo).render(),
        },
        Signal {
            name: "unpushed".to_string(),
            value: unpushed(&repo).render(),
        },
        Signal {
            name: "locked".to_string(),
            value: locked(&repo).render(),
        },
    ]
}

/// Parallel equivalent of calling [`compute_signals`] once per path,
/// preserving input order. Each worktree's git signals are independent
/// (own `gix::open`, own object store access), so this fans them out
/// across a small worker pool instead of computing them one at a time in
/// the report's hot path -- the same rationale `walk.rs` documents for
/// discovery/attribution, applied to signals.
pub fn compute_signals_parallel(
    paths: &[std::path::PathBuf],
    observed_at: u64,
) -> Vec<Vec<Signal>> {
    let n = paths.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = std::thread::available_parallelism()
        .map(|c| c.get())
        .unwrap_or(4)
        .min(n)
        .max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Vec<Signal>>> =
        (0..n).map(|_| std::sync::Mutex::new(Vec::new())).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let next = &next;
            let results = &results;
            let paths = &paths;
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if i >= n {
                        break;
                    }
                    let sigs = compute_signals(&paths[i], observed_at);
                    *results[i].lock().unwrap() = sigs;
                }
            });
        }
    });
    results
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::now;
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
    fn fresh_commit_has_small_age_and_is_clean_with_no_upstream() {
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
        assert_eq!(by_name("unpushed"), "unknown (no upstream)");
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

    #[test]
    fn unpushed_count_reflects_commits_ahead_of_upstream() {
        let remote_dir = tempdir().unwrap();
        git(remote_dir.path(), &["init", "-q", "--bare"]);

        let dir = tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        git(
            dir.path(),
            &[
                "remote",
                "add",
                "origin",
                remote_dir.path().to_str().unwrap(),
            ],
        );
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        git(dir.path(), &["push", "-q", "-u", "origin", "main"]);

        std::fs::write(dir.path().join("f2.txt"), "more").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "second"]);

        let signals = compute_signals(dir.path(), now());
        let unpushed = signals.iter().find(|s| s.name == "unpushed").unwrap();
        assert_eq!(unpushed.value, "1 unpushed");
    }

    #[test]
    fn linked_worktree_lock_file_is_detected() {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);

        let linked_parent = tempdir().unwrap();
        let linked = linked_parent.path().join("linked-wt");
        git(
            dir.path(),
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().unwrap(),
                "-b",
                "linked",
            ],
        );

        let signals = compute_signals(&linked, now());
        let locked = signals.iter().find(|s| s.name == "locked").unwrap();
        assert_eq!(locked.value, "unlocked");

        git(dir.path(), &["worktree", "lock", linked.to_str().unwrap()]);
        let signals = compute_signals(&linked, now());
        let locked = signals.iter().find(|s| s.name == "locked").unwrap();
        assert_eq!(locked.value, "locked");
    }
}
