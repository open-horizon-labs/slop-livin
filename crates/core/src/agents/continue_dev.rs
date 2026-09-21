//! Continue identification (#99): sessions, generated indexes/caches,
//! anonymized usage-event logs and protected configuration under the
//! home `crate::locations::continue_dev::ContinueDetector` resolves.
//! Module named `continue_dev` -- `continue` is a Rust keyword.
//!
//! `config.yaml`/`config.json` are confirmed by primary docs (see
//! `crate::locations::continue_dev`'s doc comment); `sessions/`,
//! `index/` and `dev_data/` are this catalog's own prior research, not
//! independently re-confirmed this chunk, so this adapter treats their
//! *presence* as a version marker (same discipline every other adapter
//! in this catalog uses) rather than asserting their interior schema is
//! fully known. `sessions/`'s own documented shape ("sessions/
//! <session-id> plus a session index file, separate from the session
//! bodies") is honored by excluding common index-like filenames
//! (`sessions.json`/`index.json`) from per-session identification and
//! protecting them separately instead -- deleting the index alongside a
//! kept session would otherwise corrupt it for every session that
//! remains.

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs,
};
use std::fs;
use std::path::Path;

pub const CONTINUE_TOOL_ID: &str = crate::locations::continue_dev::CONTINUE_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;
const SESSION_INDEX_NAMES: &[&str] = &["sessions.json", "index.json", "sessions.json.bak"];
const FORMAT_MARKERS: &[&str] = &[
    "config.yaml",
    "config.json",
    "sessions",
    "index",
    "dev_data",
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
    identify_sessions(home, &mut units);
    identify_index(home, &mut units);
    identify_dev_data(home, &mut units);
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
            "no Continue content markers found (config.yaml/config.json/sessions/index/\
             dev_data) at this resolved path; this directory may belong to a different tool, be \
             empty, or use an unsupported version -- treated as unknown format, not scanned \
             further"
                .to_string(),
        ),
    }]
}

fn identify_protected(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    for rel in ["config.yaml", "config.json"] {
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
                protect_reason: Some("main configuration".to_string()),
                project_link: ProjectLinkState::NotApplicable,
                action: AgentActionCapability::None,
                note: None,
            });
        }
    }
}

fn identify_sessions(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let base = home.join("sessions");
    let Ok(rd) = fs::read_dir(&base) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if SESSION_INDEX_NAMES.contains(&name.as_str()) {
            continue; // handled by identify_index
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let (bytes, mtime, truncated) = if meta.is_dir() {
            folded_bytes(&path, MAX_FOLD_ENTRIES)
        } else {
            (meta.len(), mtime_secs(&meta), false)
        };
        let member_kind = if meta.is_dir() {
            AgentMemberKind::SessionData
        } else {
            AgentMemberKind::Transcript
        };
        out.push(CandidateAgentUnit {
            category: AgentCategory::Sessions,
            relative_path: format!("sessions/{name}"),
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
            // No declared project/workspace path is confirmed anywhere
            // in a Continue session body or filename this chunk could
            // verify without reading conversation content -- honestly
            // unresolved rather than guessed from the session id.
            project_link: ProjectLinkState::Unresolved {
                reason: "no confirmed workspace-linkage field for Continue sessions this chunk"
                    .to_string(),
            },
            action: AgentActionCapability::SessionRemoval,
            note: truncated.then(|| "directory entry count bound reached".to_string()),
        });
    }
}

fn identify_index(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let sessions = home.join("sessions");
    for name in SESSION_INDEX_NAMES {
        let path = sessions.join(name);
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_file()
        {
            out.push(CandidateAgentUnit {
                category: AgentCategory::ProtectedConfig,
                relative_path: format!("sessions/{name}"),
                path,
                members: Vec::new(),
                bytes: meta.len(),
                mtime_max: mtime_secs(&meta),
                protected: true,
                protect_reason: Some(
                    "session index, separate from the session bodies; removing it would corrupt \
                     lookups for every session that remains"
                        .to_string(),
                ),
                project_link: ProjectLinkState::NotApplicable,
                action: AgentActionCapability::None,
                note: None,
            });
        }
    }
    let index_dir = home.join("index");
    if index_dir.is_dir() {
        let (bytes, mtime, truncated) = folded_bytes(&index_dir, MAX_FOLD_ENTRIES);
        out.push(CandidateAgentUnit {
            category: AgentCategory::Caches,
            relative_path: "index".to_string(),
            path: index_dir,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::CacheOrLogTrash,
            note: Some(if truncated {
                "embeddings/tag caches, regenerated on next indexing pass; directory entry count \
                 bound reached"
                    .to_string()
            } else {
                "embeddings/tag caches, regenerated on next indexing pass".to_string()
            }),
        });
    }
}

fn identify_dev_data(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let path = home.join("dev_data");
    if !path.is_dir() {
        return;
    }
    let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
    out.push(CandidateAgentUnit {
        category: AgentCategory::Logs,
        relative_path: "dev_data".to_string(),
        path,
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: ProjectLinkState::NotApplicable,
        action: AgentActionCapability::CacheOrLogTrash,
        note: Some(if truncated {
            "anonymized development/usage event logs; directory entry count bound reached"
                .to_string()
        } else {
            "anonymized development/usage event logs".to_string()
        }),
    });
}

fn identify_residual(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let seen: std::collections::HashSet<&str> = [
        "config.yaml",
        "config.json",
        "config.ts",
        "sessions",
        "index",
        "dev_data",
    ]
    .into_iter()
    .collect();
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
    fn config_yaml_is_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("config.yaml"), b"models: []");
        let units = identify(home, 1);
        let u = units
            .iter()
            .find(|u| u.relative_path == "config.yaml")
            .unwrap();
        assert!(u.protected);
    }

    #[test]
    fn a_session_is_actionable_and_the_index_is_protected_separately() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let canary = "CANARY-CONTINUE-DO-NOT-LEAK-66gg";
        touch(
            &home.join("sessions/s1.json"),
            format!("{{\"content\":\"{canary}\"}}").as_bytes(),
        );
        touch(&home.join("sessions/sessions.json"), b"[{\"id\":\"s1\"}]");
        let units = identify(home, 1);
        let session = units
            .iter()
            .find(|u| u.relative_path == "sessions/s1.json")
            .expect("session identified");
        assert_eq!(session.action, AgentActionCapability::SessionRemoval);
        assert!(!session.protected);
        let index = units
            .iter()
            .find(|u| u.relative_path == "sessions/sessions.json")
            .expect("index identified");
        assert!(index.protected);
        assert_eq!(index.action, AgentActionCapability::None);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "content leaked");
    }

    #[test]
    fn index_dir_and_dev_data_are_actionable_caches_and_logs() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("index/embeddings.bin"), b"vector-bytes");
        touch(&home.join("dev_data/events.jsonl"), b"{}");
        let units = identify(home, 1);
        assert_eq!(
            units
                .iter()
                .find(|u| u.relative_path == "index")
                .unwrap()
                .action,
            AgentActionCapability::CacheOrLogTrash
        );
        assert_eq!(
            units
                .iter()
                .find(|u| u.relative_path == "dev_data")
                .unwrap()
                .action,
            AgentActionCapability::CacheOrLogTrash
        );
    }

    #[test]
    fn identification_cost_is_bounded_for_many_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        for i in 0..500 {
            touch(
                &home.join(format!("sessions/s{i}.json")),
                &b"x".repeat(50_000),
            );
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!(
            "[measured] continue_dev identify() over 500 synthetic sessions took {elapsed:?}"
        );
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
