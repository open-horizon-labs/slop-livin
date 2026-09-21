//! Codex CLI identification (#93): rollout sessions (live + archived),
//! SQLite-backed state stores, and protected configuration under the
//! home `crate::locations::codex::CodexDetector` resolves.
//!
//! Layout researched from primary source during implementation (never
//! from a real `~/.codex` on this machine -- PRIVACY IS A HARD RULE),
//! all from <https://github.com/openai/codex> `codex-rs`, current `main`
//! as of this chunk:
//! - `codex-rs/utils/home-dir/src/lib.rs` (`find_codex_home`):
//!   `CODEX_HOME` env var, else `~/.codex`.
//! - `codex-rs/rollout/src/lib.rs`: `pub const SESSIONS_SUBDIR: &str =
//!   "sessions"`, `pub const ARCHIVED_SESSIONS_SUBDIR: &str =
//!   "archived_sessions"`.
//! - `codex-rs/rollout/src/list.rs`: sessions live in a `<year>/<month>/
//!   <day>/` tree under each subdir (`collect_dirs_desc`/
//!   `collect_rollout_day_files`).
//! - `codex-rs/rollout/src/rollout_file_name.rs`: filenames are
//!   `rollout-<YYYY-MM-DDTHH-MM-SS>-<thread-id>[_<rollout-id>].jsonl`
//!   (an underscore-suffixed second id only for a reverted thread).
//! - `codex-rs/app-server/src/codex_home_metrics.rs`: upstream's own
//!   background size metric walks exactly `sessions/` and
//!   `archived_sessions/` recursively, stat-only, without following
//!   symlinks and without reading file contents -- the same discipline
//!   `super::folded_bytes` already gives every adapter in this module,
//!   confirming (not just assuming) it matches upstream's own practice.
//! - `codex-rs/rollout/src/metadata.rs`: the rollout's first logical
//!   item is a `session_meta` entry carrying `meta.cwd` (and, when
//!   present, `git.commit_hash`/`git.branch`/`git.repository_url`,
//!   unused here -- only `cwd` is read, same one-field discipline
//!   `crate::agents::claude_code` uses). The exact envelope nesting
//!   (`payload.meta.cwd` vs `meta.cwd` vs a bare top-level `cwd`) is not
//!   pinned to one shape here: this is genuinely internal wire format,
//!   read only for this one field, and any shape not matched resolves to
//!   `Unresolved`, never a guess or a panic.
//! - `codex-rs/state/src/sqlite.rs`: `state_5.sqlite`, `logs_2.sqlite`,
//!   `goals_1.sqlite`, `memories_1.sqlite`, `queue_1.sqlite`,
//!   `thread_history_1.sqlite`, each `codex_home.join(<filename>)`.
//!   `SQLITE_HOME_ENV = "CODEX_SQLITE_HOME"` can relocate all of them
//!   *outside* `CODEX_HOME`; this adapter does not follow that override
//!   (documented gap, same shape as `claude_code`'s undstood
//!   `~/.claude.json` sibling gap) -- if set, these files are simply not
//!   found here rather than guessed at a wrong path.
//!
//! No managed-worktree creation by the Codex CLI itself is confirmed by
//! primary source this chunk, so `AgentCategory::ManagedWorktrees` is
//! never populated by this adapter -- an honest absence, not a silent
//! gap (`docs/agent-storage.md` records the same note in prose).

use super::{
    AgentActionCapability, AgentCategory, AgentMember, AgentMemberKind, CandidateAgentUnit,
    ProjectLinkState, folded_bytes, mtime_secs, resolve_declared_path,
};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const CODEX_TOOL_ID: &str = crate::locations::codex::CODEX_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;
/// Bound on how many bytes of a rollout file's first line this adapter
/// will ever read looking for a `cwd` field -- never whole transcripts.
const HEADER_READ_BYTES: usize = 8192;
/// Bound on how many nested date directories a session-tree walk
/// descends before giving up on a subtree, so a pathologically deep or
/// cyclic (symlink) layout cannot make identification unbounded. The
/// real layout is exactly three levels (year/month/day); this leaves
/// headroom for a future layout change without becoming unbounded.
const MAX_WALK_DEPTH: usize = 8;

pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    let mut units = Vec::new();
    identify_sessions(home, "sessions", false, &mut units);
    identify_sessions(home, "archived_sessions", true, &mut units);
    identify_sqlite_stores(home, &mut units);
    identify_static_categories(home, &mut units);
    units
}

// ---------------------------------------------------------------------
// Sessions (live + archived): each rollout file is its own unit -- no
// companion directory is documented or found in this chunk's source
// research, so a session's `members` is always exactly the one file.
// ---------------------------------------------------------------------

fn identify_sessions(home: &Path, subdir: &str, archived: bool, out: &mut Vec<CandidateAgentUnit>) {
    let base = home.join(subdir);
    let mut files = Vec::new();
    collect_jsonl_files(&base, 0, out.len(), &mut files);
    for jsonl in files {
        let Ok(meta) = fs::symlink_metadata(&jsonl) else {
            continue;
        };
        let bytes = meta.len();
        let mtime = mtime_secs(&meta);
        let project_link = resolve_declared_path(
            read_header_cwd(&jsonl),
            "no cwd field found in the session's first line",
        );
        let relative_path = relative_to(home, &jsonl);
        let note = archived.then(|| {
            "archived: hidden from the default thread list, but still unique conversation \
             history -- archiving is not evidence this session is unused"
                .to_string()
        });
        out.push(CandidateAgentUnit {
            category: AgentCategory::Sessions,
            relative_path,
            path: jsonl.clone(),
            members: vec![AgentMember {
                path: jsonl,
                bytes,
                kind: AgentMemberKind::Transcript,
            }],
            bytes,
            mtime_max: mtime,
            protected: false,
            protect_reason: None,
            project_link,
            action: AgentActionCapability::SessionRemoval,
            note,
        });
    }
}

/// Bounded recursive `*.jsonl` collection under `dir`, depth- and
/// entry-count-bounded like `super::folded_bytes`. `already_seen` is the
/// running count from earlier calls in the same `identify()` pass so
/// `sessions/` and `archived_sessions/` share one overall bound rather
/// than each independently allowing the full `MAX_FOLD_ENTRIES`.
fn collect_jsonl_files(dir: &Path, depth: usize, already_seen: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_WALK_DEPTH || already_seen + out.len() > MAX_FOLD_ENTRIES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        if already_seen + out.len() > MAX_FOLD_ENTRIES {
            return;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() && !ft.is_symlink() {
            collect_jsonl_files(&path, depth + 1, already_seen, out);
        } else if ft.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-"))
        {
            out.push(path);
        }
    }
}

fn read_header_cwd(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut buf = vec![0u8; HEADER_READ_BYTES];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf);
    let first_line = text.lines().next()?;
    let value: serde_json::Value = serde_json::from_str(first_line).ok()?;
    // Undocumented, version-varying envelope shape (see module doc
    // comment): try the plausible nestings in order, first match wins.
    for candidate in [
        value.get("cwd"),
        value.get("meta").and_then(|m| m.get("cwd")),
        value.get("payload").and_then(|p| {
            p.get("cwd")
                .or_else(|| p.get("meta").and_then(|m| m.get("cwd")))
        }),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(s) = candidate.as_str().filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------
// SQLite-backed state stores (#93's version boundary: newer Codex keeps
// these instead of, or alongside, flat history files). Each database is
// one unit with its `-wal`/`-shm` sidecars folded in as members;
// `crate::actions::is_sqlite_like` refuses any selective action on any
// of them unconditionally regardless of this adapter's own
// protected/action fields, which are set conservatively here anyway.
// ---------------------------------------------------------------------

struct SqliteStore {
    filename: &'static str,
    purpose: &'static str,
}

const SQLITE_STORES: &[SqliteStore] = &[
    SqliteStore {
        filename: "state_5.sqlite",
        purpose: "session/thread state index",
    },
    SqliteStore {
        filename: "logs_2.sqlite",
        purpose: "structured log store",
    },
    SqliteStore {
        filename: "goals_1.sqlite",
        purpose: "goal-tracking store",
    },
    SqliteStore {
        filename: "memories_1.sqlite",
        purpose: "episodic memory store",
    },
    SqliteStore {
        filename: "queue_1.sqlite",
        purpose: "task queue store",
    },
    SqliteStore {
        filename: "thread_history_1.sqlite",
        purpose: "thread history index",
    },
];

fn identify_sqlite_stores(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    for store in SQLITE_STORES {
        let path = home.join(store.filename);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mut members = vec![AgentMember {
            path: path.clone(),
            bytes: meta.len(),
            kind: AgentMemberKind::Database,
        }];
        let mut bytes = meta.len();
        let mut mtime_max = mtime_secs(&meta);
        for sidecar_ext in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{sidecar_ext}", path.display()));
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
            relative_path: store.filename.to_string(),
            path,
            members,
            bytes,
            mtime_max,
            protected: true,
            protect_reason: Some(format!(
                "SQLite {} (never opened while Codex may be writing; metadata-only, no \
                 per-row drill-down in this chunk)",
                store.purpose
            )),
            project_link: ProjectLinkState::NotApplicable,
            action: AgentActionCapability::None,
            note: None,
        });
    }
}

// ---------------------------------------------------------------------
// Static top-level categories.
// ---------------------------------------------------------------------

struct StaticEntry {
    rel: &'static str,
    category: AgentCategory,
    action: AgentActionCapability,
    protected: bool,
    note: &'static str,
}

const STATIC_ENTRIES: &[StaticEntry] = &[
    StaticEntry {
        rel: "config.toml",
        category: AgentCategory::ProtectedConfig,
        action: AgentActionCapability::None,
        protected: true,
        note: "main CLI configuration",
    },
    StaticEntry {
        rel: "auth.json",
        category: AgentCategory::ProtectedConfig,
        action: AgentActionCapability::None,
        protected: true,
        note: "OAuth/API credentials; contents are never read by this adapter",
    },
    StaticEntry {
        rel: "history.jsonl",
        category: AgentCategory::Sessions,
        action: AgentActionCapability::None,
        protected: true,
        note: "cross-session prompt history; contains prompt text and is never read by this \
               adapter",
    },
    StaticEntry {
        rel: "skills",
        category: AgentCategory::ProtectedConfig,
        action: AgentActionCapability::None,
        protected: true,
        note: "personal skill definitions; exact directory name found in source search but not \
               independently confirmed by a primary docs page this chunk -- treated as \
               protected pending confirmation, same discipline claude_code.rs uses for its own \
               unconfirmed entries",
    },
    StaticEntry {
        rel: "log",
        category: AgentCategory::Logs,
        action: AgentActionCapability::CacheOrLogTrash,
        protected: false,
        note: "CLI debug logs; regenerated automatically. Directory name is this epic's prior \
               research, not independently re-confirmed by source in this chunk",
    },
];

fn identify_static_categories(home: &Path, out: &mut Vec<CandidateAgentUnit>) {
    let mut seen_top_level: HashSet<String> = HashSet::new();
    for entry in STATIC_ENTRIES {
        seen_top_level.insert(entry.rel.to_string());
        let path = home.join(entry.rel);
        if !path.exists() {
            continue;
        }
        let (bytes, mtime, truncated) = folded_bytes(&path, MAX_FOLD_ENTRIES);
        let note = if truncated {
            format!(
                "{} (directory entry count bound reached; total may be an undercount)",
                entry.note
            )
        } else {
            entry.note.to_string()
        };
        out.push(CandidateAgentUnit {
            category: entry.category,
            relative_path: entry.rel.to_string(),
            path,
            members: Vec::new(),
            bytes,
            mtime_max: mtime,
            protected: entry.protected,
            protect_reason: entry.protected.then(|| entry.note.to_string()),
            project_link: ProjectLinkState::NotApplicable,
            action: entry.action,
            note: Some(note),
        });
    }
    for store in SQLITE_STORES {
        seen_top_level.insert(store.filename.to_string());
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
        if name == "sessions" || name == "archived_sessions" || seen_top_level.contains(&name) {
            continue;
        }
        let (bytes, mtime, _truncated) = folded_bytes(&e.path(), MAX_FOLD_ENTRIES);
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

    fn header_line(cwd: &str, canary: &str) -> String {
        format!(
            "{{\"type\":\"session_meta\",\"cwd\":\"{cwd}\",\"payload\":{{\"prompt\":\"{canary}\"}}}}\n"
        )
    }

    #[test]
    fn empty_home_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identify(dir.path(), 1).is_empty());
    }

    #[test]
    fn a_session_in_the_date_tree_is_identified_and_linked() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let canary = "CANARY-CODEX-DO-NOT-LEAK-91a3";
        let jsonl = home
            .join("sessions/2026/09/21/rollout-2026-09-21T10-00-00-11111111-1111-4111-8111-111111111111.jsonl");
        touch(
            &jsonl,
            header_line(&repo.display().to_string(), canary).as_bytes(),
        );
        let units = identify(home, 1);
        let session = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions && u.path == jsonl)
            .expect("session identified");
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Linked { .. }
        ));
        assert_eq!(session.action, AgentActionCapability::SessionRemoval);
        assert_eq!(session.members.len(), 1);
        let serialized = format!("{units:?}");
        assert!(!serialized.contains(canary), "prompt content leaked");
    }

    #[test]
    fn archived_sessions_are_identified_with_an_explicit_note() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let jsonl = home.join(
            "archived_sessions/2026/01/02/rollout-2026-01-02T00-00-00-22222222-2222-4222-8222-222222222222.jsonl",
        );
        touch(&jsonl, header_line("/nonexistent", "x").as_bytes());
        let units = identify(home, 1);
        let session = units.iter().find(|u| u.path == jsonl).unwrap();
        assert!(session.note.as_deref().unwrap().contains("archived"));
    }

    #[test]
    fn malformed_header_is_unresolved_not_a_panic() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let jsonl = home.join("sessions/2026/01/01/rollout-2026-01-01T00-00-00-x.jsonl");
        touch(&jsonl, b"not valid json");
        let units = identify(home, 1);
        let session = units.iter().find(|u| u.path == jsonl).unwrap();
        assert!(matches!(
            session.project_link,
            ProjectLinkState::Unresolved { .. }
        ));
    }

    #[test]
    fn sqlite_stores_are_protected_and_fold_wal_shm_sidecars() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("state_5.sqlite"), b"sqlite-bytes");
        touch(&home.join("state_5.sqlite-wal"), b"wal-bytes");
        touch(&home.join("state_5.sqlite-shm"), b"shm-bytes");
        let units = identify(home, 1);
        let db = units
            .iter()
            .find(|u| u.relative_path == "state_5.sqlite")
            .expect("db unit present");
        assert!(db.protected);
        assert_eq!(db.action, AgentActionCapability::None);
        assert_eq!(db.members.len(), 3, "db + wal + shm folded together");
        assert_eq!(
            db.bytes,
            "sqlite-bytes".len() as u64 + "wal-bytes".len() as u64 + "shm-bytes".len() as u64
        );
    }

    #[test]
    fn protected_config_is_protected_by_default() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("auth.json"), b"[redacted]");
        touch(&home.join("config.toml"), b"[redacted]");
        let units = identify(home, 1);
        for rel in ["auth.json", "config.toml"] {
            let u = units.iter().find(|u| u.relative_path == rel).unwrap();
            assert!(u.protected, "{rel} must be protected");
            assert_eq!(u.action, AgentActionCapability::None);
        }
    }

    #[test]
    fn log_directory_is_actionable_cache_or_log_trash() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("log").join("codex.log"), b"debug line");
        let units = identify(home, 1);
        let u = units.iter().find(|u| u.relative_path == "log").unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn unclassified_residual_captures_unknown_top_level_entries() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("some-future-file.json"), b"{}");
        let units = identify(home, 1);
        let residual = units
            .iter()
            .find(|u| u.relative_path == "(unclassified residual)")
            .expect("residual present");
        assert!(
            residual
                .note
                .as_deref()
                .unwrap()
                .contains("some-future-file.json")
        );
    }

    #[test]
    fn identification_cost_is_bounded_for_many_sessions() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("big-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        for i in 0..500 {
            let jsonl = home.join(format!(
                "sessions/2026/09/21/rollout-2026-09-21T10-00-{i:02}-77777777-7777-4777-8{i:03}-777777777777.jsonl"
            ));
            let mut content = header_line(&repo.display().to_string(), "unread-canary");
            content.push_str(&"x".repeat(200_000));
            touch(&jsonl, content.as_bytes());
        }
        let start = SystemTime::now();
        let units = identify(home, 1);
        let elapsed = SystemTime::now().duration_since(start).unwrap_or_default();
        eprintln!("[measured] codex identify() over 500 synthetic sessions took {elapsed:?}");
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
