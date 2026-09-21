//! Gemini CLI identification (#96): per-project-hash temp state (shell
//! history, checkpoints, saved chats), shadow-Git checkpoint history and
//! protected configuration under the home
//! `crate::locations::gemini_cli::GeminiCliDetector` resolves.
//!
//! Layout and the `getProjectHash` algorithm are sourced from primary
//! docs/source during implementation (never a real `~/.gemini` on this
//! machine -- PRIVACY IS A HARD RULE); see
//! `crate::locations::gemini_cli`'s doc comment for the full citations.
//!
//! ## Project-hash linkage (#96's explicit bar)
//!
//! `getProjectHash(projectRoot) = sha256(projectRoot).hex()` is
//! confirmed, not guessed -- but it is a **one-way** function. Given a
//! `tmp/<hash>` or `history/<hash>` directory name, this adapter cannot
//! recover the project root that produced it without hashing every
//! candidate path in a project catalog this identification layer does
//! not have (`identify` only receives this tool's own home path). Rather
//! than fabricate a match or silently drop the fact that a real,
//! documented algorithm exists, every unit under a hash directory
//! carries `ProjectLinkState::Unresolved` with a reason naming the
//! algorithm explicitly. A future caller with a project catalog in hand
//! (e.g. the CLI's own worktree list) could resolve this by hashing each
//! candidate and comparing -- not implemented this chunk.
//!
//! ## Version-aware boundary
//!
//! Checked content markers: `settings.json`, `GEMINI.md` (or another
//! configured context filename -- only the default is checked),
//! `extensions/`, `tmp/`, `history/`, `trustedFolders.json`, `bin/`. None
//! present but the directory non-empty -> one `Unclassified`,
//! non-actionable "unsupported layout version" residual, same discipline
//! `crate::agents::opencode`/`oh_my_pi` use.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub const GEMINI_CLI_TOOL_ID: &str = crate::locations::gemini_cli::GEMINI_CLI_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;
const HASH_UNRESOLVED_REASON: &str = "Gemini CLI's project hash is sha256(project root path) per \
     packages/core/src/utils/paths.ts's getProjectHash, a one-way function; this adapter cannot \
     recover the project root from the hash alone without a candidate project-path catalog";

const FORMAT_MARKERS: &[&str] = &[
    "settings.json",
    "GEMINI.md",
    "extensions",
    "tmp",
    "history",
    "trustedFolders.json",
    "bin",
];

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.is_dir() {
        return Vec::new();
    }
    if !FORMAT_MARKERS.iter().any(|rel| home.join(rel).exists()) {
        return unknown_version_residual(home);
    }
    let mut units = Vec::new();
    identify_protected(home, &mut units);
    identify_tmp(home, &mut units);
    identify_history(home, &mut units);
    identify_residual(home, &mut units);
    units
}

fn unknown_version_residual(home: &Path) -> Vec<CandidateAgentUnit> {
    let has_entries = fs::read_dir(home)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, _t) = folded_bytes(home, MAX_FOLD_ENTRIES);
    vec![CandidateAgentUnit {
        category: AgentCategory::Unclassified,
        relative_path: "(unsupported layout version)".to_string(),
        path: home.to_path_buf(),
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: Some(
            "no recognized Gemini CLI markers found (settings.json/GEMINI.md/extensions/tmp/\
             history/trustedFolders.json/bin) at this resolved path -- unsupported or future \
             layout version, treated as unknown, not scanned further"
                .to_string(),
        ),
    }]
}

fn identify_protected(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    for (rel, note) in [
        ("settings.json", "user settings"),
        ("GEMINI.md", "context/memory file"),
        ("trustedFolders.json", "trusted-folder decisions"),
    ] {
        let path = home.join(rel);
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_file()
        {
            out.push(CandidateAgentUnit {
                category: AgentCategory::ProtectedConfig,
                relative_path: rel.to_string(),
                path,
                members: Vec::new(),
                bytes: meta.len(),
                mtime_max: mtime_secs(&meta),
                protected: true,
                protect_reason: Some(note.to_string()),
                project_link: ProjectLinkState::NotApplicable,
                action: AgentActionCapability::None,
                note: None,
            });
        }
    }
    // Defensive filename-pattern check (guardrail precedent:
    // `crate::actions::is_sqlite_like`): no primary source this chunk
    // named the exact OAuth/account credential file(s) --
    // docs/cli/authentication.md and docs/get-started/authentication.md
    // both 404 against current main -- so any top-level file whose name
    // looks credential-shaped is protected rather than left unprotected
    // pending an unconfirmed filename.
    if let Ok(rd) = fs::read_dir(home) {
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if (name.contains("oauth") || name.contains("cred")) && !name.starts_with('.') {
                let path = e.path();
                if let Ok(meta) = e.metadata() {
                    out.push(CandidateAgentUnit {
                        category: AgentCategory::ProtectedConfig,
                        relative_path: e.file_name().to_string_lossy().into_owned(),
                        path,
                        members: Vec::new(),
                        bytes: meta.len(),
                        mtime_max: mtime_secs(&meta),
                        protected: true,
                        protect_reason: Some(
                            "credential-shaped filename (defensive pattern match; exact upstream \
                             name not confirmed this chunk)"
                                .to_string(),
                        ),
                        project_link: ProjectLinkState::NotApplicable,
                        action: AgentActionCapability::None,
                        note: None,
                    });
                }
            }
        }
    }
    let extensions = home.join("extensions");
    if extensions.is_dir() {
        let (bytes, mtime, _t) = folded_bytes(&extensions, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Plugins,
            relative_path: "extensions".to_string(),
            path: extensions,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: true,
            protect_reason: Some(
                "installed extensions; removing breaks the CLI's configured integrations"
                    .to_string(),
            ),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: None,
        });
    }
    let bin = home.join("bin");
    if bin.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&bin, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Caches,
            relative_path: "bin".to_string(),
            path: bin,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "downloaded runtime tools (e.g. LiteRT-LM), re-downloadable; directory entry \
                 count bound reached"
                    .to_string()
            } else {
                "downloaded runtime tools (e.g. LiteRT-LM), re-downloadable".to_string()
            }),
        });
    }
}

fn identify_tmp(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let base = home.join("tmp");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for hash_entry in rd.flatten() {
        let Ok(ft) = hash_entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let hash_dir = hash_entry.path();
        let mut seen: HashSet<&str> = HashSet::new();

        let shell_history = hash_dir.join("shell_history");
        if let Ok(meta) = fs::symlink_metadata(&shell_history)
            && meta.is_file()
        {
            seen.insert("shell_history");
            out.push(CandidateAgentUnit {
                category: AgentCategory::Logs,
                relative_path: relative_to(home, &shell_history),
                path: shell_history,
                members: Vec::new(),
                bytes: meta.len(),
                mtime_max: mtime_secs(&meta),
                protected: false,
                protect_reason: None,
                project_link: ProjectLinkState::Unresolved {
                    reason: HASH_UNRESOLVED_REASON.to_string(),
                },
                action: AgentActionCapability::CacheOrLogTrash,
                note: Some("per-project shell command history for this CLI session".to_string()),
            });
        }

        let checkpoints = hash_dir.join("checkpoints");
        if checkpoints.is_dir() {
            seen.insert("checkpoints");
            let (bytes, mtime, truncated) = folded_bytes(&checkpoints, MAX_FOLD_ENTRIES);
            out.push(CandidateAgentUnit {
                category: AgentCategory::Checkpoints,
                relative_path: relative_to(home, &checkpoints),
                path: checkpoints,
                members: Vec::new(),
                bytes,
                mtime_max: mtime,
                protected: false,
                protect_reason: None,
                project_link: ProjectLinkState::Unresolved {
                    reason: HASH_UNRESOLVED_REASON.to_string(),
                },
                action: AgentActionCapability::None,
                note: Some(if truncated {
                    "tool-call checkpoint state for /restore; not a supported selective action \
                     this chunk (directory entry count bound reached)"
                        .to_string()
                } else {
                    "tool-call checkpoint state for /restore; not a supported selective action \
                     this chunk"
                        .to_string()
                }),
            });
        }

        let chats = hash_dir.join("chats");
        if let Ok(chat_rd) = fs::read_dir(&chats) {
            seen.insert("chats");
            for chat_entry in chat_rd.flatten() {
                let Ok(cft) = chat_entry.file_type() else {
                    continue;
                };
                if !cft.is_file() {
                    continue;
                }
                let path = chat_entry.path();
                let Ok(meta) = chat_entry.metadata() else {
                    continue;
                };
                out.push(CandidateAgentUnit {
                    category: AgentCategory::Sessions,
                    relative_path: relative_to(home, &path),
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
                    project_link: ProjectLinkState::Unresolved {
                        reason: HASH_UNRESOLVED_REASON.to_string(),
                    },
                    action: AgentActionCapability::SessionRemoval,
                    note: Some("saved chat (/chat save, /resume)".to_string()),
                });
            }
        }

        let (residual_bytes, residual_mtime, residual_names) =
            fold_residual_children(&hash_dir, &seen);
        if !residual_names.is_empty() {
            out.push(CandidateAgentUnit {
                category: AgentCategory::Unclassified,
                relative_path: format!("{} (unclassified residual)", relative_to(home, &hash_dir)),
                path: hash_dir.clone(),
                members: Vec::new(),
                bytes: residual_bytes,
                mtime_max: residual_mtime,
                protected: false,
                protect_reason: None,
                project_link: ProjectLinkState::Unresolved {
                    reason: HASH_UNRESOLVED_REASON.to_string(),
                },
                action: AgentActionCapability::None,
                note: Some(format!(
                    "entries with no specific rule in this adapter: {}",
                    residual_names.join(", ")
                )),
            });
        }
    }
}

fn identify_history(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let base = home.join("history");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Checkpoints,
            relative_path: relative_to(home, &path),
            path,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::Unresolved {
                reason: HASH_UNRESOLVED_REASON.to_string(),
            },
            action: AgentActionCapability::None,
            note: Some(if truncated {
                "shadow Git repository backing this project's /restore checkpoints, independent \
                 of the project's own .git; not a supported selective action this chunk \
                 (directory entry count bound reached)"
                    .to_string()
            } else {
                "shadow Git repository backing this project's /restore checkpoints, independent \
                 of the project's own .git; not a supported selective action this chunk"
                    .to_string()
            }),
        });
    }
}

fn fold_residual_children(dir: &Path, seen: &HashSet<&str>) -> (u64, u64, Vec<String>) {
    let mut bytes = 0u64;
    let mut mtime = 0u64;
    let mut names = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return (bytes, mtime, names);
    };
    for e in rd.flatten() {
        let Some(name) = e
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if seen.contains(name.as_str()) {
            continue;
        }
        let (b, m, _t) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
        bytes += b;
        mtime = mtime.max(m);
        names.push(name);
    }
    names.sort();
    (bytes, mtime, names)
}

fn identify_residual(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let seen: HashSet<&str> = [
        "settings.json",
        "GEMINI.md",
        "trustedFolders.json",
        "extensions",
        "bin",
        "tmp",
        "history",
    ]
    .into_iter()
    .collect();
    let (bytes, mtime, names) = fold_residual_children(home, &seen);
    // Credential-shaped filenames were already claimed above; exclude
    // them from the generic residual so they are not double-counted.
    let names: Vec<String> = names
        .into_iter()
        .filter(|n| {
            let lower = n.to_ascii_lowercase();
            !((lower.contains("oauth") || lower.contains("cred")) && !lower.starts_with('.'))
        })
        .collect();
    if names.is_empty() {
        return;
    }
    out.push(CandidateAgentUnit {
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
        note: Some(format!(
            "entries with no specific rule in this adapter: {}",
            names.join(", ")
        )),
    });
}

fn relative_to(home: &Path, path: &Path) -> String {
    path.strip_prefix(home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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
    fn no_markers_yields_unsupported_version_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("unrelated.txt"), b"hello");
        let units = identify(dir.path(), 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].relative_path, "(unsupported layout version)");
    }

    #[test]
    fn settings_and_gemini_md_are_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(&home.join("GEMINI.md"), b"# context");
        let units = identify(home, 1);
        assert!(
            units
                .iter()
                .find(|u| u.relative_path == "settings.json")
                .unwrap()
                .protected
        );
        assert!(
            units
                .iter()
                .find(|u| u.relative_path == "GEMINI.md")
                .unwrap()
                .protected
        );
    }

    #[test]
    fn credential_shaped_filename_is_protected_defensively() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(&home.join("oauth_creds.json"), b"[redacted]");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "oauth_creds.json")
            .expect("credential-shaped file identified");
        assert!(u.protected);
        assert_eq!(u.action, AgentActionCapability::None);
    }

    #[test]
    fn extensions_are_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("extensions/foo/package.json"), b"{}");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "extensions")
            .unwrap();
        assert!(u.protected);
    }

    #[test]
    fn tmp_hash_dir_children_are_categorized_and_hash_is_unresolved() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let hash = "e".repeat(64);
        touch(&home.join(format!("tmp/{hash}/shell_history")), b"ls\ncd\n");
        touch(&home.join(format!("tmp/{hash}/checkpoints/1.json")), b"{}");
        let canary = "CANARY-GEMINI-DO-NOT-LEAK-33ff";
        touch(
            &home.join(format!("tmp/{hash}/chats/decision-point.json")),
            format!("{{\"note\":\"{canary}\"}}").as_bytes(),
        );
        let units = identify(home, 1);
        let shell = units
            .iter()
            .find(|u| u.category == AgentCategory::Logs)
            .expect("shell history");
        assert!(matches!(
            shell.project_link,
            ProjectLinkState::Unresolved { .. }
        ));
        let checkpoint = units
            .iter()
            .find(|u| u.category == AgentCategory::Checkpoints)
            .expect("checkpoint dir");
        assert_eq!(checkpoint.action, AgentActionCapability::None);
        let chat = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions)
            .expect("saved chat");
        assert_eq!(chat.action, AgentActionCapability::SessionRemoval);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "chat content leaked");
    }

    #[test]
    fn history_shadow_repo_is_checkpoint_category_and_not_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let hash = "f".repeat(64);
        touch(
            &home.join(format!("history/{hash}/HEAD")),
            b"ref: refs/heads/main",
        );
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.category == AgentCategory::Checkpoints)
            .expect("history shadow repo identified");
        assert_eq!(u.action, AgentActionCapability::None);
        assert!(matches!(
            u.project_link,
            ProjectLinkState::Unresolved { .. }
        ));
    }

    #[test]
    fn identification_cost_is_bounded_for_many_project_hashes() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        for i in 0..300 {
            let hash = format!("{i:064}");
            touch(
                &home.join(format!("tmp/{hash}/chats/session.json")),
                &b"x".repeat(2_000),
            );
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!(
            "[measured] gemini_cli identify() over 300 synthetic project hashes took {elapsed:?}"
        );
        assert_eq!(
            units
                .iter()
                .filter(|u| u.category == AgentCategory::Sessions)
                .count(),
            300
        );
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
    }
}
