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
//!   `IdentifyCtx::folded_bytes` already gives every adapter in this
//!   module, confirming (not just assuming) it matches upstream's own
//!   practice.
//! - `codex-rs/rollout/src/metadata.rs`: the rollout's first logical
//!   item is a `session_meta` entry carrying `meta.cwd` (and, when
//!   present, `git.commit_hash`/`git.branch`/`git.repository_url`,
//!   unused here -- only `cwd` is read, same one-field discipline
//!   `crate::agents::claude_code` uses). The exact envelope nesting
//!   (`payload.meta.cwd` vs `meta.cwd` vs a bare top-level `cwd`) is not
//!   pinned to one shape here: this is genuinely internal wire format,
//!   read only for this one field, and any shape not matched resolves to
//!   `Unresolved`, never a guess or a panic.
//! - `codex-rs/state/src/sqlite.rs` @
//!   `ac7634b9f73ec1bf96466be7a5869f0949d20b30`:
//!   `const RUNTIME_DBS: [RuntimeDbSpec; 7]` -- `state_5.sqlite`,
//!   `logs_2.sqlite`, `goals_1.sqlite`, `memories_1.sqlite`,
//!   `memories_v2_1.sqlite`, `queue_1.sqlite`,
//!   `thread_history_1.sqlite`, each `codex_home.join(<filename>)`.
//!   Seven, not six: `memories_v2_1.sqlite` is declared as a
//!   struct-update over `MEMORIES_DB` with its filename inline, so
//!   counting the `*_DB_FILENAME` consts gives six and misses it.
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
    AdapterCapabilities, AgentActionCapability, AgentAdapter, AgentCategory, AgentMember,
    AgentMemberKind, AgentUnitBuilder, CandidateAgentUnit, IdentifyCtx, ProjectLinkState,
    mtime_secs, resolve_declared_path,
};
use std::collections::HashSet;
use std::fs;
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

pub struct Adapter;

impl AgentAdapter for Adapter {
    fn id(&self) -> &'static str {
        CODEX_TOOL_ID
    }
    fn name(&self) -> &'static str {
        "Codex"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities::default()
    }
    fn identify(&self, home: &Path, ctx: &IdentifyCtx) -> Vec<CandidateAgentUnit> {
        identify(home, ctx)
    }
}

pub fn identify(home: &Path, ctx: &IdentifyCtx) -> Vec<CandidateAgentUnit> {
    let mut units = Vec::new();
    identify_sessions(home, "sessions", false, ctx, &mut units);
    identify_sessions(home, "archived_sessions", true, ctx, &mut units);
    identify_sqlite_stores(home, &mut units);
    identify_static_categories(home, ctx, &mut units);
    units
}

// ---------------------------------------------------------------------
// Sessions (live + archived): each rollout file is its own unit -- no
// companion directory is documented or found in this chunk's source
// research, so a session's `members` is always exactly the one file.
// ---------------------------------------------------------------------

fn identify_sessions(
    home: &Path,
    subdir: &str,
    archived: bool,
    ctx: &IdentifyCtx,
    out: &mut Vec<CandidateAgentUnit>,
) {
    let base = home.join(subdir);
    let mut files = Vec::new();
    collect_jsonl_files(&base, 0, out.len(), ctx, &mut files);
    for jsonl in files {
        let Ok(meta) = fs::symlink_metadata(&jsonl) else {
            continue;
        };
        let bytes = meta.len();
        let mtime = mtime_secs(&meta);
        let project_link = resolve_declared_path(
            read_header_cwd(&jsonl, ctx),
            "no cwd field found in the session's first line",
        );
        let mut unit = AgentUnitBuilder::new(CODEX_TOOL_ID, AgentCategory::Sessions, jsonl.clone())
            .relative_to(home)
            .members(vec![AgentMember {
                path: jsonl,
                bytes,
                kind: AgentMemberKind::Transcript,
            }])
            .mtime_max(mtime)
            .project_link(project_link)
            .action(AgentActionCapability::SessionRemoval);
        if archived {
            unit = unit.note(
                "archived: hidden from the default thread list, but still unique conversation \
                 history -- archiving is not evidence this session is unused",
            );
        }
        out.push(unit.build());
    }
}

/// Bounded recursive `*.jsonl` collection under `dir`, one explicit
/// level at a time through the shared capped listing (`IdentifyCtx::list`
/// never follows a symlink and never recurses on its own), depth- and
/// entry-count-bounded like `IdentifyCtx::folded_bytes`. `already_seen`
/// is the running count from earlier calls in the same `identify()` pass
/// so `sessions/` and `archived_sessions/` share one overall bound rather
/// than each independently allowing the full `MAX_FOLD_ENTRIES`.
fn collect_jsonl_files(
    dir: &Path,
    depth: usize,
    already_seen: usize,
    ctx: &IdentifyCtx,
    out: &mut Vec<PathBuf>,
) {
    if depth > MAX_WALK_DEPTH || already_seen + out.len() > MAX_FOLD_ENTRIES {
        return;
    }
    for entry in ctx.list(dir) {
        if already_seen + out.len() > MAX_FOLD_ENTRIES {
            return;
        }
        let path = dir.join(&entry.name);
        if entry.is_dir {
            collect_jsonl_files(&path, depth + 1, already_seen, ctx, out);
        } else if entry.name.ends_with(".jsonl") && entry.name.starts_with("rollout-") {
            out.push(path);
        }
    }
}

/// The session's declared `cwd`, derived once per `(size, mtime,
/// adapter version)` through the identification cache: an unchanged
/// home costs zero header bytes on a second pass.
fn read_header_cwd(path: &Path, ctx: &IdentifyCtx) -> Option<String> {
    ctx.derived(CODEX_TOOL_ID, "cwd", path, HEADER_READ_BYTES, &|text| {
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
    })
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
    // The seventh. `RUNTIME_DBS: [RuntimeDbSpec; 7]` is easy to
    // under-count because this one's filename is an inline literal in a
    // struct-update expression rather than a `*_DB_FILENAME` const, and
    // there are exactly six of those. Missing it meant a state store
    // that was never folded with its `-wal`/`-shm` sidecars and never
    // put in the protected SQLite category -- the 2026-09-22 re-review
    // found it by reading the file this row already cited.
    SqliteStore {
        filename: "memories_v2_1.sqlite",
        purpose: "episodic memory store, v2 schema",
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
        out.push(
            AgentUnitBuilder::new(CODEX_TOOL_ID, AgentCategory::Sessions, path)
                .relative_path(store.filename)
                .bytes(bytes)
                .members_keep_bytes(members)
                .mtime_max(mtime_max)
                .project_link(ProjectLinkState::NotApplicable)
                .action(AgentActionCapability::None)
                .protect(format!(
                    "SQLite {} (never opened while Codex may be writing; metadata-only, no \
                     per-row drill-down in this chunk)",
                    store.purpose
                ))
                .build(),
        );
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

fn identify_static_categories(home: &Path, ctx: &IdentifyCtx, out: &mut Vec<CandidateAgentUnit>) {
    let mut seen_top_level: HashSet<String> = HashSet::new();
    for entry in STATIC_ENTRIES {
        seen_top_level.insert(entry.rel.to_string());
        let path = home.join(entry.rel);
        if !path.exists() {
            continue;
        }
        let (bytes, mtime, truncated) = ctx.folded_bytes(&path, MAX_FOLD_ENTRIES);
        let note = if truncated {
            format!(
                "{} (directory entry count bound reached; total may be an undercount)",
                entry.note
            )
        } else {
            entry.note.to_string()
        };
        let mut unit = AgentUnitBuilder::new(CODEX_TOOL_ID, entry.category, path)
            .relative_path(entry.rel)
            .bytes(bytes)
            .mtime_max(mtime)
            .project_link(ProjectLinkState::NotApplicable)
            .action(entry.action)
            .note(note);
        if entry.protected {
            unit = unit.protect(entry.note);
        }
        out.push(unit.build());
    }
    for store in SQLITE_STORES {
        seen_top_level.insert(store.filename.to_string());
        // The sidecars too. They are already members of the store's own
        // unit; leaving them out of this set counted their bytes a
        // second time in the unclassified residual, and reported a
        // `-wal` file as an unrecognized top-level entry of a layout
        // this adapter does in fact recognize.
        for sidecar_ext in ["-wal", "-shm"] {
            seen_top_level.insert(format!("{}{sidecar_ext}", store.filename));
        }
    }

    let mut residual_bytes = 0u64;
    let mut residual_mtime = 0u64;
    let mut residual_names: Vec<String> = Vec::new();
    for e in ctx.list(home) {
        let name = e.name;
        if name == "sessions" || name == "archived_sessions" || seen_top_level.contains(&name) {
            continue;
        }
        let (bytes, mtime, _truncated) = ctx.folded_bytes(&home.join(&name), MAX_FOLD_ENTRIES);
        residual_bytes += bytes;
        residual_mtime = residual_mtime.max(mtime);
        residual_names.push(name);
    }
    if !residual_names.is_empty() {
        residual_names.sort();
        out.push(
            AgentUnitBuilder::new(
                CODEX_TOOL_ID,
                AgentCategory::Unclassified,
                home.to_path_buf(),
            )
            .relative_path("(unclassified residual)")
            .bytes(residual_bytes)
            .mtime_max(residual_mtime)
            .project_link(ProjectLinkState::NotApplicable)
            .action(AgentActionCapability::None)
            .note(format!(
                "entries with no specific rule in this adapter: {}",
                residual_names.join(", ")
            ))
            .build(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{IdentificationCache, bounded_io, contract};
    use std::time::{Duration, SystemTime};

    fn run(home: &Path) -> Vec<CandidateAgentUnit> {
        let cache = IdentificationCache::disabled();
        identify(home, &IdentifyCtx::new(1, &cache))
    }

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
        assert!(run(dir.path()).is_empty());
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
        let units = run(home);
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
    fn the_envelope_nesting_around_cwd_is_not_pinned_to_one_shape() {
        // The tolerance the module doc records: `cwd`, `meta.cwd`,
        // `payload.cwd` and `payload.meta.cwd` all resolve, and anything
        // else is `Unresolved` rather than a guess.
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = repo_dir.path().join("declared-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let cwd = repo.display().to_string();
        for (i, header) in [
            format!("{{\"cwd\":\"{cwd}\"}}"),
            format!("{{\"meta\":{{\"cwd\":\"{cwd}\"}}}}"),
            format!("{{\"payload\":{{\"cwd\":\"{cwd}\"}}}}"),
            format!("{{\"payload\":{{\"meta\":{{\"cwd\":\"{cwd}\"}}}}}}"),
        ]
        .into_iter()
        .enumerate()
        {
            let home = tempfile::tempdir().unwrap();
            let jsonl = home
                .path()
                .join(format!("sessions/2026/09/21/rollout-shape-{i}.jsonl"));
            touch(&jsonl, format!("{header}\nbody\n").as_bytes());
            let units = run(home.path());
            let session = units.iter().find(|u| u.path == jsonl).unwrap();
            assert!(
                matches!(session.project_link, ProjectLinkState::Linked { .. }),
                "shape {i} ({header}) must still resolve: {:?}",
                session.project_link
            );
        }
    }

    #[test]
    fn archived_sessions_are_identified_with_an_explicit_note() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let jsonl = home.join(
            "archived_sessions/2026/01/02/rollout-2026-01-02T00-00-00-22222222-2222-4222-8222-222222222222.jsonl",
        );
        touch(&jsonl, header_line("/nonexistent", "x").as_bytes());
        let units = run(home);
        let session = units.iter().find(|u| u.path == jsonl).unwrap();
        assert!(session.note.as_deref().unwrap().contains("archived"));
    }

    #[test]
    fn malformed_header_is_unresolved_not_a_panic() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let jsonl = home.join("sessions/2026/01/01/rollout-2026-01-01T00-00-00-x.jsonl");
        touch(&jsonl, b"not valid json");
        let units = run(home);
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
        let units = run(home);
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
        assert!(
            db.members
                .iter()
                .all(|m| m.kind == AgentMemberKind::Database),
            "every member of a SQLite store stays a Database member"
        );
    }

    /// All **seven** runtime databases, each folded with its sidecars and
    /// each protected. The count is the assertion: `RUNTIME_DBS` is
    /// declared `[RuntimeDbSpec; 7]` upstream
    /// (`codex-rs/state/src/sqlite.rs:105-113` @
    /// `ac7634b9f73ec1bf96466be7a5869f0949d20b30`, vendored at
    /// `crates/core/tests/fixtures/upstream/codex/ac7634b9f7/sqlite.rs`),
    /// and this adapter modelled six -- `memories_v2_1.sqlite` was
    /// unprotected, unfolded and uncounted.
    #[test]
    fn every_one_of_the_seven_runtime_databases_is_folded_and_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let expected = [
            "state_5.sqlite",
            "logs_2.sqlite",
            "goals_1.sqlite",
            "memories_1.sqlite",
            "memories_v2_1.sqlite",
            "queue_1.sqlite",
            "thread_history_1.sqlite",
        ];
        for name in expected {
            touch(&home.join(name), b"sqlite-bytes");
            touch(&home.join(format!("{name}-wal")), b"wal");
            touch(&home.join(format!("{name}-shm")), b"shm");
        }
        let units = run(home);
        for name in expected {
            let db = units
                .iter()
                .find(|u| u.relative_path == name)
                .unwrap_or_else(|| panic!("{name} must be identified as its own store"));
            assert!(db.protected, "{name} must be protected");
            assert_eq!(db.action, AgentActionCapability::None, "{name}");
            assert_eq!(
                db.members.len(),
                3,
                "{name}: db + wal + shm folded together"
            );
            assert!(
                db.members
                    .iter()
                    .all(|m| m.kind == AgentMemberKind::Database),
                "{name}"
            );
        }
        // And nothing leaks into the unclassified residual, which is
        // where an unmodelled store would otherwise have shown up.
        let residual = units
            .iter()
            .find(|u| u.category == AgentCategory::Unclassified);
        assert!(
            residual.is_none(),
            "every runtime database must have its own unit: {:?}",
            residual.map(|u| u.relative_path.clone())
        );
    }

    #[test]
    fn protected_config_is_protected_by_default() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("auth.json"), b"[redacted]");
        touch(&home.join("config.toml"), b"[redacted]");
        let units = run(home);
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
        let units = run(home);
        let u = units.iter().find(|u| u.relative_path == "log").unwrap();
        assert!(!u.protected);
        assert_eq!(u.action, AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn unclassified_residual_captures_unknown_top_level_entries() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("some-future-file.json"), b"{}");
        let units = run(home);
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
        let units = run(home);
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

    // --- the five contract tests ---------------------------------------

    #[test]
    fn unknown_format_is_explicit_not_empty() {
        // A home holding nothing this adapter has a rule for is reported
        // as an explicit `(unclassified residual)` row naming what was
        // found -- never an empty vec, and never re-read as some other
        // Codex-family layout.
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("unrecognised-v9-store.bin"), b"\x00\x01\x02");
        fs::create_dir_all(home.join("future-layout")).unwrap();
        touch(&home.join("future-layout/thing.dat"), b"opaque");
        let units = run(home);
        assert!(!units.is_empty(), "an unrecognized layout must still speak");
        let residual = units
            .iter()
            .find(|u| u.relative_path == "(unclassified residual)")
            .expect("an explicit residual row, not silence");
        assert_eq!(residual.category, AgentCategory::Unclassified);
        assert_eq!(residual.action, AgentActionCapability::None);
        let note = residual.note.as_deref().unwrap_or_default();
        assert!(
            note.contains("no specific rule")
                && note.contains("unrecognised-v9-store.bin")
                && note.contains("future-layout"),
            "the residual must name what it could not classify: {note}"
        );
        assert!(
            matches!(residual.project_link, ProjectLinkState::NotApplicable),
            "an unclassified residual is tool-wide, never linked to a project"
        );
    }

    #[test]
    fn canary_content_never_appears_in_output() {
        let canary = "CANARY-CODEX-DO-NOT-LEAK-7c02";
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let jsonl = home.join("sessions/2026/09/21/rollout-2026-09-21T10-00-00-canary.jsonl");
        // The canary sits on the header line this adapter *does* read
        // and again in the body it must never reach.
        let mut content = header_line("/nonexistent", canary);
        content.push_str(&format!("{{\"role\":\"user\",\"text\":\"{canary}\"}}\n"));
        content.push_str(&format!("plain body line: {canary}\n"));
        touch(&jsonl, content.as_bytes());
        touch(&home.join("history.jsonl"), canary.as_bytes());
        let units = run(home);
        assert!(units.iter().any(|u| u.path == jsonl), "session identified");
        contract::no_content_leak(&units, canary);
    }

    #[test]
    fn identification_reads_no_more_than_header_cap() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let sessions = 40usize;
        let per_file = 100_000usize;
        for i in 0..sessions {
            let jsonl = home.join(format!("sessions/2026/09/21/rollout-cap-{i}.jsonl"));
            let mut content = header_line("/nonexistent", "unread");
            content.push_str(&"x".repeat(per_file));
            touch(&jsonl, content.as_bytes());
        }
        let (units, counters) = contract::measured(|| run(home));
        assert_eq!(
            units
                .iter()
                .filter(|u| u.category == AgentCategory::Sessions)
                .count(),
            sessions
        );
        // One capped header read per session, and this adapter's own cap
        // is tighter than the shared ceiling.
        assert!(
            counters.header_bytes_read <= (sessions * HEADER_READ_BYTES) as u64,
            "read {} bytes, above {sessions} x this adapter's {HEADER_READ_BYTES} byte cap",
            counters.header_bytes_read
        );
        assert!(
            counters.header_bytes_read < (sessions * per_file) as u64,
            "identification read a transcript's worth of bytes"
        );
        const {
            assert!(
                HEADER_READ_BYTES <= bounded_io::MAX_HEADER_BYTES,
                "this adapter's cap must sit under the shared ceiling"
            )
        };
        contract::within_header_cap(counters, sessions as u64);
    }

    #[test]
    fn protected_categories_default_protected() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("auth.json"), b"[redacted]");
        touch(&home.join("config.toml"), b"model = \"redacted\"\n");
        touch(&home.join("skills/mine/SKILL.md"), b"# redacted");
        touch(&home.join("history.jsonl"), b"{}\n");
        let units = run(home);
        contract::protection_defaults_hold(&units);
        // The two non-default-protected-category units this adapter
        // still protects on its own judgment keep saying why.
        let history = units
            .iter()
            .find(|u| u.relative_path == "history.jsonl")
            .expect("history.jsonl identified");
        assert!(history.protected);
        assert!(
            history
                .protect_reason
                .as_deref()
                .is_some_and(|r| r.contains("prompt history")),
            "the adapter's own protection reason must survive the builder"
        );
    }

    #[test]
    fn project_link_is_declared_or_unresolved_never_basename_guess() {
        // (a) declared metadata naming a real worktree resolves.
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = repo_dir.path().join("declared-worktree");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let declared = home.join("sessions/2026/09/21/rollout-declared.jsonl");
        touch(
            &declared,
            header_line(&repo.display().to_string(), "x").as_bytes(),
        );
        // (b) a session sitting in a directory *named* like a project,
        // declaring nothing, must never become a link.
        let guessed = home.join("sessions/guessable-project-name/rollout-guessed.jsonl");
        touch(&guessed, b"not a json header at all\n");
        let units = run(home);
        let a = units.iter().find(|u| u.path == declared).unwrap();
        match &a.project_link {
            ProjectLinkState::Linked { source, .. } => {
                assert_eq!(*source, crate::agents::LinkSource::Declared)
            }
            other => panic!("declared cwd must link: {other:?}"),
        }
        let b = units.iter().find(|u| u.path == guessed).unwrap();
        assert!(
            matches!(b.project_link, ProjectLinkState::Unresolved { .. }),
            "a basename is not evidence: {:?}",
            b.project_link
        );
        contract::linkage_is_declared_or_explicit(&units, "guessable-project-name");
    }
}
