//! Shared agent-tool storage discovery, model, and privacy-preserving
//! history (#91), built on chunk B2's external-unit model.
//!
//! An agent tool's home directory (e.g. Claude Code's `~/.claude`) is an
//! ordinary [`crate::external::ExternalUnit`]: detector-resolved
//! (`crate::locations`), identity independent of any project, measured
//! and history-tracked exactly like a Cargo registry or a Homebrew
//! prefix -- no new mechanism for the home level. This module identifies
//! the *interior* of that home into finer-grained [`AgentUnit`]s
//! (sessions, caches, logs, checkpoints, protected config, ...), the way
//! `crate::cargo_artifacts` identifies the interior of a `target/`
//! artifact row into [`crate::artifact::NestedArtifact`]s -- same shape,
//! different domain.
//!
//! History reuse is literal, not just architectural: an [`AgentUnit`]'s
//! growth/regrowth comes from the *same* current+reverse-delta Parquet
//! key family `crate::external`'s home-level units already use
//! (`crate::growth::observe_and_annotate_external` and friends). The key
//! is `(detector_id, category, device, path)`; nothing about that key
//! scheme assumes its `path` is a whole detector-resolved location
//! rather than a path *within* one, so an agent unit's key is simply
//! this tool's `detector_id`, an `"agent:"`-prefixed category string
//! (never collides with `crate::locations::StorageCategory`'s own kebab
//! strings), the home's device, and the unit's own canonical path. One
//! store, one key family, two granularities -- never a third model.
//!
//! Identification cost is bounded by construction: category directories
//! are folded (byte totals, not per-file retention), and a session's
//! project linkage reads only its transcript's first line. See
//! `identification_cost_is_bounded` in `claude_code.rs`'s tests for the
//! measured shape, and `.oh/sessions/2026-09-21-agent-storage-claude-code.md`
//! for the recorded number.
//!
//! Privacy is a hard contract, not a convention: nothing in this module
//! or its adapters reads past a transcript's first line, and nothing
//! here puts a path's *contents* into any `AgentUnit` field. Tests across
//! this module and `claude_code.rs` seed a canary string into fixture
//! session bodies and assert it never appears in any `AgentUnit`,
//! `Debug`, or JSON-serialized output.

pub mod aider;
pub mod claude_code;
pub mod cline;
pub mod codex;
pub mod codex_desktop;
pub mod continue_dev;
pub mod copilot_cli;
pub mod cursor;
pub mod gemini_cli;
pub mod matrix;
pub mod oh_my_pi;
pub mod opencode;
pub mod pi;
pub mod roo_code;
pub mod vscode_family;
pub mod windsurf;

use crate::growth::{ObservedExternal, annotate_readonly_external, observe_and_annotate_external};
use crate::scope::EffectiveScope;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------
// Categories (#91 acceptance: distinct categories only where evidence
// supports them, plus an unclassified residual bucket).
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCategory {
    Sessions,
    Attachments,
    Checkpoints,
    Caches,
    Logs,
    ManagedWorktrees,
    Plugins,
    ProtectedConfig,
    Unclassified,
}

impl AgentCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Attachments => "attachments",
            Self::Checkpoints => "checkpoints",
            Self::Caches => "caches",
            Self::Logs => "logs",
            Self::ManagedWorktrees => "managed-worktrees",
            Self::Plugins => "plugins",
            Self::ProtectedConfig => "protected-config",
            Self::Unclassified => "unclassified",
        }
    }

    /// The `"agent:"`-prefixed string used in the growth-store key, kept
    /// distinct from `StorageCategory`'s own kebab strings (see module
    /// docs) even though nothing currently collides in practice.
    fn key_str(self) -> String {
        format!("agent:{}", self.label())
    }

    /// Categories protected from any selective action by *default*
    /// (guardrail: "protect credentials/config/skills/automation
    /// definitions by default"). A unit outside these categories can
    /// still be individually protected (`AgentUnit.protected`) by the
    /// adapter (e.g. Claude Code's `history.jsonl`) or by a human
    /// `swamp protect` entry.
    pub fn default_protected(self) -> bool {
        matches!(self, Self::ProtectedConfig)
    }
}

// ---------------------------------------------------------------------
// Project linkage (#91's required project-linkage acceptance).
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkSource {
    /// Read directly from metadata the tool itself wrote (a session
    /// header's `cwd` field, a per-project directory's declared path).
    /// Never a basename guess.
    Declared,
    /// Derived from declared evidence plus a bounded inference step
    /// (e.g. corroborating one session's declared cwd against another's
    /// when the two disagree). Not currently produced by the Claude
    /// Code adapter; kept in the shared model for adapters where the
    /// only available evidence needs this weaker label.
    Inferred,
}

/// A tool session/workspace's relationship to a swamp project/worktree.
/// Every variant is a first-class, explicitly reported outcome -- never
/// silently collapsed into "no link" or a fabricated match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ProjectLinkState {
    /// Resolved to a known Git checkout/worktree identity
    /// (`crate::git`'s own object-store-based project id -- never a
    /// filesystem path or a basename match).
    Linked {
        project_id: String,
        project_name: String,
        project_path: PathBuf,
        source: LinkSource,
        /// `"main"` or `"linked"` (`crate::report::WorktreeKind`,
        /// stringified for display; never re-typed here).
        worktree_kind: String,
    },
    /// No project metadata could be extracted at all (malformed/empty
    /// header, no `cwd` field found in the bounded read).
    Unresolved { reason: String },
    /// Metadata names a path that no longer exists on disk.
    Missing { path: PathBuf },
    /// Metadata names a path that exists but is not inside any Git
    /// checkout/worktree this adapter can identify.
    NotAProject { path: PathBuf },
    /// The declared path used to resolve to one project identity and
    /// now resolves to a different one (or none). Not populated by the
    /// Claude Code adapter in this chunk -- it has no record of a
    /// session's *previous* linkage to compare against; recorded here,
    /// honestly unused, rather than guessed.
    Moved { from: PathBuf, to: PathBuf },
    /// The declared path is on a different host than this observation
    /// is running on. Not populated by the Claude Code adapter in this
    /// chunk (no reliable remote-host signal in a session header).
    Remote { host: String, path: PathBuf },
    /// This unit's members collectively named more than one distinct
    /// project identity. Not produced by the Claude Code adapter this
    /// chunk (one session unit always has exactly one declared cwd);
    /// kept for adapters/aggregates where it can genuinely happen.
    Shared { project_ids: Vec<String> },
    /// This unit is inherently tool-wide (a cache, a log directory,
    /// protected config): project linkage does not apply, which is a
    /// different, more honest fact than "we tried and could not tell".
    NotApplicable,
}

// ---------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMemberKind {
    Transcript,
    SubagentDir,
    Todos,
    FileHistory,
    Attachments,
    CategoryDir,
    ConfigFile,
    /// A SQLite database file (or one of its `-wal`/`-shm` sidecars)
    /// backing session/message/state storage for a tool whose newer
    /// layout moved off flat JSON/JSONL files (#93 Codex, #95 OpenCode's
    /// `opencode.db`, #94 Oh My Pi's `agent.db`). Always folded into one
    /// unit with its sidecars as members, never split -- `is_sqlite_like`
    /// in `crate::actions` refuses any selective action on a path with
    /// this kind unconditionally, independent of category/protection.
    Database,
    /// A session-keyed companion directory/file that is neither a raw
    /// transcript, a subagent dir, todos, file-history, nor an
    /// attachment -- e.g. OpenCode's `storage/message/<session-id>/` and
    /// `storage/session_diff/<session-id>/`, matched to a session by the
    /// same exact-id-match discipline `crate::agents::claude_code` uses
    /// for `file-history/`/`image-cache/`/`uploads/`, never guessed.
    SessionData,
}

/// One physical path this unit's byte total is made of. A session unit
/// typically has several (transcript file, companion subagent dir,
/// matching todos entries, a `file-history/<session>/` directory); a
/// category unit typically has exactly one (its own folded directory or
/// file). #101's session removal moves exactly these members together,
/// after re-verifying each still belongs only to this unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMember {
    pub path: PathBuf,
    pub bytes: u64,
    pub kind: AgentMemberKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentActionCapability {
    /// No supported selective action for this unit yet. The default;
    /// identification always works with cleanup disabled.
    None,
    /// A single recoverable Trash move of this unit's one category
    /// directory (cache/log categories only -- see `agents_actions`).
    CacheOrLogTrash,
    /// Removing this unit means moving its whole member set together,
    /// with reference/occupancy re-verification and explicit loss
    /// warnings (#101's "explicit individual session removal").
    SessionRemoval,
}

/// One identified unit of agent-tool storage: a session, or a folded
/// category directory/file. Identity is `(tool_id, category,
/// relative_path)` -- independent of any project attribution, per
/// `crate::artifact`'s own stated discipline for nested units.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUnit {
    pub tool_id: String,
    pub tool_name: String,
    /// This tool's home directory, as resolved this pass (detector
    /// override or convention). Carried per-unit, not assumed shared
    /// across every unit in a list, so a caller acting on a mixed
    /// selection across multiple tools always knows which home to
    /// re-derive facts against (e.g. `execute_agent_session_removal`'s
    /// fresh re-identification).
    pub tool_home: PathBuf,
    pub category: AgentCategory,
    /// Stable content-addressed id: `blake3(tool_id, category, relative_path)`.
    pub id: String,
    /// Relative to the tool home (`crate::locations`-resolved), forward
    /// slashes, never absolute.
    pub relative_path: String,
    /// This unit's own canonical anchor path (a session's transcript
    /// file, or a category's own directory/file).
    pub path: PathBuf,
    pub members: Vec<AgentMember>,
    pub bytes: u64,
    pub hardlinked: bool,
    pub growth_bytes: Option<i64>,
    pub regrowth_count: u32,
    pub observed_at: u64,
    pub mtime_max: u64,
    /// True when this unit may never be selectively acted on, either by
    /// category default (`AgentCategory::default_protected`), by the
    /// adapter's own judgment (e.g. `history.jsonl`), or by a human
    /// `swamp protect` entry. `protect_reason` says which.
    pub protected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protect_reason: Option<String>,
    pub project_link: ProjectLinkState,
    pub action: AgentActionCapability,
    /// Coverage/unknown notes (e.g. "coverage incomplete this pass:
    /// could not be read"), never content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What an adapter (`claude_code::identify`) produces before growth
/// history and human-protect intent are layered on by
/// [`discover_and_measure`]. Never serialized on its own; exists so an
/// adapter never has to fabricate `growth_bytes`/`regrowth_count`.
#[derive(Debug)]
pub struct CandidateAgentUnit {
    pub category: AgentCategory,
    pub relative_path: String,
    pub path: PathBuf,
    pub members: Vec<AgentMember>,
    pub bytes: u64,
    pub mtime_max: u64,
    pub protected: bool,
    pub protect_reason: Option<String>,
    pub project_link: ProjectLinkState,
    pub action: AgentActionCapability,
    pub note: Option<String>,
}

pub fn unit_id(tool_id: &str, category: AgentCategory, relative_path: &str) -> String {
    crate::entities::id_for(&format!(
        "agent-unit:v1:{tool_id}:{}:{relative_path}",
        category.label()
    ))
}

fn unit_key(tool_id: &str, category: AgentCategory, device: u64, path: &Path) -> String {
    crate::growth::external_row_key(
        tool_id,
        &category.key_str(),
        device,
        &path.display().to_string(),
    )
}

pub(crate) fn device_of(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path).map(|m| m.dev()).unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

/// Bounded, stat-only folded byte total for `path` (file or directory),
/// deliberately *not* `crate::walk::resize_artifact`'s parallel-pool
/// machinery: that machinery is tuned for a handful of potentially huge
/// artifact roots, not hundreds of small per-session directories, and
/// spinning up its thread pool that many times would itself be the
/// "unacceptable scanning cost" #91 guards against. Reads directory
/// names and `stat` calls only -- never file contents. Bounded by
/// `max_entries`; a directory that hits the bound is reported truncated
/// rather than silently under-measured.
pub fn folded_bytes(path: &Path, max_entries: usize) -> (u64, u64, bool) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0, false);
    };
    if meta.is_file() {
        let mtime = mtime_secs(&meta);
        return (meta.len(), mtime, false);
    }
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return (0, 0, false);
    }
    let mut total = 0u64;
    let mut mtime_max = mtime_secs(&meta);
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0usize;
    let mut truncated = false;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            seen += 1;
            if seen > max_entries {
                truncated = true;
                break;
            }
            let Ok(m) = entry.metadata() else { continue };
            mtime_max = mtime_max.max(mtime_secs(&m));
            if m.is_dir() && !m.file_type().is_symlink() {
                stack.push(entry.path());
            } else if m.is_file() {
                total += m.len();
            }
        }
        if truncated {
            break;
        }
    }
    (total, mtime_max, truncated)
}

pub(crate) fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Shared declared-path -> project-identity resolution (#91's original
// contract, factored out during #93/#94/#95 so every adapter that reads
// a declared absolute path out of its own tool's metadata --
// `claude_code`'s transcript `cwd`, `codex`'s session-header `cwd`,
// `oh_my_pi`'s session-header `cwd`, `opencode`'s `project.json`
// `worktree` -- resolves it against swamp's project/worktree identity
// the same way, once. Never a basename guess: this walks upward from
// `path` looking for a `.git` directory/file and resolves through
// `crate::git`'s own object-store-based project identity primitives.
// ---------------------------------------------------------------------

/// Resolves an absolute path declared by some tool's own metadata (never
/// a filename/directory-name guess) to this swamp instance's project
/// identity. `field_missing_reason` is the adapter-specific explanation
/// for why no path could be extracted at all (e.g. "no cwd field found
/// in the session's first line"), used only for the `Unresolved` case.
pub(crate) fn resolve_declared_path(
    declared: Option<String>,
    field_missing_reason: &str,
) -> ProjectLinkState {
    let Some(declared) = declared else {
        return ProjectLinkState::Unresolved {
            reason: field_missing_reason.to_string(),
        };
    };
    let path = PathBuf::from(&declared);
    if !path.exists() {
        return ProjectLinkState::Missing { path };
    }
    for ancestor in path.ancestors() {
        let git_path = ancestor.join(".git");
        let Ok(git_meta) = fs::symlink_metadata(&git_path) else {
            continue;
        };
        if git_meta.is_dir() {
            if let Some(dw) = crate::git::classify_main_checkout(ancestor, &git_path) {
                return ProjectLinkState::Linked {
                    project_id: dw.project_id,
                    project_name: dw.project_name,
                    project_path: dw.path,
                    source: LinkSource::Declared,
                    worktree_kind: "main".to_string(),
                };
            }
        } else if git_meta.is_file()
            && let Some(dw) = crate::git::classify_git_file(ancestor, &git_path)
        {
            let kind = if dw.kind == crate::report::WorktreeKind::Linked {
                "linked"
            } else {
                "main"
            };
            return ProjectLinkState::Linked {
                project_id: dw.project_id,
                project_name: dw.project_name,
                project_path: dw.path,
                source: LinkSource::Declared,
                worktree_kind: kind.to_string(),
            };
        }
    }
    ProjectLinkState::NotAProject { path }
}

/// The worktree root containing `path`, if any -- reuses
/// `resolve_declared_path`'s own upward `.git` search. Lets a caller
/// that only has one exact target path (`swamp propose-agents --path`,
/// which does not compute a full `Report`) still supply
/// `discover_and_measure`'s `project_worktrees` for Aider's per-repo
/// units, without a whole-scope walk.
pub fn worktree_root_containing(path: &Path) -> Option<PathBuf> {
    match resolve_declared_path(Some(path.display().to_string()), "") {
        ProjectLinkState::Linked { project_path, .. } => Some(project_path),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Human keep/protect intent (#100's `swamp protect add/list/remove`):
// a small JSON sidecar, deliberately decoupled from the growth store,
// mirroring `external.rs`'s consumer-association sidecar. Survives
// refresh; blocks actions; never inferred from observation.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProtectFile {
    /// Canonical absolute paths a human explicitly asked to keep.
    paths: Vec<String>,
}

fn protect_path(swamp_dir: &Path) -> PathBuf {
    swamp_dir.join("agent_protect.json")
}

fn load_protect(swamp_dir: &Path) -> Result<Vec<PathBuf>> {
    match fs::read_to_string(protect_path(swamp_dir)) {
        Ok(text) => {
            let f: ProtectFile = serde_json::from_str(&text).unwrap_or_default();
            Ok(f.paths.into_iter().map(PathBuf::from).collect())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn save_protect(swamp_dir: &Path, paths: &[PathBuf]) -> Result<()> {
    fs::create_dir_all(swamp_dir)?;
    let f = ProtectFile {
        paths: paths.iter().map(|p| p.display().to_string()).collect(),
    };
    fs::write(protect_path(swamp_dir), serde_json::to_string_pretty(&f)?)?;
    Ok(())
}

/// Adds `path` to the human keep list, used verbatim (never
/// canonicalized): `AgentUnit.path`/`AgentMember.path` are themselves
/// built as non-canonical `home.join(relative)` joins, and canonicalizing
/// only one side of the comparison risks a silent mismatch wherever the
/// tool home sits under a symlinked directory (e.g. macOS `/tmp` ->
/// `/private/tmp`). A caller that wants symlink-independent protection
/// can canonicalize before calling. Idempotent; a not-yet-observed path
/// can still be protected in advance.
pub fn protect_add(swamp_dir: &Path, path: &Path) -> Result<()> {
    let mut paths = load_protect(swamp_dir)?;
    if !paths.iter().any(|p| p == path) {
        paths.push(path.to_path_buf());
        save_protect(swamp_dir, &paths)?;
    }
    Ok(())
}

pub fn protect_remove(swamp_dir: &Path, path: &Path) -> Result<()> {
    let mut paths = load_protect(swamp_dir)?;
    let before = paths.len();
    paths.retain(|p| p != path);
    if paths.len() != before {
        save_protect(swamp_dir, &paths)?;
    }
    Ok(())
}

pub fn protect_list(swamp_dir: &Path) -> Result<Vec<PathBuf>> {
    load_protect(swamp_dir)
}

/// Whether `path`, or any of `unit_paths` (a unit's own members), falls
/// under a human-protected path (exact match or protected-path is an
/// ancestor).
fn is_human_protected(protected: &[PathBuf], candidate: &Path) -> bool {
    protected
        .iter()
        .any(|p| candidate == p || candidate.starts_with(p))
}

// ---------------------------------------------------------------------
// Active-session check (#92's acceptance): occupancy is checked only at
// the point of proposing/executing an action on a *specific* unit, not
// during ordinary identification -- an `lsof`-style check per unit would
// mean hundreds of process spawns on an ordinary `report`, which is
// exactly the "unacceptable scanning cost" #91 guards against, and would
// violate the no-blocking-scan discipline the TUI's render/event path
// depends on. `crate::occupancy::occupied` is the existing seam.
// ---------------------------------------------------------------------

pub fn is_active(path: &Path) -> bool {
    crate::occupancy::occupied(path)
}

// ---------------------------------------------------------------------
// Discovery orchestration
// ---------------------------------------------------------------------

/// Every tool this chunk implements identification for. Extending this
/// list is how a future adapter (#93-#99) plugs in; see
/// `crate::agents::matrix` for the full required-tool matrix, including
/// the tools with no entry here yet.
fn identify_for_tool(
    tool_id: &str,
    home: &Path,
    observed_at: u64,
) -> Option<Vec<CandidateAgentUnit>> {
    match tool_id {
        claude_code::CLAUDE_CODE_TOOL_ID => Some(claude_code::identify(home, observed_at)),
        codex::CODEX_TOOL_ID => Some(codex::identify(home, observed_at)),
        codex_desktop::CODEX_DESKTOP_TOOL_ID => Some(codex_desktop::identify(home, observed_at)),
        oh_my_pi::OH_MY_PI_TOOL_ID => Some(oh_my_pi::identify(home, observed_at)),
        opencode::OPENCODE_TOOL_ID => Some(opencode::identify(home, observed_at)),
        gemini_cli::GEMINI_CLI_TOOL_ID => Some(gemini_cli::identify(home, observed_at)),
        pi::PI_TOOL_ID => Some(pi::identify(home, observed_at)),
        // aider::AIDER_TOOL_ID is dispatched here too (home-level caches/
        // only); its per-repo units come from a separate code path -- see
        // `discover_and_measure`'s `project_worktrees` handling below.
        aider::AIDER_TOOL_ID => Some(aider::identify(home, observed_at)),
        copilot_cli::COPILOT_CLI_TOOL_ID => Some(copilot_cli::identify(home, observed_at)),
        cursor::CURSOR_TOOL_ID => Some(cursor::identify(home, observed_at)),
        windsurf::WINDSURF_TOOL_ID => Some(windsurf::identify(home, observed_at)),
        cline::CLINE_TOOL_ID => Some(cline::identify(home, observed_at)),
        roo_code::ROO_CODE_TOOL_ID => Some(roo_code::identify(home, observed_at)),
        continue_dev::CONTINUE_TOOL_ID => Some(continue_dev::identify(home, observed_at)),
        _ => None,
    }
}

/// Tool ids whose storage can be installed into more than one editor
/// host at once (#99's explicit "model each host as a separate detector
/// location, dedupe nothing that is genuinely separate storage"):
/// `discover_and_measure` decomposes *every* `Resolved` location this
/// detector proposes, not just the first, unlike every other tool in
/// this catalog (including the multi-location `opencode`/`copilot-cli`/
/// `cursor`/`windsurf` detectors, whose secondary locations are
/// deliberately *not* decomposed -- see each one's own doc comment).
fn multi_location_tool(tool_id: &str) -> bool {
    matches!(tool_id, cline::CLINE_TOOL_ID | roo_code::ROO_CODE_TOOL_ID)
}

fn tool_name_for(tool_id: &str, scope: &EffectiveScope) -> String {
    scope
        .detectors
        .iter()
        .find(|d| d.detector_id == tool_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| tool_id.to_string())
}

/// Discovers and measures every agent unit for every implemented adapter
/// whose tool home resolved in `scope`. Mirrors
/// `external::discover_and_measure`'s `swamp_dir`/`observe` contract
/// exactly (`None` => no history; `observe: false` => read-only
/// annotation of existing history; `observe: true` => persist this
/// pass). Never walks a tool home not resolved by the detector registry,
/// and never reads past what each adapter's own bounded contract allows.
pub fn discover_and_measure(
    scope: &EffectiveScope,
    project_worktrees: &[PathBuf],
    swamp_dir: Option<&Path>,
    observe: bool,
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<Vec<AgentUnit>> {
    let protected_paths = swamp_dir.map(load_protect).transpose()?.unwrap_or_default();

    let mut candidates_by_key: HashMap<String, (String, PathBuf, CandidateAgentUnit, u64)> =
        HashMap::new();
    let mut observed: Vec<ObservedExternal> = Vec::new();

    for summary in &scope.detectors {
        let resolved_locations = summary
            .locations
            .iter()
            .filter(|l| l.status == crate::locations::LocationStatus::Resolved);
        let homes: Vec<&Path> = if multi_location_tool(&summary.detector_id) {
            resolved_locations
                .filter_map(|l| l.path.as_deref())
                .collect()
        } else {
            resolved_locations
                .filter_map(|l| l.path.as_deref())
                .take(1)
                .collect()
        };
        let tool_name = tool_name_for(&summary.detector_id, scope);
        for home in homes {
            let Some(units) = identify_for_tool(&summary.detector_id, home, observed_at) else {
                continue;
            };
            let device = device_of(home);
            for cand in units {
                let key = unit_key(&summary.detector_id, cand.category, device, &cand.path);
                observed.push(ObservedExternal {
                    key: key.clone(),
                    detector_id: summary.detector_id.clone(),
                    category: cand.category.key_str(),
                    device,
                    path: cand.path.display().to_string(),
                    bytes: cand.bytes,
                    hardlinked: true,
                });
                candidates_by_key
                    .insert(key, (tool_name.clone(), home.to_path_buf(), cand, device));
            }
        }
    }

    // Aider's per-repo units (#96): materially different shape from
    // every other tool in this catalog -- attached to each *known
    // project worktree root* the caller supplies, never derived from a
    // `crate::locations` detector home. Skipped entirely when the
    // `aider` detector itself is disabled, so disabling a detector
    // always turns off everything it would otherwise identify, home-
    // level or project-local alike.
    let aider_disabled = scope
        .detectors
        .iter()
        .find(|d| d.detector_id == aider::AIDER_TOOL_ID)
        .is_some_and(|d| {
            d.locations
                .iter()
                .any(|l| l.status == crate::locations::LocationStatus::Disabled)
        });
    if !aider_disabled {
        let tool_name = tool_name_for(aider::AIDER_TOOL_ID, scope);
        for wt_path in project_worktrees {
            let device = device_of(wt_path);
            for cand in aider::identify_repo_units(wt_path, observed_at) {
                let key = unit_key(aider::AIDER_TOOL_ID, cand.category, device, &cand.path);
                observed.push(ObservedExternal {
                    key: key.clone(),
                    detector_id: aider::AIDER_TOOL_ID.to_string(),
                    category: cand.category.key_str(),
                    device,
                    path: cand.path.display().to_string(),
                    bytes: cand.bytes,
                    hardlinked: true,
                });
                candidates_by_key.insert(key, (tool_name.clone(), wt_path.clone(), cand, device));
            }
        }
    }

    let annotations: HashMap<String, (Option<i64>, u32)> = match swamp_dir {
        Some(dir) if observe => observe_and_annotate_external(
            dir,
            &observed,
            &HashSet::new(),
            observed_at,
            retention_days,
            since_secs,
        )?,
        Some(dir) => {
            let keys: Vec<String> = observed.iter().map(|o| o.key.clone()).collect();
            annotate_readonly_external(dir, &keys, observed_at, retention_days, since_secs)?
        }
        None => HashMap::new(),
    };

    let mut units = Vec::with_capacity(candidates_by_key.len());
    for (key, (tool_name, tool_home, cand, _device)) in candidates_by_key {
        let (growth_bytes, regrowth_count) = annotations.get(&key).copied().unwrap_or((None, 0));
        let default_protected = cand.category.default_protected();
        let human_protected = is_human_protected(&protected_paths, &cand.path)
            || cand
                .members
                .iter()
                .any(|m| is_human_protected(&protected_paths, &m.path));
        let (protected, protect_reason) = if cand.protected {
            (true, cand.protect_reason.clone())
        } else if default_protected {
            (
                true,
                Some(format!(
                    "{} is protected by default (credentials/config/skills/automation)",
                    cand.category.label()
                )),
            )
        } else if human_protected {
            (true, Some("kept by `swamp protect`".to_string()))
        } else {
            (false, None)
        };
        // Extract the detector_id back out of the key rather than
        // threading it separately; the key's first field always is it.
        let tool_id = key.split('\u{1}').next().unwrap_or_default().to_string();
        units.push(AgentUnit {
            id: unit_id(&tool_id, cand.category, &cand.relative_path),
            tool_id,
            tool_name,
            tool_home,
            category: cand.category,
            relative_path: cand.relative_path,
            path: cand.path,
            members: cand.members,
            bytes: cand.bytes,
            hardlinked: true,
            growth_bytes,
            regrowth_count,
            observed_at,
            mtime_max: cand.mtime_max,
            protected,
            protect_reason,
            project_link: cand.project_link,
            action: cand.action,
            note: cand.note,
        });
    }
    units.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.id.cmp(&b.id)));
    Ok(units)
}

/// Sum of every unit's bytes, for a tool/category total. Counted once
/// per unit regardless of member count (mirrors `external::total_bytes`).
pub fn total_bytes(units: &[AgentUnit]) -> u64 {
    units.iter().map(|u| u.bytes).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_default_protection_is_config_only() {
        for c in [
            AgentCategory::Sessions,
            AgentCategory::Attachments,
            AgentCategory::Checkpoints,
            AgentCategory::Caches,
            AgentCategory::Logs,
            AgentCategory::ManagedWorktrees,
            AgentCategory::Plugins,
            AgentCategory::Unclassified,
        ] {
            assert!(
                !c.default_protected(),
                "{c:?} must not be protected by category default"
            );
        }
        assert!(AgentCategory::ProtectedConfig.default_protected());
    }

    #[test]
    fn category_key_strings_never_collide_with_storage_category() {
        // Defensive: the growth store's key family is shared with
        // `external.rs`'s top-level detector locations, which use
        // `StorageCategory`'s own kebab strings; the "agent:" prefix
        // must make collision structurally impossible.
        let storage_strs = [
            "installation",
            "downloads",
            "cache",
            "local-state",
            "environments",
            "build-output",
            "models",
            "unclassified",
        ];
        for c in [
            AgentCategory::Sessions,
            AgentCategory::Attachments,
            AgentCategory::Checkpoints,
            AgentCategory::Caches,
            AgentCategory::Logs,
            AgentCategory::ManagedWorktrees,
            AgentCategory::Plugins,
            AgentCategory::ProtectedConfig,
            AgentCategory::Unclassified,
        ] {
            assert!(!storage_strs.contains(&c.key_str().as_str()));
            assert!(c.key_str().starts_with("agent:"));
        }
    }

    #[test]
    fn protect_add_list_remove_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("some-session.jsonl");
        std::fs::write(&target, b"x").unwrap();
        protect_add(dir.path(), &target).unwrap();
        let listed = protect_list(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(is_human_protected(&listed, &target));
        // Idempotent add.
        protect_add(dir.path(), &target).unwrap();
        assert_eq!(protect_list(dir.path()).unwrap().len(), 1);
        protect_remove(dir.path(), &target).unwrap();
        assert!(protect_list(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn protect_survives_reload_and_blocks_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        protect_add(dir.path(), &home).unwrap();
        let reloaded = protect_list(dir.path()).unwrap();
        let nested = home.join("projects/x/session.jsonl");
        assert!(is_human_protected(&reloaded, &nested));
    }

    #[test]
    fn folded_bytes_bounds_a_pathological_directory() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..50 {
            std::fs::write(dir.path().join(format!("f{i}")), b"12345").unwrap();
        }
        let (bytes, _mtime, truncated) = folded_bytes(dir.path(), 10);
        assert!(
            truncated,
            "must report truncation, not a silently short total"
        );
        assert!(bytes > 0);
        let (bytes_full, _, truncated_full) = folded_bytes(dir.path(), 1000);
        assert!(!truncated_full);
        assert_eq!(bytes_full, 50 * 5);
    }

    #[test]
    fn folded_bytes_handles_a_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("session.jsonl");
        std::fs::write(&f, b"0123456789").unwrap();
        let (bytes, _mtime, truncated) = folded_bytes(&f, 100);
        assert_eq!(bytes, 10);
        assert!(!truncated);
    }
}
