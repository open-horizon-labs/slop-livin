//! Aider identification (#96): a home-level, wholly re-downloadable
//! cache root under `~/.aider` (`crate::locations::aider::AiderDetector`
//! resolves the home), and -- materially different from every other
//! adapter in this catalog -- per-repo history/tags-cache files that
//! live **inside each project checkout** rather than under any tool
//! home. See `crate::locations::aider`'s doc comment for the full
//! primary-source citations
//! (`aider/models.py`/`versioncheck.py`/`args.py`/`repomap.py`, current
//! `main` of <https://github.com/Aider-AI/aider> as of this chunk).
//!
//! [`identify`] covers the home: `caches/` (regenerable) and, if
//! present, a home-level `.aider.conf.yml`. [`identify_repo_units`]
//! covers the per-repo files and is called once per **known project
//! worktree root** by `crate::agents::discover_and_measure`'s
//! `project_worktrees` parameter -- #96's explicit acceptance ("attach
//! to the existing worktree artifact model as an agent category, not a
//! tool-home unit"). A worktree root is itself the "declared path" here
//! (the caller already knows it is a real checkout), so project linkage
//! reuses `resolve_declared_path` against that same root rather than
//! inventing a second mechanism.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::fs;
use std::path::Path;

pub const AIDER_TOOL_ID: &str = crate::locations::aider::AIDER_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;

// ---------------------------------------------------------------------
// Home-level: ~/.aider/caches (+ an optional home-level .aider.conf.yml).
// ---------------------------------------------------------------------

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.is_dir() {
        return Vec::new();
    }
    let mut units = Vec::new();

    let caches = home.join("caches");
    let caches_existed = caches.exists();
    if caches.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&caches, MAX_FOLD_ENTRIES);
        units.push(CandidateAgentUnit {
            category: AgentCategory::Caches,
            relative_path: "caches".to_string(),
            path: caches,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "model-price/context-window and version-check caches, wholly re-downloadable; \
                 directory entry count bound reached"
                    .to_string()
            } else {
                "model-price/context-window and version-check caches, wholly re-downloadable"
                    .to_string()
            }),
        });
    }

    let conf = home.join(".aider.conf.yml");
    if let Ok(meta) = fs::symlink_metadata(&conf)
        && meta.is_file()
    {
        units.push(CandidateAgentUnit {
            category: AgentCategory::ProtectedConfig,
            relative_path: ".aider.conf.yml".to_string(),
            path: conf,
            members: Vec::new(),
            bytes: meta.len(),
            mtime_max: mtime_secs(&meta),
            protected: true,
            protect_reason: Some("home-level Aider configuration".to_string()),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: None,
        });
    }

    if !caches_existed && !home.join(".aider.conf.yml").exists() {
        // A genuinely empty/unrelated ~/.aider is not "unsupported
        // version" the way an ambiguous ~/.omp or ~/.opencode-shaped
        // directory would be -- Aider has no other documented top-level
        // marker this chunk found, so there is nothing to classify, not
        // an unknown format to flag.
        let has_entries = fs::read_dir(home)
            .map(|mut rd| rd.next().is_some())
            .unwrap_or(false);
        if has_entries {
            let (bytes, mtime, _t) = folded_bytes(home, MAX_FOLD_ENTRIES);
            units.push(CandidateAgentUnit {
                category: AgentCategory::Unclassified,
                relative_path: "(unclassified residual)".to_string(),
                path: home.to_path_buf(),
                members: Vec::new(),
                bytes,
                mtime_max: mtime,
                protected: false,
                protect_reason: None,
                project_link: ProjectLinkState::NotApplicable,
                action: AgentActionCapability::None,
                note: Some(
                    "no recognized Aider home markers (caches/, .aider.conf.yml) found here"
                        .to_string(),
                ),
            });
        }
    }
    units
}

// ---------------------------------------------------------------------
// Per-repo: identified against a known worktree root, not a tool home.
// ---------------------------------------------------------------------

/// The three per-repo Aider files/directories this chunk's primary-source
/// research confirmed, checked directly at `worktree_root` (never a
/// recursive search -- these files live exactly at the git root per
/// `aider/args.py`/`repomap.py`).
pub fn identify_repo_units(worktree_root: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !worktree_root.is_dir() {
        return Vec::new();
    }
    let project_link = resolve_declared_path(
        Some(worktree_root.display().to_string()),
        "worktree root supplied by the caller was not itself resolvable",
    );
    let mut units = Vec::new();

    for (rel, note) in [
        (
            ".aider.chat.history.md",
            "unique chat transcript for this checkout; not regenerated by re-running Aider",
        ),
        (
            ".aider.input.history",
            "unique input-line history for this checkout; not regenerated by re-running Aider",
        ),
    ] {
        let path = worktree_root.join(rel);
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_file()
        {
            units.push(CandidateAgentUnit {
                category: AgentCategory::Sessions,
                relative_path: rel.to_string(),
                path: path.clone(),
                members: vec![AgentMember {
                    path,
                    bytes: meta.len(),
                    kind: AgentMemberKind::Transcript,
                }],
                bytes: meta.len(),
                mtime_max: mtime_secs(&meta),
                protected: false,
                protect_reason: None,
                project_link: project_link.clone(),
                action: AgentActionCapability::SessionRemoval,
                note: Some(note.to_string()),
            });
        }
    }

    // `.aider.tags.cache.v{3,4}` -- the version number is a repomap
    // implementation detail (whether the optional TSL pack is in use),
    // checked directly rather than guessed at one fixed number.
    let Ok(rd) = fs::read_dir(worktree_root) else {
        return units;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(".aider.tags.cache.v") {
            continue;
        }
        let path = entry.path();
        let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
        units.push(CandidateAgentUnit {
            category: AgentCategory::Caches,
            relative_path: name,
            path,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: project_link.clone(),
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "repo-map tags cache, regenerated on next Aider run; directory entry count bound \
                 reached"
                    .to_string()
            } else {
                "repo-map tags cache, regenerated on next Aider run".to_string()
            }),
        });
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn empty_home_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identify(dir.path(), 1).is_empty());
    }

    #[test]
    fn caches_dir_is_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(
            &home.join("caches/model_prices_and_context_window.json"),
            b"{}",
        );
        touch(&home.join("caches/versioncheck"), b"");
        let units = identify(home, 1);
        let u = units.iter().find(|u| u.relative_path == "caches").unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn home_conf_is_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join(".aider.conf.yml"), b"dark-mode: true");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == ".aider.conf.yml")
            .unwrap();
        assert!(u.protected);
    }

    #[test]
    fn repo_units_are_not_identified_without_a_known_worktree_root() {
        // identify() (home-level) must never reach into a project
        // checkout for these files -- that is identify_repo_units's job.
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        fs::create_dir_all(home.join(".git")).unwrap();
        touch(
            &home.join(".aider.chat.history.md"),
            b"# aider chat history",
        );
        let units = identify(home, 1);
        assert!(
            units
                .iter()
                .all(|u| u.relative_path != ".aider.chat.history.md"),
            "home-level identify must not pick up per-repo files"
        );
    }

    #[test]
    fn repo_history_files_are_linked_and_actionable() {
        let repo = tempfile::tempdir().unwrap();
        let repo = repo.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        let canary = "CANARY-AIDER-DO-NOT-LEAK-55dd";
        touch(
            &repo.join(".aider.chat.history.md"),
            format!("# aider chat started\n\nUser: {canary}\n").as_bytes(),
        );
        touch(&repo.join(".aider.input.history"), b"+++ /help\n");
        let units = identify_repo_units(repo, 1);
        assert_eq!(units.len(), 2);
        for u in &units {
            assert_eq!(u.category, AgentCategory::Sessions);
            assert_eq!(u.action, AgentActionCapability::SessionRemoval);
            assert!(matches!(u.project_link, ProjectLinkState::Linked { .. }));
        }
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "chat content leaked");
    }

    #[test]
    fn tags_cache_is_versioned_and_regenerable() {
        let repo = tempfile::tempdir().unwrap();
        let repo = repo.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        touch(&repo.join(".aider.tags.cache.v3/tags.db"), b"cache-bytes");
        let units = identify_repo_units(repo, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == ".aider.tags.cache.v3")
            .expect("tags cache identified");
        assert_eq!(u.category, AgentCategory::Caches);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn a_worktree_with_no_aider_files_yields_nothing() {
        let repo = tempfile::tempdir().unwrap();
        let repo = repo.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        assert!(identify_repo_units(repo, 1).is_empty());
    }

    #[test]
    fn identification_cost_is_bounded_for_a_large_tags_cache() {
        let repo = tempfile::tempdir().unwrap();
        let repo = repo.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        for i in 0..2000 {
            touch(
                &repo.join(format!(".aider.tags.cache.v3/f{i}")),
                &b"x".repeat(2_000),
            );
        }
        let start = SystemTime::now();
        let units = identify_repo_units(repo, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!(
            "[measured] aider identify_repo_units() over a 2000-file tags cache took {elapsed:?}"
        );
        assert_eq!(units.len(), 1);
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
    }
}
