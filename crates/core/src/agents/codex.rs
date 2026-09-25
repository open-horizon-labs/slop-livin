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
//! - 2026-09-25, structural probe of the 100 most recent local rollouts
//!   (key names and byte offsets only, no values): every first record is
//!   `{"timestamp","ordinal","type":"session_meta","payload":{...}}` with
//!   the `cwd` flattened at `payload.cwd`, 220-334 bytes in -- and every
//!   first record is longer than [`HEADER_READ_BYTES`] (22 KB median,
//!   48 KB max), because `payload.base_instructions.text` carries the
//!   project's instructions file in the same record. A parser that needs
//!   the whole line therefore found *no* Codex `cwd` at all (3,450
//!   sessions "unresolved" on the owner's machine). `read_header_cwd`
//!   now streams the record one byte at a time and stops at the closing
//!   quote of the `cwd` ([`SessionMetaCwd`] over
//!   `bounded_io::scan_header`): the instructions text after it is never
//!   fetched from the file, let alone parsed and discarded. Not used,
//!   deliberately: `payload.git.{branch,commit_hash,
//!   repository_url}` sits after the instructions text (18-48 KB in,
//!   92/100 records) and would need the read bound raised through it;
//!   `payload.forked_from_id` (9/100) names another *session*, not a
//!   project; per-turn `turn_context.cwd` records (9/100 within 64 KB)
//!   are beyond the first line. Each is a documented corroboration
//!   candidate, none is evidence this adapter reads.
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
    bounded_io, mtime_secs,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const CODEX_TOOL_ID: &str = "codex";

const MAX_FOLD_ENTRIES: usize = 200_000;
/// Rollout files one session-tree **container** will identify. Per
/// container, never shared: a bound a day directory shares with its
/// siblings would make its stored rows mean something different from a
/// live identification of the same directory (see [`collect_sessions`]).
const MAX_CONTAINER_ENTRIES: usize = 20_000;
/// Session-tree containers one pass will identify. Caps the whole pass
/// without making any one container's contents depend on another's.
const MAX_CONTAINERS: usize = 20_000;
/// Depth below a session root at which a directory becomes a container:
/// `sessions/<yyyy>/<mm>/<dd>/`, upstream's documented layout.
const CONTAINER_DEPTH: usize = 3;
/// Ceiling on how many bytes of a rollout file's first line this adapter
/// will ever read looking for a `cwd` field -- never whole transcripts.
/// A ceiling, not a read size: the read stops at the `cwd`'s closing
/// quote, a few hundred bytes in ([`SessionMetaCwd`]).
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
    let mut containers_used = 0usize;
    identify_sessions(
        home,
        "sessions",
        false,
        ctx,
        &mut units,
        &mut containers_used,
    );
    identify_sessions(
        home,
        "archived_sessions",
        true,
        ctx,
        &mut units,
        &mut containers_used,
    );
    identify_sqlite_stores(home, ctx, &mut units);
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
    containers_used: &mut usize,
) {
    let base = home.join(subdir);
    collect_sessions(&base, 0, home, archived, ctx, out, containers_used);
}

/// One rollout file's unit. Factored out so the same code produces it
/// whether it was found inside a container or directly under a session
/// root.
fn session_unit(
    home: &Path,
    jsonl: PathBuf,
    archived: bool,
    ctx: &IdentifyCtx,
) -> Option<CandidateAgentUnit> {
    let meta = ctx.stat(&jsonl).ok()?;
    let bytes = meta.len();
    let mtime = mtime_secs(&meta);
    // `project_link_declared`, not `project_link`: the declared path is
    // what a replayed container re-resolves live, so a worktree deleted
    // between two passes is never reported as still linked
    // (`crate::agents::LinkBasis`). A unit whose link is `Fixed` makes
    // its whole container unstorable.
    let declared = read_header_cwd(&jsonl, ctx);
    // `archived_sessions/` is its own category, not folded into
    // `sessions/`: stack/26's Codex reconciliation defect was exactly
    // this row reporting into `AgentCategory::Sessions` regardless of
    // `archived`, which summed live and archived bytes into one
    // "sessions" total a user could not decompose against `du`.
    let category = if archived {
        AgentCategory::ArchivedSessions
    } else {
        AgentCategory::Sessions
    };
    let mut unit = AgentUnitBuilder::new(CODEX_TOOL_ID, category, jsonl.clone())
        .relative_to(home)
        .members(vec![AgentMember {
            path: jsonl,
            bytes,
            kind: AgentMemberKind::Transcript,
        }])
        .mtime_max(mtime)
        .project_link_declared(declared, "no cwd field found in the session's first line")
        .action(AgentActionCapability::SessionRemoval);
    if archived {
        unit = unit.note(
            "archived: hidden from the default thread list, but still unique conversation \
             history -- archiving says nothing about whether this session is still needed",
        );
    }
    Some(unit.build())
}

/// Walks a session root, wrapping every **day directory**
/// (`sessions/<yyyy>/<mm>/<dd>/`) in [`IdentifyCtx::container`] so a day
/// nothing touched is replayed from the store instead of re-listed and
/// re-`stat`ed.
///
/// The bound is the reason this could not be done before. It used to be
/// one budget shared across `sessions/` and `archived_sessions/`
/// (`already_seen + out.len()`), which made a day's output depend on how
/// many files the days before it had produced -- so a day replayed from
/// the store would have meant something different from the same day
/// identified live, and the stored rows could not be trusted. The budget
/// is now **per container** ([`MAX_CONTAINER_ENTRIES`]), which each day
/// owns outright, plus a pass-level cap on how many containers are
/// identified at all ([`MAX_CONTAINERS`]). Both are deterministic from
/// the tree alone; neither depends on what a sibling produced.
fn collect_sessions(
    dir: &Path,
    depth: usize,
    home: &Path,
    archived: bool,
    ctx: &IdentifyCtx,
    out: &mut Vec<CandidateAgentUnit>,
    containers_used: &mut usize,
) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    for entry in ctx.list(dir) {
        let path = dir.join(&entry.name);
        if entry.is_dir {
            if depth + 1 == CONTAINER_DEPTH {
                if *containers_used >= MAX_CONTAINERS {
                    return;
                }
                *containers_used += 1;
                let units = ctx.container(CODEX_TOOL_ID, &path, &|| {
                    let mut files = Vec::new();
                    collect_jsonl_files(&path, depth + 1, ctx, &mut files);
                    files
                        .into_iter()
                        .filter_map(|jsonl| session_unit(home, jsonl, archived, ctx))
                        .collect()
                });
                out.extend(units);
            } else {
                collect_sessions(&path, depth + 1, home, archived, ctx, out, containers_used);
            }
        } else if is_rollout(&entry.name) {
            // A rollout file sitting above the day level (an older or
            // hand-moved layout) is identified inline: it belongs to no
            // container, so it is never replayed.
            out.extend(session_unit(home, path, archived, ctx));
        }
    }
}

fn is_rollout(name: &str) -> bool {
    name.ends_with(".jsonl") && name.starts_with("rollout-")
}

/// Bounded recursive `*.jsonl` collection under one container, one
/// explicit level at a time through the shared capped listing
/// (`IdentifyCtx::list` never follows a symlink and never recurses on
/// its own). The entry budget is this container's own: see
/// [`collect_sessions`].
fn collect_jsonl_files(dir: &Path, depth: usize, ctx: &IdentifyCtx, out: &mut Vec<PathBuf>) {
    if depth > MAX_WALK_DEPTH || out.len() >= MAX_CONTAINER_ENTRIES {
        return;
    }
    for entry in ctx.list(dir) {
        if out.len() >= MAX_CONTAINER_ENTRIES {
            return;
        }
        let path = dir.join(&entry.name);
        if entry.is_dir {
            collect_jsonl_files(&path, depth + 1, ctx, out);
        } else if is_rollout(&entry.name) {
            out.push(path);
        }
    }
}

/// The session's declared `cwd`, derived once per `(size, mtime,
/// adapter version)` through the identification cache: an unchanged
/// home costs zero header bytes on a second pass.
fn read_header_cwd(path: &Path, ctx: &IdentifyCtx) -> Option<String> {
    ctx.derived_scanned(
        CODEX_TOOL_ID,
        "cwd",
        path,
        HEADER_READ_BYTES,
        &|path, max_bytes| {
            let mut scanner = SessionMetaCwd::default();
            bounded_io::scan_header(path, max_bytes, &mut |b| scanner.feed(b))?;
            scanner.finish()
        },
    )
}

/// A byte-at-a-time scanner for the one field this adapter reads: the
/// `cwd` of a rollout's first `session_meta` record.
///
/// Why a scanner and not a parser: the record is far longer than the
/// `cwd` is deep into it (see the module doc's 2026-09-25 probe -- the
/// project's instructions text rides in the same record), and the
/// privacy contract is about what is *read*, not what is kept. So the
/// read ([`bounded_io::scan_header`]) fetches one byte per `read(2)` and
/// stops at the byte this scanner marks done: the closing quote of the
/// `cwd` value. Nothing after it is fetched from the file.
///
/// What it decodes: object key names (to know where it is), the
/// top-level `type`, and the `cwd` at one of the supported paths
/// (`cwd`, `meta.cwd`, `payload.cwd`, `payload.meta.cwd` -- the module
/// doc's tolerance). Every other value is skipped byte by byte, never
/// collected. Anything inside an array is not a supported path.
///
/// What it answers: the `cwd` when `type` was `session_meta` or no
/// `type` had appeared before the `cwd` (the older envelope shapes carry
/// none); nothing when `type` named another record kind -- and that
/// answer is given at the `type`'s closing quote, before any `cwd`. A
/// `type` written *after* the `cwd` is not consulted: reaching it would
/// mean reading past the field, which is the thing this exists to avoid.
/// Codex's serializer writes `type` before `payload`, as the probe saw
/// in 100/100 records.
#[derive(Default)]
struct SessionMetaCwd {
    /// One frame per open `{` / `[`. An object frame carries its current
    /// key; an array frame carries `None` and matches no supported path.
    stack: Vec<Frame>,
    /// Inside an object, the next string is a key.
    expect_key: bool,
    /// Inside a string: what to do with its bytes.
    string: Option<StringState>,
    record_type: Option<String>,
    cwd: Option<String>,
    /// The scan ended because the record could not carry the field
    /// (another record kind).
    refused: bool,
}

enum Frame {
    Object { key: Option<String> },
    Array,
}

struct StringState {
    capture: Capture,
    /// Raw bytes of a captured string (escapes still encoded), decoded
    /// as JSON at the closing quote. Empty for a skipped string.
    raw: Vec<u8>,
    escaped: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    Key,
    Type,
    Cwd,
    Skip,
}

impl SessionMetaCwd {
    fn path_is(&self, want: &[&str]) -> bool {
        self.stack.len() == want.len()
            && self.stack.iter().zip(want).all(|(f, w)| match f {
                Frame::Object { key: Some(k) } => k == w,
                _ => false,
            })
    }

    fn at_supported_cwd(&self) -> bool {
        self.path_is(&["cwd"])
            || self.path_is(&["meta", "cwd"])
            || self.path_is(&["payload", "cwd"])
            || self.path_is(&["payload", "meta", "cwd"])
    }

    fn feed(&mut self, b: u8) -> bounded_io::Scan {
        if self.string.is_some() {
            self.feed_in_string(b)
        } else {
            self.feed_structure(b)
        }
    }

    fn feed_in_string(&mut self, b: u8) -> bounded_io::Scan {
        use bounded_io::Scan;
        let Some(st) = self.string.as_mut() else {
            return Scan::More;
        };
        if st.escaped {
            st.escaped = false;
            if st.capture != Capture::Skip {
                st.raw.push(b);
            }
            return Scan::More;
        }
        match b {
            b'\\' => {
                st.escaped = true;
                if st.capture != Capture::Skip {
                    st.raw.push(b);
                }
                Scan::More
            }
            b'"' => {
                let Some(st) = self.string.take() else {
                    return Scan::More;
                };
                self.close_string(st)
            }
            _ => {
                if st.capture != Capture::Skip {
                    st.raw.push(b);
                }
                Scan::More
            }
        }
    }

    /// The closing quote of a string: a key names the frame, `type`
    /// decides whether the record can carry the field, the `cwd` ends
    /// the scan.
    fn close_string(&mut self, st: StringState) -> bounded_io::Scan {
        use bounded_io::Scan;
        match st.capture {
            Capture::Key => {
                if let Some(Frame::Object { key }) = self.stack.last_mut() {
                    *key = String::from_utf8(st.raw).ok();
                }
                Scan::More
            }
            Capture::Type => {
                let t = decode_json_string(&st.raw);
                let ok = t.as_deref() == Some("session_meta");
                self.record_type = t;
                if ok {
                    Scan::More
                } else {
                    self.refused = true;
                    Scan::Done
                }
            }
            Capture::Cwd => {
                self.cwd = decode_json_string(&st.raw);
                Scan::Done
            }
            Capture::Skip => Scan::More,
        }
    }

    fn feed_structure(&mut self, b: u8) -> bounded_io::Scan {
        use bounded_io::Scan;
        match b {
            b'{' => {
                self.stack.push(Frame::Object { key: None });
                self.expect_key = true;
            }
            b'[' => {
                self.stack.push(Frame::Array);
                self.expect_key = false;
            }
            b'}' | b']' => {
                self.stack.pop();
                self.expect_key = false;
            }
            b':' => self.expect_key = false,
            b',' => {
                self.expect_key = matches!(self.stack.last(), Some(Frame::Object { .. }));
            }
            b'"' => {
                let capture = if self.expect_key {
                    Capture::Key
                } else if self.path_is(&["type"]) {
                    Capture::Type
                } else if self.at_supported_cwd() {
                    Capture::Cwd
                } else {
                    Capture::Skip
                };
                self.expect_key = false;
                self.string = Some(StringState {
                    capture,
                    raw: Vec::new(),
                    escaped: false,
                });
            }
            b'\n' => {
                // End of the first line without the field.
                self.refused = true;
                return Scan::Done;
            }
            // Numbers, `true`/`false`/`null`, whitespace: structure
            // the delimiters above already track.
            _ => {}
        }
        Scan::More
    }

    fn finish(self) -> Option<String> {
        if self.refused {
            return None;
        }
        self.cwd.filter(|s| !s.is_empty())
    }
}

/// The JSON string whose raw (still escaped) contents are `raw`.
fn decode_json_string(raw: &[u8]) -> Option<String> {
    let mut quoted = Vec::with_capacity(raw.len() + 2);
    quoted.push(b'"');
    quoted.extend_from_slice(raw);
    quoted.push(b'"');
    serde_json::from_slice::<String>(&quoted).ok()
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

fn identify_sqlite_stores(home: &Path, ctx: &IdentifyCtx, out: &mut Vec<CandidateAgentUnit>) {
    for store in SQLITE_STORES {
        let path = home.join(store.filename);
        let Ok(meta) = ctx.stat(&path) else {
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
            if let Ok(sm) = ctx.stat(&sidecar)
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
            // `ProtectedDatabases`, not `Sessions`: these are state
            // stores, not conversation history, and folding them into
            // `Sessions` was stack/26's Codex reconciliation defect --
            // it inflated the reported "sessions" total by every
            // SQLite store's bytes (plus `-wal`/`-shm`) against what
            // `du` shows per top-level entry.
            AgentUnitBuilder::new(CODEX_TOOL_ID, AgentCategory::ProtectedDatabases, path)
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
    StaticEntry {
        rel: "plugins",
        category: AgentCategory::Plugins,
        action: AgentActionCapability::None,
        protected: false,
        note: "installed plugin cache (`~/.codex/plugins/cache/<marketplace>/<plugin>/<version>/`, \
               per developers.openai.com/codex/plugins/build: \"ChatGPT installs plugins into \
               ~/.codex/plugins/cache/$MARKETPLACE_NAME/$PLUGIN_NAME/$VERSION/\"); previously fell \
               into the unclassified residual, which is stack/26's Codex reconciliation defect",
    },
];

fn identify_static_categories(home: &Path, ctx: &IdentifyCtx, out: &mut Vec<CandidateAgentUnit>) {
    let mut seen_top_level: HashSet<String> = HashSet::new();
    for entry in STATIC_ENTRIES {
        seen_top_level.insert(entry.rel.to_string());
        let path = home.join(entry.rel);
        if !ctx.exists(&path) {
            continue;
        }
        let (bytes, mtime, truncated) = ctx.folded_bytes(&path, MAX_FOLD_ENTRIES);
        let mut unit = AgentUnitBuilder::new(CODEX_TOOL_ID, entry.category, path)
            .relative_path(entry.rel)
            .bytes(bytes)
            .mtime_max(mtime)
            .project_link(ProjectLinkState::NotApplicable)
            .action(entry.action)
            .note(entry.note);
        if truncated {
            unit =
                unit.incomplete("directory entry count bound reached; total may be an undercount");
        }
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
    use std::fs;
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
            .find(|u| u.category() == AgentCategory::Sessions && u.path == jsonl)
            .expect("session identified");
        assert!(matches!(
            session.project_link(),
            ProjectLinkState::Linked { .. }
        ));
        assert_eq!(session.action(), AgentActionCapability::SessionRemoval);
        assert_eq!(session.members().len(), 1);
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
                matches!(session.project_link(), ProjectLinkState::Linked { .. }),
                "shape {i} ({header}) must still resolve: {:?}",
                session.project_link()
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
            session.project_link(),
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
            .find(|u| u.relative_path() == "state_5.sqlite")
            .expect("db unit present");
        assert!(db.protected());
        assert_eq!(db.action(), AgentActionCapability::None);
        assert_eq!(db.members().len(), 3, "db + wal + shm folded together");
        assert_eq!(
            db.bytes(),
            "sqlite-bytes".len() as u64 + "wal-bytes".len() as u64 + "shm-bytes".len() as u64
        );
        assert!(
            db.members()
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
                .find(|u| u.relative_path() == name)
                .unwrap_or_else(|| panic!("{name} must be identified as its own store"));
            assert!(db.protected(), "{name} must be protected");
            assert_eq!(db.action(), AgentActionCapability::None, "{name}");
            assert_eq!(
                db.members().len(),
                3,
                "{name}: db + wal + shm folded together"
            );
            assert!(
                db.members()
                    .iter()
                    .all(|m| m.kind == AgentMemberKind::Database),
                "{name}"
            );
        }
        // And nothing leaks into the unclassified residual, which is
        // where an unmodelled store would otherwise have shown up.
        let residual = units
            .iter()
            .find(|u| u.category() == AgentCategory::Unclassified);
        assert!(
            residual.is_none(),
            "every runtime database must have its own unit: {:?}",
            residual.map(|u| u.relative_path().to_string())
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
            let u = units.iter().find(|u| u.relative_path() == rel).unwrap();
            assert!(u.protected(), "{rel} must be protected");
            assert_eq!(u.action(), AgentActionCapability::None);
        }
    }

    #[test]
    fn log_directory_is_actionable_cache_or_log_trash() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("log").join("codex.log"), b"debug line");
        let units = run(home);
        let u = units.iter().find(|u| u.relative_path() == "log").unwrap();
        assert!(!u.protected());
        assert_eq!(u.action(), AgentActionCapability::CacheOrLogTrash);
    }

    #[test]
    fn unclassified_residual_captures_unknown_top_level_entries() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        touch(&home.join("some-future-file.json"), b"{}");
        let units = run(home);
        let residual = units
            .iter()
            .find(|u| u.relative_path() == "(unclassified residual)")
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
                .filter(|u| u.category() == AgentCategory::Sessions)
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
            .find(|u| u.relative_path() == "(unclassified residual)")
            .expect("an explicit residual row, not silence");
        assert_eq!(residual.category(), AgentCategory::Unclassified);
        assert_eq!(residual.action(), AgentActionCapability::None);
        let note = residual.note.as_deref().unwrap_or_default();
        assert!(
            note.contains("no specific rule")
                && note.contains("unrecognised-v9-store.bin")
                && note.contains("future-layout"),
            "the residual must name what it could not classify: {note}"
        );
        assert!(
            matches!(residual.project_link(), ProjectLinkState::NotApplicable),
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

    /// The record Codex writes today (2026-09-25 local probe, 100/100
    /// rollouts): one `session_meta` line carrying `payload.cwd` a few
    /// hundred bytes in, followed in the *same* record by
    /// `payload.base_instructions.text` -- the project's instructions
    /// file, 22 KB median -- so the line is far longer than the ceiling.
    /// The read must end at the `cwd`'s closing quote: the canary that
    /// begins one byte later is never fetched, and the counted header
    /// bytes say so exactly.
    #[test]
    fn the_read_ends_at_the_closing_quote_of_the_cwd_and_the_canary_after_it_is_never_fetched() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let repo = home.join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let canary = "CANARY-instructions-3c9e";
        let head = format!(
            "{{\"timestamp\":\"2026-09-25T10:00:00.000Z\",\"ordinal\":0,\"type\":\"session_meta\",\
             \"payload\":{{\"session_id\":\"s\",\"id\":\"s\",\"forked_from_id\":null,\
             \"timestamp\":\"t\",\"cwd\":\"{}\"",
            repo.display()
        );
        let line = format!(
            "{head},\"canary\":\"{canary}\",\"runtime_workspace_roots\":[\"{}\"],\
             \"base_instructions\":{{\"text\":\"{}\"}},\"git\":{{\"branch\":\"main\"}}}}}}\n",
            repo.display(),
            "i".repeat(3 * HEADER_READ_BYTES)
        );
        assert!(line.len() > HEADER_READ_BYTES);
        let jsonl = home.join("sessions/2026/09/25/rollout-big.jsonl");
        touch(&jsonl, line.as_bytes());

        let (units, counters) = contract::measured(|| run(home));
        let unit = units.iter().find(|u| u.path() == jsonl).unwrap();
        match unit.project_link() {
            ProjectLinkState::Linked {
                source,
                project_path,
                ..
            } => {
                assert_eq!(*source, crate::agents::LinkSource::Declared);
                assert_eq!(project_path, &repo);
            }
            other => panic!("the early cwd must link the session: {other:?}"),
        }
        // Exactly the bytes through the cwd's closing quote -- `head`
        // ends with it -- and not one more.
        assert_eq!(
            counters.header_bytes_read,
            head.len() as u64,
            "the read must end at the closing quote of the cwd"
        );
        assert!(
            line[head.len()..].starts_with(",\"canary\""),
            "fixture: the canary must begin right after the cwd"
        );
        contract::no_content_leak(&units, canary);
    }

    /// The scanner over synthetic records, with the exact byte at which
    /// each scan stops. What it refuses: a `cwd` beyond the ceiling; a
    /// `cwd` the ceiling cuts through (never a prefix of a path); any
    /// record whose `type` is not `session_meta` -- refused at the
    /// `type`'s closing quote, before any `cwd`; a `cwd` inside an array.
    /// What it accepts: an escaped path, `payload.meta.cwd`, and a record
    /// with no `type` before the `cwd` (the older envelope shapes).
    #[test]
    fn the_scanner_stops_at_the_field_and_refuses_at_the_record_kind() {
        fn scan(line: &str, cap: usize) -> (Option<String>, usize) {
            let mut sc = SessionMetaCwd::default();
            let mut n = 0usize;
            for &b in line.as_bytes().iter().take(cap) {
                n += 1;
                if sc.feed(b) == bounded_io::Scan::Done {
                    break;
                }
            }
            (sc.finish(), n)
        }
        let cap = HEADER_READ_BYTES;
        let big = "x".repeat(cap);

        // cwd after the ceiling: nothing, and the read stopped at the cap.
        let after = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"base_instructions\":{{\"text\":\"{big}\"}},\"cwd\":\"/a/b\"}}}}"
        );
        assert_eq!(scan(&after, cap), (None, cap));

        // The ceiling lands inside the cwd value: never a partial path.
        let inside = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"/a/{}\"}}}}",
            "b".repeat(cap)
        );
        assert_eq!(scan(&inside, cap).0, None);

        // Another record kind: refused at the type's closing quote.
        let other = "{\"type\":\"turn_context\",\"payload\":{\"cwd\":\"/a/b\"}}";
        let (got, n) = scan(other, cap);
        assert_eq!(got, None);
        assert_eq!(n, "{\"type\":\"turn_context\"".len());

        // A cwd inside an array is not the supported path; the record
        // ends without one.
        let arr = "{\"type\":\"session_meta\",\"payload\":{\"roots\":[{\"cwd\":\"/a/b\"}]}}\n";
        assert_eq!(scan(arr, cap).0, None);

        // Escapes decode; the read ends at the closing quote.
        let escaped =
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/a/q\\\"b\\\\c\",\"x\":\"SECRET\"}}";
        let (got, n) = scan(escaped, cap);
        assert_eq!(got.as_deref(), Some("/a/q\"b\\c"));
        assert_eq!(
            &escaped[..n],
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/a/q\\\"b\\\\c\""
        );
        assert!(!escaped[..n].contains("SECRET"));

        // `payload.meta.cwd` and a record with no `type` before the cwd.
        let nested = "{\"type\":\"session_meta\",\"payload\":{\"meta\":{\"cwd\":\"/a/b\"}}}";
        assert_eq!(scan(nested, cap).0.as_deref(), Some("/a/b"));
        let untyped = "{\"payload\":{\"cwd\":\"/a/b\"},\"type\":\"session_meta\"}";
        let (got, n) = scan(untyped, cap);
        assert_eq!(got.as_deref(), Some("/a/b"));
        assert_eq!(&untyped[..n], "{\"payload\":{\"cwd\":\"/a/b\"");

        // A non-ASCII path arrives intact.
        let utf8 = "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/a/caf\u{e9}\"}}";
        assert_eq!(scan(utf8, cap).0.as_deref(), Some("/a/caf\u{e9}"));
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
                .filter(|u| u.category() == AgentCategory::Sessions)
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
            .find(|u| u.relative_path() == "history.jsonl")
            .expect("history.jsonl identified");
        assert!(history.protected());
        assert!(
            history
                .protect_reason()
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
        match &a.project_link() {
            ProjectLinkState::Linked { source, .. } => {
                assert_eq!(*source, crate::agents::LinkSource::Declared)
            }
            other => panic!("declared cwd must link: {other:?}"),
        }
        let b = units.iter().find(|u| u.path == guessed).unwrap();
        assert!(
            matches!(b.project_link(), ProjectLinkState::Unresolved { .. }),
            "a basename is not evidence: {:?}",
            b.project_link()
        );
        contract::linkage_is_declared_or_explicit(&units, "guessable-project-name");
    }

    /// stack/26's Codex reconciliation defect, falsified directly: build
    /// a home with one entry in every category this adapter knows about
    /// (live session, archived session, every SQLite store with its
    /// `-wal`/`-shm` sidecars, `config.toml`, `auth.json`, `skills/`,
    /// `log/`, `plugins/`) plus one genuinely unrecognized entry, and
    /// assert the identified units' bytes sum to *exactly* the home's
    /// own folded byte total -- not "close", not "within a category or
    /// two of each other". Before this chunk, `state_5.sqlite`/
    /// `logs_2.sqlite`/`thread_history_1.sqlite` (etc.) reported into
    /// `AgentCategory::Sessions`, `plugins/` fell into the unclassified
    /// residual, and `archived_sessions/` was indistinguishable from
    /// `sessions/` -- category *totals* were wrong even when this
    /// grand total happened to still add up, which is exactly the
    /// silent failure mode a per-category breakdown (`--view agents`)
    /// exists to prevent.
    #[test]
    fn category_totals_reconcile_against_the_homes_folded_bytes() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();

        touch(
            &home.join("sessions/2026/09/21/rollout-live.jsonl"),
            header_line("/nonexistent/live", "x").as_bytes(),
        );
        touch(
            &home.join(
                "archived_sessions/2026/01/02/rollout-2026-01-02T00-00-00-\
                 22222222-2222-4222-8222-222222222222.jsonl",
            ),
            header_line("/nonexistent/archived", "x").as_bytes(),
        );
        for store in SQLITE_STORES {
            touch(&home.join(store.filename), b"sqlite-bytes-payload");
            touch(&home.join(format!("{}-wal", store.filename)), b"wal-bytes");
            touch(&home.join(format!("{}-shm", store.filename)), b"shm-bytes");
        }
        touch(&home.join("config.toml"), b"model = \"gpt\"\n");
        touch(&home.join("auth.json"), b"{\"token\":\"x\"}");
        touch(&home.join("skills/foo/SKILL.md"), b"# a skill\n");
        touch(&home.join("log/codex.log"), b"debug line\n");
        touch(
            &home.join("plugins/marketplace/example-plugin/1.0.0/plugin.json"),
            b"{\"name\":\"example-plugin\"}",
        );
        // The one entry nothing here has a rule for.
        touch(&home.join("some-future-store.bin"), b"opaque-future-bytes");

        let units = run(home);

        // Every category this fixture touches is represented, and
        // distinctly: a defect that folds two of them together would
        // still pass a bytes-only reconciliation (the totals can agree
        // by coincidence), so this asserts the *set* of categories
        // present first.
        let categories: std::collections::HashSet<AgentCategory> =
            units.iter().map(|u| u.category()).collect();
        for expected in [
            AgentCategory::Sessions,
            AgentCategory::ArchivedSessions,
            AgentCategory::ProtectedDatabases,
            AgentCategory::ProtectedConfig,
            AgentCategory::Logs,
            AgentCategory::Plugins,
            AgentCategory::Unclassified,
        ] {
            assert!(
                categories.contains(&expected),
                "{expected:?} missing from {categories:?}"
            );
        }

        let identified_total: u64 = units.iter().map(|u| u.bytes()).sum();
        let (home_total, _mtime, truncated) = crate::agents::folded_bytes(home, MAX_FOLD_ENTRIES);
        assert!(!truncated, "fixture is far under the fold bound");
        assert_eq!(
            identified_total,
            home_total,
            "identified units: {:?}",
            units
                .iter()
                .map(|u| (u.category(), u.relative_path().to_string(), u.bytes()))
                .collect::<Vec<_>>()
        );
    }
}
