//! GitHub Copilot CLI identification (#97): session/workspace artifacts,
//! command-recall history, a cross-session SQLite store, logs and
//! protected configuration under the home
//! `crate::locations::copilot_cli::CopilotCliDetector` resolves first.
//!
//! Layout sourced directly from GitHub's own reference page (fetched
//! during implementation, never from a real `~/.copilot` on this machine
//! -- PRIVACY IS A HARD RULE):
//! <https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference>,
//! current as of this chunk -- see `crate::locations::copilot_cli`'s doc
//! comment for the full item-by-item citation, including the correction
//! of #97's own issue-text guess (`history-session-state/`) to the real
//! `session-state/`/`command-history-state/` names.
//!
//! `session-state/`'s own interior schema (what exactly each session
//! artifact directory contains) is not documented on that reference page
//! beyond "session history and workspace artifacts"; this adapter folds
//! each immediate child of `session-state/` into one `Sessions` unit and
//! attempts a bounded, small-JSON-file scan for a `cwd`/`workspace`
//! field for project linkage, honestly reporting `Unresolved` when
//! nothing is found rather than guessing a schema this chunk could not
//! confirm.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::fs;
use std::path::{Path, PathBuf};

pub const COPILOT_CLI_TOOL_ID: &str = crate::locations::copilot_cli::COPILOT_CLI_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;
/// Bound on how many bytes of any one metadata file this adapter reads
/// looking for a `cwd`/`workspace` field -- session/task content itself
/// is never read (guardrail: no prompt/transcript content in output).
const MAX_METADATA_SCAN_BYTES: u64 = 65_536;

const PROTECTED_CONFIG_FILES: &[(&str, &str)] = &[
    (
        "config.json",
        "automatically managed application state (authentication, plugins)",
    ),
    ("settings.json", "primary user configuration"),
    ("mcp-config.json", "user-level MCP server definitions"),
    (
        "lsp-config.json",
        "user-level Language Server Protocol definitions",
    ),
    (
        "permissions-config.json",
        "saved tool and directory permissions per project",
    ),
    (
        "providers.json",
        "bring-your-own-key provider/model registry",
    ),
    (
        "copilot-instructions.md",
        "personal instructions applied across all sessions",
    ),
];
const PROTECTED_CONFIG_DIRS: &[(&str, &str)] = &[
    ("instructions", "additional cross-session instructions"),
    ("agents", "custom agent definitions"),
    ("hooks", "user-level hook scripts"),
    ("skills", "personal custom skill definitions"),
    ("extensions", "user-level extension files"),
    ("installed-plugins", "installed plugin files"),
    ("plugin-data", "persistent plugin data"),
    (
        "mcp-oauth-config",
        "MCP OAuth tokens and registration fallback storage",
    ),
    ("mcp-secrets", "MCP secret placeholders and index"),
];
const FORMAT_MARKERS: &[&str] = &[
    "settings.json",
    "config.json",
    "session-state",
    "session-store.db",
    "logs",
];

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.is_dir() {
        return Vec::new();
    }
    if !FORMAT_MARKERS.iter().any(|rel| home.join(rel).exists()) {
        return unknown_format_residual(home);
    }
    let mut units = Vec::new();
    identify_protected(home, &mut units);
    identify_session_state(home, &mut units);
    identify_command_history(home, &mut units);
    identify_session_store(home, &mut units);
    identify_logs(home, &mut units);
    identify_ide_state(home, &mut units);
    identify_residual(home, &mut units);
    units
}

fn unknown_format_residual(home: &Path) -> Vec<CandidateAgentUnit> {
    let has_entries = fs::read_dir(home)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, _t) = folded_bytes(home, MAX_FOLD_ENTRIES);
    vec![CandidateAgentUnit {
        category: AgentCategory::Unclassified,
        relative_path: "(unknown format)".to_string(),
        path: home.to_path_buf(),
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: Some(
            "no GitHub Copilot CLI content markers found (settings.json/config.json/\
             session-state/session-store.db/logs) at this resolved path; this directory may \
             belong to a different tool, be empty, or use an unsupported version -- treated as \
             unknown format, not scanned further"
                .to_string(),
        ),
    }]
}

fn identify_protected(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    for (rel, note) in PROTECTED_CONFIG_FILES {
        let path = home.join(rel);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        out.push(protected_unit(
            rel,
            path,
            meta.len(),
            mtime_secs(&meta),
            note,
        ));
    }
    for (rel, note) in PROTECTED_CONFIG_DIRS {
        let path = home.join(rel);
        if !path.is_dir() {
            continue;
        }
        let (bytes, mtime, _t) = folded_bytes(&path, MAX_FOLD_ENTRIES);
        out.push(protected_unit(rel, path, bytes, mtime, note));
    }
}

fn protected_unit(
    rel: &str,
    path: PathBuf,
    bytes: u64,
    mtime: u64,
    note: &str,
) -> CandidateAgentUnit {
    CandidateAgentUnit {
        category: AgentCategory::ProtectedConfig,
        relative_path: rel.to_string(),
        path,
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: true,
        protect_reason: Some(note.to_string()),
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: None,
    }
}

/// Bounded small-JSON scan of `dir` (not recursive beyond one level) for
/// a `cwd`/`workspace`/`workspaceFolder` string field. Never reads a
/// file larger than `MAX_METADATA_SCAN_BYTES`, and never returns
/// anything but the one field value -- no content is retained.
fn declared_path_in_dir(dir: &Path) -> Option<String> {
    let rd = fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > MAX_METADATA_SCAN_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for field in ["cwd", "workspace", "workspaceFolder"] {
            if let Some(s) = value.get(field).and_then(|v| v.as_str())
                && !s.is_empty()
            {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn identify_session_state(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let base = home.join("session-state");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let (bytes, mtime, truncated, declared) = if meta.is_dir() {
            let (b, m, t) = folded_bytes(&path, MAX_FOLD_ENTRIES);
            (b, m, t, declared_path_in_dir(&path))
        } else {
            (meta.len(), mtime_secs(&meta), false, None)
        };
        let project_link = resolve_declared_path(
            declared,
            "no cwd/workspace field found in this session artifact's own small metadata files",
        );
        let member_kind = if meta.is_dir() {
            AgentMemberKind::SessionData
        } else {
            AgentMemberKind::Transcript
        };
        out.push(CandidateAgentUnit {
            category: AgentCategory::Sessions,
            relative_path: relative_to(home, &path),
            path: path.clone(),
            members: vec![AgentMember {
                path,
                bytes,
                kind: member_kind,
            }],
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link,
            action: AgentActionCapability::SessionRemoval,
            note: truncated.then(|| "directory entry count bound reached".to_string()),
        });
    }
}

fn identify_command_history(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let path = home.join("command-history-state");
    if !path.is_dir() {
        return;
    }
    let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
    out.push(CandidateAgentUnit {
        category: AgentCategory::Caches,
        relative_path: "command-history-state".to_string(),
        path,
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::CacheOrLogTrash,
        note: Some(if truncated {
            "reverse-search command recall history; directory entry count bound reached".to_string()
        } else {
            "reverse-search command recall history, not conversation content".to_string()
        }),
    });
}

fn identify_session_store(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let db = home.join("session-store.db");
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
        relative_path: "session-store.db".to_string(),
        path: db,
        members,
        bytes,
        mtime_max,
        protected: true,
        protect_reason: Some(
            "SQLite database for cross-session data; metadata-only, never opened while writable"
                .to_string(),
        ),
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: None,
    });
}

fn identify_logs(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let path = home.join("logs");
    if !path.is_dir() {
        return;
    }
    let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
    out.push(CandidateAgentUnit {
        category: AgentCategory::Logs,
        relative_path: "logs".to_string(),
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

/// `ide/` (IDE integration state and lock files) is deliberately not
/// actionable this chunk: a lock file backing an active IDE integration
/// is a real corruption risk if moved out from under it, and this
/// adapter has no documented way to tell which entries are safely idle.
fn identify_ide_state(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let path = home.join("ide");
    if !path.is_dir() {
        return;
    }
    let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
    out.push(CandidateAgentUnit {
        category: AgentCategory::Unclassified,
        relative_path: "ide".to_string(),
        path,
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::None,
        note: Some(format!(
            "IDE integration state and lock files; not offered as a supported action this \
                 chunk -- a lock file backing an active integration could be corrupted by \
                 removal, and this adapter has no documented way to tell which entries are idle{}",
            if truncated {
                " (directory entry count bound reached)"
            } else {
                ""
            }
        )),
    });
}

fn identify_residual(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (rel, _) in PROTECTED_CONFIG_FILES {
        seen.insert(rel);
    }
    for (rel, _) in PROTECTED_CONFIG_DIRS {
        seen.insert(rel);
    }
    for rel in [
        "session-state",
        "command-history-state",
        "session-store.db",
        "session-store.db-wal",
        "session-store.db-shm",
        "logs",
        "ide",
    ] {
        seen.insert(rel);
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
        if seen.contains(name.as_str()) {
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

    #[test]
    fn empty_home_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identify(dir.path(), 1).is_empty());
    }

    #[test]
    fn no_markers_yields_unknown_format_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("unrelated.txt"), b"hello");
        let units = identify(dir.path(), 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].relative_path, "(unknown format)");
    }

    #[test]
    fn config_json_is_protected_by_default() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("config.json"), b"{}");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "config.json")
            .unwrap();
        assert!(u.protected);
        assert_eq!(u.action, AgentActionCapability::None);
    }

    #[test]
    fn mcp_secrets_dir_is_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(&home.join("mcp-secrets").join("index.json"), b"{}");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "mcp-secrets")
            .unwrap();
        assert!(u.protected);
    }

    #[test]
    fn session_state_links_via_bounded_metadata_scan() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let canary = "CANARY-COPILOT-DO-NOT-LEAK-99aa";
        touch(
            &home.join("session-state/s1/meta.json"),
            format!("{{\"cwd\":\"{}\",\"note\":\"{canary}\"}}", repo.display()).as_bytes(),
        );
        let units = identify(home, 1);
        let s = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions && u.relative_path.contains("s1"))
            .expect("session unit present");
        assert!(matches!(s.project_link, ProjectLinkState::Linked { .. }));
        assert_eq!(s.action, AgentActionCapability::SessionRemoval);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "content leaked");
    }

    #[test]
    fn session_state_without_metadata_is_unresolved_not_guessed() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(&home.join("session-state/s2/blob.bin"), b"opaque");
        let units = identify(home, 1);
        let s = units
            .iter()
            .find(|u| u.relative_path.contains("s2"))
            .unwrap();
        assert!(matches!(
            s.project_link,
            ProjectLinkState::Unresolved { .. }
        ));
    }

    #[test]
    fn command_history_state_is_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(
            &home.join("command-history-state/recall.log"),
            b"cmd1\ncmd2",
        );
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "command-history-state")
            .unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn session_store_db_is_protected_and_folds_sidecars() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("session-store.db"), b"sqlite");
        touch(&home.join("session-store.db-wal"), b"wal");
        let units = identify(home, 1);
        let db = units
            .iter()
            .find(|u| u.relative_path == "session-store.db")
            .unwrap();
        assert!(db.protected);
        assert_eq!(db.action, AgentActionCapability::None);
        assert_eq!(db.members.len(), 2);
    }

    #[test]
    fn ide_state_is_unclassified_and_never_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        touch(&home.join("ide/lock.json"), b"{}");
        let units = identify(home, 1);
        let u = units.iter().find(|u| u.relative_path == "ide").unwrap();
        assert_eq!(u.action, AgentActionCapability::None);
    }

    #[test]
    fn logs_are_actionable() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("logs/session.log"), b"debug");
        let units = identify(home, 1);
        let u = units.iter().find(|u| u.relative_path == "logs").unwrap();
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn identification_cost_is_bounded_for_many_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("settings.json"), b"{}");
        for i in 0..500 {
            touch(
                &home.join(format!("session-state/s{i}/blob.bin")),
                &b"x".repeat(200_000),
            );
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!("[measured] copilot_cli identify() over 500 synthetic sessions took {elapsed:?}");
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
