//! OpenCode identification (#95): version-aware session/message storage
//! (SQLite-backed in current releases, a JSON file tree in older ones),
//! git-backed checkpoint snapshots, and protected configuration under
//! the **data root**
//! `crate::locations::opencode::OpenCodeDetector` resolves first (see
//! that module's doc comment for why config/cache are separate,
//! non-decomposed locations).
//!
//! Layout researched from primary source during implementation (never
//! from a real OpenCode data directory on this machine -- PRIVACY IS A
//! HARD RULE), from <https://github.com/sst/opencode>
//! (`packages/opencode/src/storage/storage.ts`, current `dev` branch)
//! and its own DeepWiki-indexed documentation as of this chunk:
//! - Older/file-tree layout: `storage/session/<project-id>/<session-id>
//!   .json`, `storage/message/<session-id>/*.json` ("stores messages by
//!   session ID" -- same session-id-keyed exact-match discipline
//!   `crate::agents::claude_code` uses for its own companion
//!   directories), `storage/part/<message-id>/*.json` ("stores message
//!   parts by message ID" -- keyed by *message*, not session, id: this
//!   adapter does not correlate parts to sessions without reading
//!   message file content, and says so, the same honest-gap discipline
//!   `claude_code::identify` uses for its own `paste-cache`),
//!   `storage/session_diff/<session-id>/` and `storage/project/
//!   <project-id>.json` (migration code: "the git repository's initial
//!   commit ID becomes the project identifier"; the file itself carries
//!   `id`, `vcs`, `worktree`, `time` -- `worktree` is a real filesystem
//!   path, read directly rather than needing any session-body scan for
//!   project linkage).
//! - Newer/SQLite layout: `opencode.db` (+ `-wal`/`-shm`) directly under
//!   the data root, replacing per-session file-tree storage for
//!   sessions/messages/history. This adapter never opens it; it is
//!   folded into one protected, non-actionable unit, same discipline
//!   `crate::agents::codex`'s own SQLite stores use.
//! - Both layouts: `snapshot/<project-id>/<hash>` -- an internal
//!   git-backed object store, decoupled from the project's own `.git`,
//!   capturing a tree snapshot before/after every agent step so `/undo`
//!   can revert. Unique checkpoint/recovery state, never a cache.
//! - `auth.json` and `log/` live directly under the data root (not under
//!   the separate config root), per
//!   <https://opencode.ai/docs/troubleshooting/>.
//!
//! Version-aware boundary (#95's explicit acceptance): `identify` checks
//! for `opencode.db`/`storage/`/`snapshot/`/`auth.json`/`log/` before
//! doing anything else. If a resolved data root exists but is non-empty
//! and matches none of them, this adapter reports one `Unclassified`,
//! non-actionable "unsupported layout version" unit rather than
//! guessing at either schema.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const OPENCODE_TOOL_ID: &str = crate::locations::opencode::OPENCODE_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.is_dir() {
        return Vec::new();
    }
    let has_db = home.join("opencode.db").is_file();
    let has_storage = home.join("storage").is_dir();
    let has_snapshot = home.join("snapshot").is_dir();
    let has_auth = home.join("auth.json").is_file();
    let has_log = home.join("log").is_dir();
    if !(has_db || has_storage || has_snapshot || has_auth || has_log) {
        return unknown_version_residual(home);
    }

    let mut units = Vec::new();
    if has_db {
        identify_sqlite_store(home, &mut units);
    }
    let projects = load_project_worktrees(home);
    let mut claimed_session_ids: HashSet<String> = HashSet::new();
    if has_storage {
        if !has_db {
            identify_file_tree_sessions(home, &projects, &mut claimed_session_ids, &mut units);
        }
        identify_storage_auxiliary(home, &claimed_session_ids, &mut units);
    }
    if has_snapshot {
        identify_snapshots(home, &projects, &mut units);
    }
    identify_static_categories(home, has_db, has_storage, has_snapshot, &mut units);
    units
}

fn unknown_version_residual(home: &Path) -> Vec<CandidateAgentUnit> {
    // See `oh_my_pi::unknown_format_residual`'s identical note: a
    // genuinely empty, existing directory is not "unsupported version",
    // and `folded_bytes`'s mtime is non-zero for an empty directory
    // itself, so entries are checked directly instead.
    let has_entries = fs::read_dir(home)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, _truncated) = folded_bytes(home, MAX_FOLD_ENTRIES);
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
            "no recognized OpenCode data-directory markers found (opencode.db/storage/snapshot/ \
             auth.json/log) at this resolved path; unsupported or future layout version -- \
             treated as unknown, not scanned further"
                .to_string(),
        ),
    }]
}

// ---------------------------------------------------------------------
// Project linkage: `storage/project/<project-id>.json`'s `worktree`
// field, read once per project directory and reused for every session
// under it -- declared metadata, no session-body scan needed at all.
// ---------------------------------------------------------------------

fn load_project_worktrees(home: &Path) -> HashMap<String, ProjectLinkState> {
    let mut map = HashMap::new();
    let dir = home.join("storage").join("project");
    let Ok(rd) = fs::read_dir(&dir) else {
        return map;
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(project_id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Small, bounded metadata file -- not conversation content.
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let worktree = value
            .get("worktree")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let link = resolve_declared_path(worktree, "no worktree field in project.json");
        map.insert(project_id.to_string(), link);
    }
    map
}

fn project_link_for(
    projects: &HashMap<String, ProjectLinkState>,
    project_id: &str,
) -> ProjectLinkState {
    projects
        .get(project_id)
        .cloned()
        .unwrap_or_else(|| ProjectLinkState::Unresolved {
            reason: format!("no storage/project/{project_id}.json found"),
        })
}

// ---------------------------------------------------------------------
// SQLite-backed layout (current releases).
// ---------------------------------------------------------------------

fn identify_sqlite_store(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let db = home.join("opencode.db");
    let Ok(meta) = fs::symlink_metadata(&db) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let mut members = vec![AgentMember {
        path: db.clone(),
        bytes: meta.len(),
        kind: AgentMemberKind::Database,
    }];
    let mut bytes = meta.len();
    let mut mtime_max = mtime_secs(&meta);
    for ext in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{ext}", db.display()));
        if let Ok(sm) = fs::symlink_metadata(&sidecar)
            && sm.is_file()
        {
            bytes += sm.len();
            mtime_max = mtime_max.max(mtime_secs(&sm));
            members.push(AgentMember {
                path: sidecar,
                bytes: sm.len(),
                kind: AgentMemberKind::Database,
            });
        }
    }
    out.push(CandidateAgentUnit {
        category: AgentCategory::Sessions,
        relative_path: "opencode.db".to_string(),
        path: db,
        members,
        bytes,
        mtime_max,
        protected: true,
        protect_reason: Some(
            "SQLite-backed session/message/history store (current OpenCode layout); \
             metadata-only, never opened while writable; no per-session drill-down in this \
             version boundary"
                .to_string(),
        ),
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: None,
    });
}

// ---------------------------------------------------------------------
// File-tree layout (older releases).
// ---------------------------------------------------------------------

fn identify_file_tree_sessions(
    home: &Path,
    projects: &HashMap<String, ProjectLinkState>,
    claimed: &mut HashSet<String>,
    out: &mut Vec<CandidateAgentUnit>,
) {
    let base = home.join("storage").join("session");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for project_entry in rd.flatten() {
        let Ok(ft) = project_entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let project_id = project_entry.file_name().to_string_lossy().into_owned();
        let project_link = project_link_for(projects, &project_id);
        let Ok(session_files) = fs::read_dir(project_entry.path()) else {
            continue;
        };
        for sf in session_files.flatten() {
            let path = sf.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(session_id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            let mut members = vec![AgentMember {
                path: path.clone(),
                bytes: meta.len(),
                kind: AgentMemberKind::Transcript,
            }];
            let mut bytes = meta.len();
            let mut mtime_max = mtime_secs(&meta);
            for (dir_name, kind) in [
                ("message", AgentMemberKind::SessionData),
                ("session_diff", AgentMemberKind::SessionData),
            ] {
                let companion = home.join("storage").join(dir_name).join(&session_id);
                if companion.is_dir() {
                    let (b, m, _t) = folded_bytes(&companion, MAX_FOLD_ENTRIES);
                    bytes += b;
                    mtime_max = mtime_max.max(m);
                    members.push(AgentMember {
                        path: companion,
                        bytes: b,
                        kind,
                    });
                }
            }
            claimed.insert(session_id);
            let relative_path = relative_to(home, &path);
            out.push(CandidateAgentUnit {
                category: AgentCategory::Sessions,
                relative_path,
                path,
                members,
                bytes,
                mtime_max,
                protected: false,
                protect_reason: None,
                project_link: project_link.clone(),
                action: AgentActionCapability::SessionRemoval,
                note: None,
            });
        }
    }
}

/// `storage/message/` and `storage/session_diff/` entries no session
/// claimed above (folded into one residual note each, never silently
/// dropped -- same discipline `claude_code::identify_session_keyed_top_level`
/// uses), plus `storage/part/`, which is *never* session-keyed (keyed by
/// message id -- see module doc comment) and is always folded whole.
fn identify_storage_auxiliary(
    home: &Path,
    claimed: &HashSet<String>,
    out: &mut Vec<CandidateAgentUnit>,
) {
    for dir_name in ["message", "session_diff"] {
        let base = home.join("storage").join(dir_name);
        let Ok(rd) = fs::read_dir(&base) else {
            continue;
        };
        let mut bytes = 0u64;
        let mut mtime_max = 0u64;
        let mut any = false;
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if claimed.contains(&name) {
                continue;
            }
            let (b, m, _t) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
            bytes += b;
            mtime_max = mtime_max.max(m);
            any = true;
        }
        if any {
            out.push(CandidateAgentUnit {
                category: AgentCategory::Unclassified,
                relative_path: format!("storage/{dir_name} (unlinked)"),
                path: base,
                members: Vec::new(),
                bytes,
                mtime_max,
                protected: false,
                protect_reason: None,
                project_link: ProjectLinkState::NotApplicable,
                action: AgentActionCapability::None,
                note: Some(format!(
                    "{dir_name} entries keyed by session id with no matching current session \
                     file in storage/session/ (already removed, or the current data root uses \
                     the SQLite-backed layout with no file-tree session to correlate against)"
                )),
            });
        }
    }
    let part = home.join("storage").join("part");
    if part.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&part, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Attachments,
            relative_path: "storage/part".to_string(),
            path: part,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: true,
            protect_reason: Some(
                "message parts, keyed by message id; no per-session reference evidence is \
                 available without reading message file content, so this adapter does not \
                 offer it as a supported action"
                    .to_string(),
            ),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: truncated.then(|| "directory entry count bound reached".to_string()),
        });
    }
}

// ---------------------------------------------------------------------
// Git-backed checkpoint snapshots (both layout versions).
// ---------------------------------------------------------------------

fn identify_snapshots(
    home: &Path,
    projects: &HashMap<String, ProjectLinkState>,
    out: &mut Vec<CandidateAgentUnit>,
) {
    let base = home.join("snapshot");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let project_id = e.file_name().to_string_lossy().into_owned();
        let (bytes, mtime, truncated) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
        let note = if truncated {
            "git-backed checkpoint history for this project's /undo; removing it loses the \
             ability to revert past this point (directory entry count bound reached) -- not a \
             supported selective action in this chunk"
        } else {
            "git-backed checkpoint history for this project's /undo; removing it loses the \
             ability to revert past this point -- not a supported selective action in this \
             chunk"
        };
        out.push(CandidateAgentUnit {
            category: AgentCategory::Checkpoints,
            relative_path: relative_to(home, &e.path()),
            path: e.path(),
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: project_link_for(projects, &project_id),
            action: AgentActionCapability::None,
            note: Some(note.to_string()),
        });
    }
}

// ---------------------------------------------------------------------
// Static top-level entries.
// ---------------------------------------------------------------------

fn identify_static_categories(
    home: &Path,
    has_db: bool,
    has_storage: bool,
    has_snapshot: bool,
    out: &mut Vec<CandidateAgentUnit>,
) {
    let auth = home.join("auth.json");
    if let Ok(meta) = fs::symlink_metadata(&auth)
        && meta.is_file()
    {
        out.push(CandidateAgentUnit {
            category: AgentCategory::ProtectedConfig,
            relative_path: "auth.json".to_string(),
            path: auth,
            members: Vec::new(),
            bytes: meta.len(),
            mtime_max: mtime_secs(&meta),
            protected: true,
            protect_reason: Some(
                "authentication data (API keys, OAuth tokens); contents are never read by this \
                 adapter"
                    .to_string(),
            ),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: None,
        });
    }

    let log = home.join("log");
    if log.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&log, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Logs,
            relative_path: "log".to_string(),
            path: log,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: truncated.then(|| "directory entry count bound reached".to_string()),
        });
    }

    let mut seen_top_level: HashSet<&str> = HashSet::from(["auth.json", "log"]);
    if has_db {
        seen_top_level.extend(["opencode.db", "opencode.db-wal", "opencode.db-shm"]);
    }
    if has_storage {
        seen_top_level.insert("storage");
    }
    if has_snapshot {
        seen_top_level.insert("snapshot");
    }
    let Ok(rd) = fs::read_dir(home) else { return };
    let mut residual_bytes = 0u64;
    let mut residual_mtime = 0u64;
    let mut residual_names: Vec<String> = Vec::new();
    for e in rd.flatten() {
        let Some(name) = e
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if seen_top_level.contains(name.as_str()) {
            continue;
        }
        let (bytes, mtime, _t) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
        residual_bytes += bytes;
        residual_mtime = residual_mtime.max(mtime);
        residual_names.push(name);
    }
    if !residual_names.is_empty() {
        residual_names.sort();
        out.push(CandidateAgentUnit {
            category: AgentCategory::Unclassified,
            relative_path: "(unclassified residual)".to_string(),
            path: home.to_path_buf(),
            members: Vec::new(),
            bytes: residual_bytes,
            mtime_max: residual_mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: Some(format!(
                "entries with no specific rule in this adapter: {}",
                residual_names.join(", ")
            )),
        });
    }
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

    fn project_json(worktree: &str) -> String {
        format!("{{\"id\":\"p1\",\"vcs\":\"git\",\"worktree\":\"{worktree}\"}}")
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
    fn file_tree_session_is_identified_and_linked_via_project_json() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        touch(
            &home.join("storage/project/p1.json"),
            project_json(&repo.display().to_string()).as_bytes(),
        );
        let canary = "CANARY-OC-DO-NOT-LEAK-44cc";
        touch(
            &home.join("storage/session/p1/s1.json"),
            format!("{{\"id\":\"s1\",\"title\":\"{canary}\"}}").as_bytes(),
        );
        touch(
            &home.join("storage/message/s1/m1.json"),
            format!("{{\"content\":\"{canary}\"}}").as_bytes(),
        );
        let units = identify(home, 1);
        let session = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions)
            .expect("session identified");
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Linked { .. }
        ));
        assert_eq!(session.action, AgentActionCapability::SessionRemoval);
        assert_eq!(session.members.len(), 2, "transcript + message companion");
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "content leaked");
    }

    #[test]
    fn sqlite_layout_is_protected_and_skips_file_tree_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("opencode.db"), b"sqlite-bytes");
        touch(&home.join("opencode.db-wal"), b"wal-bytes");
        // Legacy file-tree debris that must NOT be double-counted as a
        // live session once the DB-backed layout is in play.
        touch(&home.join("storage/session/p1/s1.json"), b"{}");
        let units = identify(home, 1);
        let db = units
            .iter()
            .find(|u| u.relative_path == "opencode.db")
            .expect("db unit present");
        assert!(db.protected);
        assert_eq!(db.action, AgentActionCapability::None);
        assert_eq!(db.members.len(), 2);
        assert!(
            units
                .iter()
                .all(|u| u.category != AgentCategory::Sessions || u.relative_path == "opencode.db"),
            "no separate file-tree session unit once DB-backed"
        );
    }

    #[test]
    fn part_directory_is_protected_pending_session_correlation() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("storage/part/msg1/p1.json"), b"{}");
        let units = identify(home, 1);
        let part = units
            .iter()
            .find(|u| u.relative_path == "storage/part")
            .unwrap();
        assert!(part.protected);
        assert_eq!(part.action, AgentActionCapability::None);
    }

    #[test]
    fn snapshot_is_checkpoint_category_and_not_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        touch(
            &home.join("storage/project/p1.json"),
            project_json(&repo.display().to_string()).as_bytes(),
        );
        touch(&home.join("snapshot/p1/abcd1234"), b"git-object-bytes");
        let units = identify(home, 1);
        let snap = units
            .iter()
            .find(|u| u.category == AgentCategory::Checkpoints)
            .expect("snapshot identified");
        assert_eq!(snap.action, AgentActionCapability::None);
        assert!(matches!(snap.project_link, ProjectLinkState::Linked { .. }));
    }

    #[test]
    fn auth_json_is_protected_by_default() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("auth.json"), b"[redacted]");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "auth.json")
            .unwrap();
        assert!(u.protected);
        assert_eq!(u.action, AgentActionCapability::None);
    }

    #[test]
    fn log_directory_is_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("log").join("app.log"), b"debug line");
        let units = identify(home, 1);
        let u = units.iter().find(|u| u.relative_path == "log").unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn identification_cost_is_bounded_for_many_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("big-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        touch(
            &home.join("storage/project/p1.json"),
            project_json(&repo.display().to_string()).as_bytes(),
        );
        for i in 0..500 {
            let mut content =
                format!("{{\"id\":\"s{i}\",\"body\":\"unread-canary\"}}").into_bytes();
            content.extend_from_slice(&b"x".repeat(200_000));
            touch(
                &home.join(format!("storage/session/p1/s{i}.json")),
                &content,
            );
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!("[measured] opencode identify() over 500 synthetic sessions took {elapsed:?}");
        assert_eq!(
            units
                .iter()
                .filter(|u| u.category == AgentCategory::Sessions)
                .count(),
            500
        );
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
    }
}
