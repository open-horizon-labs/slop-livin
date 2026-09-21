//! Shared identification for the VS-Code-family editors/extensions in
//! this catalog: **forks** that keep their own editor profile
//! (`crate::agents::cursor`, `crate::agents::windsurf` -- a `User/`
//! directory with `globalStorage`/`workspaceStorage`/`History`), and
//! **extensions** installed into one or more of those hosts plus stock
//! VS Code (`crate::agents::cline`, `crate::agents::roo_code` -- a
//! `globalStorage/<extension-id>/` directory holding `tasks/<id>/`).
//! Both shapes share the same underlying VS Code storage conventions
//! (`state.vscdb` SQLite key-value stores, `workspace.json` folder-URI
//! linkage), so this module holds the one real implementation; each
//! tool-specific adapter is a thin, honestly-labelled wrapper -- per the
//! brief's "extend the shared model only where a tool genuinely needs a
//! new concept".
//!
//! Sourced from community reverse-engineering during implementation
//! (never a real editor profile on this machine -- PRIVACY IS A HARD
//! RULE); see `crate::locations::cursor`'s doc comment for the primary
//! citations (cursaves, cursor-chat-browser) this module's `state.vscdb`
//! shape is based on, and `crate::locations::cline`/`roo_code` for the
//! `tasks/<id>/` shape's citations.
//!
//! `state.vscdb`(+`-wal`/`-shm`) is **never** opened by this module --
//! folded as one protected `Database`-kind unit, unconditionally, same
//! discipline `crate::agents::opencode`'s `opencode.db` uses. Guardrail:
//! "WAL/SHM never removed" -- `is_sqlite_like` in `crate::actions`
//! refuses any selective action on a path with this kind regardless.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FOLD_ENTRIES: usize = 200_000;
/// Bound on a task's own small metadata file -- never its conversation
/// content (guardrail: no prompt/transcript content in output).
const MAX_METADATA_SCAN_BYTES: u64 = 65_536;

/// A `state.vscdb` (+ `-wal`/`-shm`) file at `path`, folded into one
/// protected, non-actionable `Database`-kind unit. `category` lets a
/// caller distinguish the global database from a per-workspace one in
/// its own labelling even though both are handled identically here.
fn vscdb_unit(
    path: PathBuf,
    category: AgentCategory,
    relative_path: String,
    project_link: ProjectLinkState,
    note: &str,
) -> Option<CandidateAgentUnit> {
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut members = vec![AgentMember {
        path: path.clone(),
        bytes: meta.len(),
        kind: AgentMemberKind::Database,
    }];
    let mut bytes = meta.len();
    let mut mtime_max = mtime_secs(&meta);
    for ext in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{ext}", path.display()));
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
    Some(CandidateAgentUnit {
        category,
        relative_path,
        path,
        members,
        bytes,
        mtime_max,
        protected: true,
        protect_reason: Some(
            "SQLite chat/composer state (WAL mode); metadata-only, never opened while writable, \
             WAL/SHM sidecars never removed individually"
                .to_string(),
        ),
        project_link,
        action: AgentActionCapability::None,
        note: Some(note.to_string()),
    })
}

// ---------------------------------------------------------------------
// Editor-profile shape (Cursor, Windsurf): a `User/` directory with
// globalStorage/workspaceStorage/History, plus Electron-conventional
// cache/log siblings.
// ---------------------------------------------------------------------

const PROFILE_MARKERS: &[&str] = &["User"];

pub fn identify_editor_profile(profile_root: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !profile_root.is_dir() {
        return Vec::new();
    }
    let user = profile_root.join("User");
    if !PROFILE_MARKERS
        .iter()
        .any(|rel| profile_root.join(rel).exists())
        || !user.is_dir()
    {
        return unknown_layout_residual(profile_root, "no User/ directory found");
    }
    let mut units = Vec::new();

    if let Some(u) = vscdb_unit(
        user.join("globalStorage").join("state.vscdb"),
        AgentCategory::Sessions,
        "User/globalStorage/state.vscdb".to_string(),
        ProjectLinkState::NotApplicable,
        "shared chat/composer content for every project this editor has opened",
    ) {
        units.push(u);
    }

    identify_workspace_storage(&user, &mut units);

    let history = user.join("History");
    if history.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&history, MAX_FOLD_ENTRIES);
        units.push(CandidateAgentUnit {
            category: AgentCategory::Caches,
            relative_path: "User/History".to_string(),
            path: history,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "local file-history/undo snapshots, unrelated to AI chat content; directory \
                 entry count bound reached"
                    .to_string()
            } else {
                "local file-history/undo snapshots, unrelated to AI chat content".to_string()
            }),
        });
    }

    for (rel, category) in [
        ("Cache", AgentCategory::Caches),
        ("CachedData", AgentCategory::Caches),
        ("CachedExtensionVSIXs", AgentCategory::Caches),
        ("logs", AgentCategory::Logs),
    ] {
        let path = profile_root.join(rel);
        if !path.is_dir() {
            continue;
        }
        let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
        units.push(CandidateAgentUnit {
            category,
            relative_path: rel.to_string(),
            path,
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
    units
}

fn identify_workspace_storage(user: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let base = user.join("workspaceStorage");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let ws_dir = entry.path();
        let project_link = workspace_json_link(&ws_dir);
        let relative_path = format!(
            "User/workspaceStorage/{}",
            entry.file_name().to_string_lossy()
        );
        if let Some(u) = vscdb_unit(
            ws_dir.join("state.vscdb"),
            AgentCategory::Sessions,
            format!("{relative_path}/state.vscdb"),
            project_link,
            "per-workspace chat/composer index; the conversation content itself lives in the \
             shared global database",
        ) {
            out.push(u);
        }
    }
}

fn workspace_json_link(ws_dir: &Path) -> ProjectLinkState {
    let path = ws_dir.join("workspace.json");
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return ProjectLinkState::Unresolved {
            reason: "no workspace.json found in this workspaceStorage directory".to_string(),
        };
    };
    if !meta.is_file() || meta.len() > MAX_METADATA_SCAN_BYTES {
        return ProjectLinkState::Unresolved {
            reason: "workspace.json missing or exceeded the bounded read size".to_string(),
        };
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return ProjectLinkState::Unresolved {
            reason: "workspace.json could not be read".to_string(),
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return ProjectLinkState::Unresolved {
            reason: "workspace.json was not valid JSON".to_string(),
        };
    };
    let folder = value.get("folder").and_then(|v| v.as_str());
    let Some(uri) = folder else {
        return ProjectLinkState::Unresolved {
            reason: "no folder field found in workspace.json".to_string(),
        };
    };
    match uri.strip_prefix("file://") {
        Some(p) => resolve_declared_path(Some(p.to_string()), ""),
        None => ProjectLinkState::Unresolved {
            reason: format!("workspace.json folder field is not a file:// URI: {uri}"),
        },
    }
}

// ---------------------------------------------------------------------
// VS Code extension shape (Cline, Roo Code): globalStorage/<ext-id>/
// with tasks/<task-id>/.
// ---------------------------------------------------------------------

/// Directory-name fragments this catalog recognizes as a VS-Code-family
/// host, checked against `ext_home`'s ancestors so every unit this
/// adapter identifies can be labelled with which host it came from --
/// #99's explicit "model each host as a separate detector location"
/// acceptance extends naturally to display, not just discovery.
const KNOWN_HOSTS: &[&str] = &[
    "Code - Insiders",
    "Code",
    "Cursor",
    "Windsurf",
    ".vscode-server",
];

pub fn host_label(ext_home: &Path) -> &'static str {
    for ancestor in ext_home.ancestors() {
        let Some(name) = ancestor.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        for host in KNOWN_HOSTS {
            if name == *host {
                return match *host {
                    ".vscode-server" => "VS Code Server (remote)",
                    "Code - Insiders" => "VS Code Insiders",
                    "Code" => "VS Code",
                    other => other,
                };
            }
        }
    }
    "unknown host"
}

pub fn identify_extension_globalstorage(
    ext_home: &Path,
    _observed_at: u64,
) -> Vec<CandidateAgentUnit> {
    if !ext_home.is_dir() {
        return Vec::new();
    }
    let host = host_label(ext_home);
    let tasks = ext_home.join("tasks");
    if !tasks.is_dir() {
        return unknown_layout_residual(ext_home, "no tasks/ directory found")
            .into_iter()
            .map(|mut u| {
                u.relative_path = format!("{host}/{}", u.relative_path);
                u
            })
            .collect();
    }
    let mut units = Vec::new();
    let Ok(rd) = fs::read_dir(&tasks) else {
        return units;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let task_dir = entry.path();
        let (bytes, mtime, truncated) = folded_bytes(&task_dir, MAX_FOLD_ENTRIES);
        let project_link = task_metadata_link(&task_dir);
        units.push(CandidateAgentUnit {
            category: AgentCategory::Sessions,
            relative_path: format!("{host}/tasks/{}", entry.file_name().to_string_lossy()),
            path: task_dir.clone(),
            members: vec![AgentMember {
                path: task_dir,
                bytes,
                kind: AgentMemberKind::SessionData,
            }],
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link,
            action: AgentActionCapability::SessionRemoval,
            note: Some(if truncated {
                "conversation history, UI messages and any checkpoint snapshots for this task; \
                 directory entry count bound reached"
                    .to_string()
            } else {
                "conversation history, UI messages and any checkpoint snapshots for this task"
                    .to_string()
            }),
        });
    }
    units
}

/// `task_metadata.json`'s `workspace` field, per the issue text's own
/// research (not independently re-confirmed against a schema doc this
/// chunk -- see `crate::locations::cline`'s doc comment). A missing
/// file or field degrades to `Unresolved`, never a guess.
fn task_metadata_link(task_dir: &Path) -> ProjectLinkState {
    let path = task_dir.join("task_metadata.json");
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return ProjectLinkState::Unresolved {
            reason: "no task_metadata.json found in this task directory".to_string(),
        };
    };
    if !meta.is_file() || meta.len() > MAX_METADATA_SCAN_BYTES {
        return ProjectLinkState::Unresolved {
            reason: "task_metadata.json missing or exceeded the bounded read size".to_string(),
        };
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return ProjectLinkState::Unresolved {
            reason: "task_metadata.json could not be read".to_string(),
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return ProjectLinkState::Unresolved {
            reason: "task_metadata.json was not valid JSON".to_string(),
        };
    };
    let workspace = value
        .get("workspace")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    resolve_declared_path(
        workspace,
        "no workspace field found in task_metadata.json (field name per this chunk's own \
         research, not independently re-confirmed against a schema doc)",
    )
}

fn unknown_layout_residual(root: &Path, why: &str) -> Vec<CandidateAgentUnit> {
    let has_entries = fs::read_dir(root)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, _t) = folded_bytes(root, MAX_FOLD_ENTRIES);
    vec![CandidateAgentUnit {
        category: AgentCategory::Unclassified,
        relative_path: "(unsupported layout version)".to_string(),
        path: root.to_path_buf(),
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: Some(format!(
            "{why}; unsupported or future layout version, treated as unknown, not scanned \
             further"
        )),
    }]
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
    fn empty_profile_root_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identify_editor_profile(dir.path(), 1).is_empty());
    }

    #[test]
    fn no_user_dir_yields_unsupported_layout_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("unrelated.txt"), b"hello");
        let units = identify_editor_profile(dir.path(), 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].relative_path, "(unsupported layout version)");
    }

    #[test]
    fn global_vscdb_is_protected_and_folds_sidecars() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        touch(&root.join("User/globalStorage/state.vscdb"), b"sqlite");
        touch(&root.join("User/globalStorage/state.vscdb-wal"), b"wal");
        let units = identify_editor_profile(root, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "User/globalStorage/state.vscdb")
            .expect("global db identified");
        assert!(u.protected);
        assert_eq!(u.action, AgentActionCapability::None);
        assert_eq!(u.members.len(), 2);
    }

    #[test]
    fn workspace_storage_links_via_workspace_json() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let repo = root.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        touch(
            &root.join("User/workspaceStorage/abc123/workspace.json"),
            format!("{{\"folder\":\"file://{}\"}}", repo.display()).as_bytes(),
        );
        touch(
            &root.join("User/workspaceStorage/abc123/state.vscdb"),
            b"sqlite",
        );
        let units = identify_editor_profile(root, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path.contains("workspaceStorage"))
            .expect("workspace db identified");
        assert!(u.protected);
        assert!(matches!(u.project_link, ProjectLinkState::Linked { .. }));
    }

    #[test]
    fn history_and_caches_are_actionable() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        touch(&root.join("User/globalStorage/state.vscdb"), b"sqlite");
        touch(&root.join("User/History/entry1/snapshot"), b"bytes");
        touch(&root.join("CachedExtensionVSIXs/ext.vsix"), b"bytes");
        let units = identify_editor_profile(root, 1);
        let history = units
            .iter()
            .find(|u| u.relative_path == "User/History")
            .unwrap();
        assert_eq!(history.action, AgentActionCapability::CacheOrLogTrash);
        let cache = units
            .iter()
            .find(|u| u.relative_path == "CachedExtensionVSIXs")
            .unwrap();
        assert_eq!(cache.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn host_label_recognizes_every_known_host() {
        assert_eq!(
            host_label(Path::new(
                "/Users/dev/Library/Application Support/Code/User/globalStorage/ext"
            )),
            "VS Code"
        );
        assert_eq!(
            host_label(Path::new(
                "/Users/dev/Library/Application Support/Cursor/User/globalStorage/ext"
            )),
            "Cursor"
        );
        assert_eq!(
            host_label(Path::new(
                "/Users/dev/.vscode-server/data/User/globalStorage/ext"
            )),
            "VS Code Server (remote)"
        );
    }

    #[test]
    fn extension_no_tasks_dir_yields_unsupported_layout() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = dir
            .path()
            .join("Library/Application Support/Code/User/globalStorage/some.ext");
        touch(&ext_home.join("unrelated.json"), b"{}");
        let units = identify_extension_globalstorage(&ext_home, 1);
        assert_eq!(units.len(), 1);
        assert!(
            units[0]
                .relative_path
                .ends_with("(unsupported layout version)")
        );
    }

    #[test]
    fn task_links_via_metadata_workspace_field() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = dir
            .path()
            .join("Library/Application Support/Cursor/User/globalStorage/some.ext");
        let repo = dir.path().join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let canary = "CANARY-VSCODE-DO-NOT-LEAK-88ee";
        touch(
            &ext_home.join("tasks/t1/task_metadata.json"),
            format!("{{\"workspace\":\"{}\"}}", repo.display()).as_bytes(),
        );
        touch(
            &ext_home.join("tasks/t1/api_conversation_history.json"),
            format!("[{{\"content\":\"{canary}\"}}]").as_bytes(),
        );
        let units = identify_extension_globalstorage(&ext_home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path.contains("t1"))
            .unwrap();
        assert!(u.relative_path.starts_with("Cursor/tasks/"));
        assert!(matches!(u.project_link, ProjectLinkState::Linked { .. }));
        assert_eq!(u.action, AgentActionCapability::SessionRemoval);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "content leaked");
    }

    #[test]
    fn task_without_metadata_is_unresolved_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = dir
            .path()
            .join("Library/Application Support/Code/User/globalStorage/some.ext");
        touch(&ext_home.join("tasks/t2/ui_messages.json"), b"[]");
        let units = identify_extension_globalstorage(&ext_home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path.contains("t2"))
            .unwrap();
        assert!(matches!(
            u.project_link,
            ProjectLinkState::Unresolved { .. }
        ));
    }

    #[test]
    fn identification_cost_is_bounded_for_many_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = dir
            .path()
            .join("Library/Application Support/Code/User/globalStorage/some.ext");
        for i in 0..500 {
            touch(
                &ext_home.join(format!("tasks/t{i}/api_conversation_history.json")),
                &b"x".repeat(50_000),
            );
        }
        let start = SystemTime::now();
        let units = identify_extension_globalstorage(&ext_home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!(
            "[measured] vscode_family identify_extension_globalstorage() over 500 synthetic \
             tasks took {elapsed:?}"
        );
        assert_eq!(units.len(), 500);
        assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
    }
}
