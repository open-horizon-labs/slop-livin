//! Git tracking status for paths inside a checkout: is this directory or
//! file **tracked**, **ignored**, or merely **untracked**?
//!
//! This is the decisive fact for a Source subtree. A tracked tree holds
//! authored work recoverable from the remote; an ignored tree is outside
//! version control entirely — generated output (rebuildable) or private
//! data (irrecoverable), and deleting it can never be undone with git.
//! The tool states the fact and never turns it into a verdict.

use gix::bstr::ByteSlice;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackState {
    /// At least one path here is in git's index.
    Tracked,
    /// Matched by a gitignore rule.
    Ignored,
    /// In the worktree, not in the index, not ignored.
    Untracked,
    /// No repository, or git could not answer.
    Unknown,
}

impl TrackState {
    pub fn label(self) -> &'static str {
        match self {
            TrackState::Tracked => "tracked",
            TrackState::Ignored => "ignored",
            TrackState::Untracked => "untracked",
            TrackState::Unknown => "",
        }
    }
}

/// An exclude stack plus index for one checkout, reused across many
/// lookups (building it per path would re-read every `.gitignore`).
pub struct IgnoreLens {
    repo: gix::Repository,
    index: gix::index::State,
}

impl IgnoreLens {
    /// Opens the checkout at `root`. `None` when it is not a repository.
    pub fn open(root: &Path) -> Option<Self> {
        let repo = gix::open(root).ok()?;
        let snapshot = repo.index_or_empty().ok()?;
        let index: gix::index::State = (**snapshot).clone().into();
        Some(Self { repo, index })
    }

    /// Status of `rel` (relative to the checkout root) — `is_dir` matters
    /// because gitignore rules can be directory-only.
    pub fn status(&self, rel: &str, is_dir: bool) -> TrackState {
        let rel_trimmed = rel
            .trim_start_matches("./")
            .trim_end_matches('/')
            .trim_end_matches('.');
        let rel_trimmed = rel_trimmed.trim_end_matches('/');
        if rel_trimmed.is_empty() {
            return TrackState::Tracked; // the checkout root itself
        }
        // Tracked wins: a path with any index entry under it is tracked,
        // even if a broad ignore rule would also match it.
        let bytes = rel_trimmed.as_bytes().as_bstr();
        if self.index.entry_by_path(bytes).is_some() {
            return TrackState::Tracked;
        }
        if is_dir {
            let with_slash = format!("{rel_trimmed}/");
            if self
                .index
                .prefixed_entries(with_slash.as_bytes().as_bstr())
                .is_some_and(|e| !e.is_empty())
            {
                return TrackState::Tracked;
            }
        }
        let Ok(mut stack) = self.repo.excludes(
            &self.index,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        ) else {
            return TrackState::Unknown;
        };
        // A directory is ignored when everything inside it is: gix's stack
        // answers about a directory's *contents*, so ask about a probe path
        // inside it rather than the directory itself (a trailing slash
        // alone returns the container's state, not the rule's effect).
        let lookup = if is_dir {
            format!("{rel_trimmed}/.slop-livin-probe")
        } else {
            rel_trimmed.to_string()
        };
        let mode = is_dir.then_some(gix::index::entry::Mode::FILE);
        match stack.at_entry(lookup.as_bytes().as_bstr(), mode) {
            Ok(platform) => {
                if platform.is_excluded() {
                    TrackState::Ignored
                } else {
                    TrackState::Untracked
                }
            }
            Err(_) => TrackState::Unknown,
        }
    }
}

/// The first few paths under `root` that git neither tracks nor ignores,
/// with their sizes: content that exists **only here**. Removing a whole
/// checkout destroys these, and nothing (remote, rebuild) brings them
/// back — so they are the bar a whole-checkout removal has to clear.
///
/// Bounded: stops after `limit` findings or `max_entries` directory
/// entries, and never descends into an ignored directory.
pub fn untracked_content(
    root: &Path,
    limit: usize,
    max_entries: usize,
) -> Vec<(std::path::PathBuf, u64)> {
    let Some(lens) = IgnoreLens::open(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        if found.len() >= limit || seen > max_entries {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            seen += 1;
            if found.len() >= limit || seen > max_entries {
                break;
            }
            let path = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel = rel.display().to_string();
            if rel == ".git" {
                continue;
            }
            let is_dir = meta.is_dir();
            match lens.status(&rel, is_dir) {
                TrackState::Ignored => {}
                TrackState::Untracked => {
                    let bytes = if is_dir {
                        crate::walk::resize_artifact(&path, crate::report::ArtifactKind::Unknown, 0)
                            .bytes
                    } else {
                        meta.len()
                    };
                    found.push((path.clone(), bytes));
                }
                TrackState::Tracked | TrackState::Unknown => {
                    if is_dir {
                        stack.push(path);
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn tracked_ignored_and_untracked_are_distinguished() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "t@e"]);
        git(root, &["config", "user.name", "t"]);
        std::fs::write(root.join(".gitignore"), "raw/\n*.log\nbuild/\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(root.join("raw")).unwrap();
        std::fs::write(root.join("raw/data.csv"), "a,b").unwrap();
        std::fs::create_dir_all(root.join("scratch")).unwrap();
        std::fs::write(root.join("scratch/note.txt"), "x").unwrap();
        std::fs::write(root.join("run.log"), "x").unwrap();
        git(root, &["add", ".gitignore", "src/main.rs"]);
        git(root, &["commit", "-qm", "init"]);

        let lens = IgnoreLens::open(root).expect("repo opens");
        assert_eq!(lens.status("src", true), TrackState::Tracked);
        assert_eq!(lens.status("src/main.rs", false), TrackState::Tracked);
        assert_eq!(lens.status("raw", true), TrackState::Ignored);
        assert_eq!(lens.status("run.log", false), TrackState::Ignored);
        assert_eq!(lens.status("scratch", true), TrackState::Untracked);
        assert_eq!(
            lens.status("scratch/note.txt", false),
            TrackState::Untracked
        );
        assert_eq!(lens.status(".", true), TrackState::Tracked);
    }

    #[test]
    fn a_non_repository_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(IgnoreLens::open(tmp.path()).is_none());
    }
}
