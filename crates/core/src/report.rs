//! Report contract: the shape every later slice fills in.
//!
//! This module defines types and rendering only. Discovery, attribution,
//! growth, and Docker joins are later slices (R2-R5); `report()` here is a
//! stub entry point that returns an empty report so the golden test can
//! exercise the real contract shape before any discovery logic exists.

use crate::entities::{Confidence, id_for};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The text renderer lives in `render.rs`; re-exported here so existing
/// `report::render_text` call sites keep working.
pub use crate::render::render_text;

/// Where a fact came from (a tool invocation, a filesystem walk, ...).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub tool: String,
}

impl Source {
    pub fn new(tool: impl Into<String>) -> Self {
        Self { tool: tool.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactKind {
    BuildOutput,
    DependencyTree,
    Git,
    /// A rebuildable tool/download cache found *inside* a checkout or
    /// worktree (e.g. `.cache`, `.parcel-cache`). A cache of the same
    /// shape found *outside* every checkout is not an artifact row at
    /// all; it becomes an `UnownedRow` with reason `SharedCache` instead.
    Cache,
    /// Bytes under a worktree that are not inside any classified
    /// artifact directory: source files, VCS-tracked content, and small
    /// housekeeping (a linked worktree's `.git` file, etc). Exactly one
    /// `Source` row per worktree.
    Source,
    DockerImage,
    DockerBuildCache,
    DockerVolume,
    Loose,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorktreeKind {
    Main,
    Linked,
    /// A main checkout that is not the first-seen main checkout of its
    /// project: another clone of the same remote, discovered under this
    /// root. Grouped into the same [`ProjectRow`] as every other
    /// checkout/worktree of that project (via normalized remote URL when
    /// one is known), listed alongside `Main` and `Linked` rows rather
    /// than becoming a second project.
    Clone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UnownedReason {
    OutsideAnyCheckout,
    OwnedByNothing,
    InconclusiveEvidence,
    /// Outside every discovered checkout/worktree and not a recognized
    /// shared cache: nothing claims this path.
    NoContainingRepo,
    /// Outside every discovered checkout/worktree, but its basename/kind
    /// is a cache shape shared across projects (`.cache`,
    /// `.cargo-registry`, `.npm`, `.pnpm-store`, a top-level `.gradle`).
    /// Counted exactly once here, never duplicated into any project.
    SharedCache,
    /// The walk could not read this path (permissions). Bytes are
    /// recorded as 0; the row exists so the count is visible instead of
    /// silently dropped.
    PermissionDenied,
    /// A Docker object (image/build-cache/volume) with no explicit join
    /// evidence: no matching compose-project label, no label whose value
    /// is a path inside a discovered worktree, and no
    /// `org.opencontainers.image.source` matching a project's git remote.
    /// Name similarity to a project is never evidence.
    DockerNoJoin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRow {
    pub kind: ArtifactKind,
    pub path: PathBuf,
    pub bytes: u64,
    /// Bytes with hardlinks deduplicated *within this row only* (a
    /// deterministic per-row figure), unlike `bytes`, where a hardlinked
    /// inode is charged to whichever row the full walk saw first. The
    /// incremental path applies `new_local - old_local` to `bytes` so a
    /// re-sized row never re-charges inodes another row already holds
    /// (#29). Zero means "not recorded"; readers fall back to `bytes`.
    #[serde(default)]
    pub local_bytes: u64,
    pub growth_bytes: Option<i64>,
    pub regrowth_count: u32,
    pub observed_at: u64,
    pub confidence: Confidence,
    pub source: Source,
    /// Set for a joined row whose evidence needed a tie-break, e.g. a
    /// `compose_ambiguous=<n>` note when several worktrees of one project
    /// contain a compose file naming the same project.
    #[serde(default)]
    pub note: Option<String>,
    /// Docker detail (#33), set only for a joined Docker row. See the
    /// matching fields on [`UnownedRow`].
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub containers: Vec<String>,
    #[serde(default)]
    pub shared_with: Vec<String>,
    #[serde(default)]
    pub dangling: bool,
}

/// R4c: one directory's rollup inside a worktree's `Source` tree.
/// Produced only for directories not inside a folded artifact (see
/// `ArtifactKind`) and not `.git` -- both are already excluded because
/// `walk::attribute_parallel` folds them into one `ArtifactRow` before
/// ever recursing, so no `DirRollup` is ever emitted underneath them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirRollup {
    pub worktree_id: String,
    /// Relative to the worktree root. The worktree root itself is `""`.
    pub rel_path: String,
    /// `None` only for the worktree root's own row.
    pub parent_rel_path: Option<String>,
    /// This directory's own files plus every descendant Source
    /// directory's `allocated_total` (bottom-up sum). Does not include
    /// bytes folded into an artifact row directly beneath it.
    pub allocated_total: u64,
    /// Allocated bytes of the files directly inside this directory only.
    pub own_allocated: u64,
    pub file_count: u32,
    /// Files + subdirectories + symlinks directly inside this directory.
    pub entry_count: u32,
    pub symlink_count: u32,
    /// Newest mtime among this directory's own direct entries, in
    /// minutes since the Unix epoch.
    pub mod_time_min: i32,
    pub complete: bool,
    #[serde(default)]
    pub growth_bytes: Option<i64>,
}

/// R4c: one large file (>= `large_file_min_bytes`) found under a
/// worktree's `Source` tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRow {
    pub worktree_id: String,
    /// Relative to the worktree root.
    pub rel_path: String,
    pub allocated: u64,
    pub mod_time_min: i32,
    #[serde(default)]
    pub growth_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRow {
    pub worktree_id: String,
    pub path: PathBuf,
    pub kind: WorktreeKind,
    pub artifacts: Vec<ArtifactRow>,
    pub signals: Vec<Signal>,
    /// Current branch (`None` for a detached HEAD). Used for GitHub
    /// enrichment and shown in the `--view worktrees` printer.
    #[serde(default)]
    pub branch: Option<String>,
    /// GitHub facts for `branch`, when the remote is on github.com and
    /// enrichment ran. `None` when the remote isn't GitHub (never
    /// queried; not the same as `Unknown`, which means GitHub was asked
    /// and couldn't answer).
    #[serde(default)]
    pub github: Option<crate::github::GithubFacts>,
    /// Composite `merge-complete` fact -- always emitted with its terms.
    /// `None` when `github` is `None` (nothing to compose from).
    #[serde(default)]
    pub merge_complete: Option<crate::github::MergeComplete>,
    /// `now - max(last commit time, newest mtime observed in Source)`,
    /// in seconds. `None` when neither could be established (no commits
    /// and nothing readable under the worktree).
    #[serde(default)]
    pub idle_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRow {
    pub project_id: String,
    pub name: String,
    pub worktrees: Vec<WorktreeRow>,
    /// Normalized `origin` remote URL shared by this project's
    /// checkouts, when at least one of them has one configured. `None`
    /// when no discovered checkout/worktree of this project has an
    /// `origin` remote (identity then falls back to object-store id).
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnownedRow {
    pub path_or_object: String,
    pub bytes: u64,
    pub reason: UnownedReason,
    /// Set for a `DockerNoJoin` image/volume: the object's `SharedSize`,
    /// shown alongside `bytes` (its `UniqueSize`) so the row's total
    /// footprint is visible even though only the unique bytes count.
    #[serde(default)]
    pub shared_bytes: Option<u64>,
    /// Set when an `org.opencontainers.image.source` label was read but
    /// matched no discovered project's git remote: the base image's
    /// source repo, recorded for visibility, never used to attribute.
    #[serde(default)]
    pub note: Option<String>,
    /// Set for a `DockerNoJoin` row: which Docker object kind it is
    /// (`"image"`, `"build-cache"`, `"volume"`), so the renderer can
    /// fold every unjoined Docker object into one summary line per kind
    /// instead of one line per object (a real `~/src` scan can have
    /// hundreds of unjoined build-cache entries).
    #[serde(default)]
    pub docker_kind: Option<String>,
    /// Docker detail (#33), set only for a Docker row: creation
    /// timestamp (image `CreatedAt`/volume inspect `CreatedAt`).
    #[serde(default)]
    pub created_at: Option<String>,
    /// Containers referencing this image/volume, formatted `"name
    /// (state[, finished <finished_at>])"`. Facts only, never a verdict
    /// about whether the object is safe to remove.
    #[serde(default)]
    pub containers: Vec<String>,
    /// Other image references sharing >=1 layer digest with this image;
    /// empty for volumes/build-cache.
    #[serde(default)]
    pub shared_with: Vec<String>,
    /// True for an image with no `RepoTags` at all.
    #[serde(default)]
    pub dangling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation {
    pub attributed: u64,
    pub unowned: u64,
    pub walked_total: u64,
    pub du_total: Option<u64>,
    /// Bytes of Docker objects joined to a project/worktree. Tracked
    /// separately from `attributed`/`walked_total`: Docker objects are
    /// not on the walked filesystem.
    pub docker_attributed: u64,
    /// Bytes of Docker objects with no join evidence (`DockerNoJoin`).
    /// Tracked separately from `unowned`/`walked_total` for the same
    /// reason.
    pub docker_unowned: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub observed_at: u64,
    pub root: PathBuf,
    pub projects: Vec<ProjectRow>,
    pub unowned: Vec<UnownedRow>,
    pub reconciliation: Reconciliation,
    /// Coverage notes that are not per-row facts, e.g. "docker:
    /// unavailable (...)" when the daemon could not be reached.
    #[serde(default)]
    pub notes: Vec<String>,
    /// R4c: per-worktree directory drill-down, populated only when the
    /// caller asked for it (`report_with(.., include_dirs: true)`) so the
    /// default report payload stays small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirs_by_worktree: Option<std::collections::HashMap<String, Vec<DirRollup>>>,
    /// R4c: per-worktree large-file rows, same opt-in as `dirs_by_worktree`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_by_worktree: Option<std::collections::HashMap<String, Vec<FileRow>>>,
    /// Scheduled-observation status line for the report header (item 5 of
    /// #31): "last scheduled run 12m ago (full, 4.2 s)" when a schedule
    /// exists, or a "no schedule (...)" suggestion when it does not. Only
    /// populated when a store directory was supplied.
    #[serde(default)]
    pub schedule_line: Option<String>,
    /// Set only when this call ran live GitHub enrichment (`enrich:
    /// true` -- `slop-livin observe`'s full walk, or `report --enrich`).
    /// `None` for a plain `report` call, which reads `enrich.parquet`
    /// as-is and never shells out to `gh`.
    #[serde(default)]
    pub github_enrichment: Option<GithubEnrichmentSummary>,
}

/// Live GitHub enrichment stats for one `report_full(.., enrich: true)`
/// call: how many `gh api graphql` calls it made (one per `(owner,
/// repo)` that needed a refresh) and how long that took, independent of
/// the rest of the report's wall time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GithubEnrichmentSummary {
    pub calls_made: u32,
    pub worktrees_enriched: u32,
    pub elapsed_secs: f64,
}

/// R2 discovers projects (checkouts and linked worktrees) under `root`
/// and fills `ProjectRow`/`WorktreeRow`. R3 then classifies artifact
/// directories at discovery, attributes each to its nearest containing
/// worktree, and folds the rest of every worktree into one `Source` row.
/// Growth and full Docker joins remain later slices (R4-R5); the one
/// Docker fact this pass understands is "no compose-project label
/// matches a discovered project name", which lands as an `UnownedRow`
/// with reason `OwnedByNothing` rather than being attributed anywhere.
pub fn report(root: &Path, docker_facts: Option<&Path>) -> Result<Report> {
    report_with(root, docker_facts, false, None, None)
}

/// Same as [`report`], optionally running `du -skPx` on the root as an
/// independent oracle for `reconciliation.du_total`, and optionally
/// observing into the reverse-delta growth store (`growth.rs`).
///
/// `store_dir` is the top-level `${SLOP_LIVIN_DIR}`-style directory; when
/// `None`, nothing is persisted and every `growth_bytes`/`regrowth_count`
/// stays at the R3 default (`None`/`0`) -- a read-only report, which is
/// what the golden test and `--no-observe` want. `since_override`
/// overrides the store's configured `since` setting for this call only
/// (`--since`). Kept at its existing 5-argument shape so callers outside
/// this slice (the MCP surface) do not need to change; use
/// [`report_with_dirs`] for the R4c `--dirs` opt-in.
pub fn report_with(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
) -> Result<Report> {
    report_full(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        true,
        false,
        false,
    )
}

/// Same as [`report_with`], with explicit control over whether this call
/// *persists* a new observation into the growth store (`observe = true`,
/// the CLI/MCP default) or only *reads* it (`observe = false`, `--no-observe`).
///
/// A read-only call still computes `growth_bytes`/`regrowth_count` from
/// whatever history the store already has for each row: growth is a
/// property of the store, not of whether this particular call wrote to
/// it. Only a store with no prior observation of a row leaves that row's
/// growth at `None`, exactly as it would immediately after `observe:
/// true`'s own first-ever observation.
pub fn report_with_observe(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    observe: bool,
) -> Result<Report> {
    report_full(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        observe,
        false,
        false,
    )
}

/// Same as [`report_with`], with `include_dirs` (R4c) additionally
/// populating `dirs_by_worktree` / `files_by_worktree` on the returned
/// report when `true`. When `false` they stay `None` so the default
/// report payload stays small. Always observes (persists), like
/// `report_with`; combine with [`report_with_observe`]'s `observe: bool`
/// through [`report_full`] directly if a caller ever needs both knobs at
/// once (none currently does).
pub fn report_with_dirs(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    include_dirs: bool,
) -> Result<Report> {
    report_full(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        true,
        include_dirs,
        false,
    )
}

/// Same as [`report_with`], with an explicit `enrich` flag: when `true`
/// (`report --enrich`), GitHub facts are refreshed live (same
/// concurrent, coalesced `observe_all` path `slop-livin observe` uses)
/// before being read back. When `false` (the default for every other
/// caller, including `report_with`), GitHub facts come **only** from
/// `enrich.parquet` -- this call never shells out to `gh`. See the
/// module doc on `github.rs` for why report and observe are split.
pub fn report_with_enrich(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    observe: bool,
    enrich: bool,
) -> Result<Report> {
    report_full(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        observe,
        false,
        enrich,
    )
}

/// The actual implementation behind [`report_with`], [`report_with_observe`],
/// [`report_with_dirs`], and [`report_with_enrich`]: `observe` controls
/// whether this call persists a new observation into the growth store or
/// only reads it, `include_dirs` (R4c) controls whether
/// `dirs_by_worktree`/`files_by_worktree` are populated on the returned
/// report, and `enrich` (#35) controls whether GitHub facts are
/// refreshed live or read as-is from `enrich.parquet`. `pub` (rather
/// than the other four's convenience wrapper shape) because the CLI's
/// `--dirs`/`--no-observe`/`--enrich` are independent flags and a caller
/// may need more than one at once.
///
/// Always takes the full-walk path (never tries FSEvents); see
/// [`report_full_mode`] for the `--full`-aware entry point R4b (#29) adds.
#[allow(clippy::too_many_arguments)]
pub fn report_full(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    observe: bool,
    include_dirs: bool,
    enrich: bool,
) -> Result<Report> {
    report_full_mode(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        observe,
        include_dirs,
        enrich,
        true,
    )
}

/// Same as [`report_full`], with one more knob: `force_full` (`--full`)
/// skips the FSEvents-driven incremental attempt entirely, same as a
/// refusal would, and still re-anchors the stored event id for the next
/// call. When `store_dir` is `None` this is identical to `report_full`
/// (no store, no FSEvents replay, no incremental path is possible).
#[allow(clippy::too_many_arguments)]
pub fn report_full_mode(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    observe: bool,
    include_dirs: bool,
    enrich: bool,
    force_full: bool,
) -> Result<Report> {
    report_full_mode_with_source(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        observe,
        include_dirs,
        enrich,
        force_full,
        crate::fs_events::platform_source().as_ref(),
    )
}

/// Same as [`report_full_mode`], with the [`crate::fs_events::FsEventsSource`]
/// supplied explicitly. Exposed so integration tests can exercise the
/// FSEvents-driven incremental path end to end (full `Report`, docker
/// join, signals, and all) with canned event batches instead of the live
/// `fseventsd`; production callers use [`report_full_mode`], which
/// resolves the real platform source.
#[allow(clippy::too_many_arguments)]
pub fn report_full_mode_with_source(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    observe: bool,
    include_dirs: bool,
    enrich: bool,
    force_full: bool,
    fs_events_source: &dyn crate::fs_events::FsEventsSource,
) -> Result<Report> {
    let trace = std::env::var("SLOP_LIVIN_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
    let observed_at = crate::entities::now();

    let large_file_min_bytes = store_dir
        .map(|dir| crate::growth::load_config(dir).large_file_min_bytes)
        .unwrap_or(crate::growth::DEFAULT_LARGE_FILE_MIN_BYTES);

    // Discovery and attribution are each still their own recursive pass
    // over the tree; `walk::discover_and_attribute` runs both over a
    // bounded thread pool instead of one directory at a time per pass.
    // See its module docs for why the two walks are kept separate rather
    // than fused into one (they stop recursion on different directories).
    //
    // R4b (#29): when a growth store is available, try an FSEvents-driven
    // incremental re-walk first (`growth::observe_tracked_with_source`);
    // it falls back to the same `discover_and_attribute` full walk on any
    // refusal. The mode/reason it reports lands in `notes` as
    // `"fsevents: mode=.. reason=.. changed_dirs=.."`, which is both this
    // report's coverage line and (parsed back out by `observe_only`) the
    // `observe` log line's `mode=`/`reason=` fields.
    let t0 = std::time::Instant::now();
    let mut notes: Vec<String> = Vec::new();
    let (discovered, mut attribution) = if let Some(dir) = store_dir {
        let tracked = crate::growth::observe_tracked_with_source(
            dir,
            root,
            observed_at,
            large_file_min_bytes,
            force_full,
            observe,
            fs_events_source,
        )?;
        notes.push(format!(
            "fsevents: mode={} reason={} changed_dirs={}",
            tracked.mode, tracked.reason, tracked.changed_dirs
        ));
        (tracked.discovered, tracked.attribution)
    } else {
        notes.push("fsevents: mode=full reason=no_store changed_dirs=0".to_string());
        crate::walk::discover_and_attribute(root, observed_at, large_file_min_bytes)?
    };
    if trace {
        eprintln!("[trace] discover_and_attribute: {:?}", t0.elapsed());
    }
    let mut dirs = std::mem::take(&mut attribution.dirs);
    let mut files = std::mem::take(&mut attribution.files);

    // Group discovered checkouts/worktrees by project identity. When a
    // checkout's `origin` remote is known, identity is the normalized
    // remote URL: two separate clones of the same repo (each its own
    // object store, its own `git::discover`-assigned `project_id`) are
    // the same project, and every main checkout after the first-seen one
    // (by path order, for determinism across the parallel walk) is
    // demoted from `Main` to `Clone` rather than starting a second
    // project row. A checkout with no remote configured falls back to
    // its object-store identity, exactly as before -- unrelated
    // checkouts never collide just because their remote is empty.
    //
    // `group_key` -> discovered rows sharing it, sorted by path so the
    // first-seen main checkout is deterministic across the parallel walk.
    let mut groups: BTreeMap<String, Vec<crate::git::DiscoveredWorktree>> = BTreeMap::new();
    // group_key -> (display name, is a Main row seen).
    for dw in discovered {
        let group_key = match dw.remote_url.as_deref().and_then(normalize_remote) {
            Some(remote) => format!("remote:{remote}"),
            None => format!("store:{}", dw.project_id),
        };
        groups.entry(group_key).or_default().push(dw);
    }

    let mut projects: Vec<ProjectRow> = Vec::new();
    let mut worktree_paths: Vec<(PathBuf, String)> = Vec::new();
    // project_id -> normalized remote URL, first one seen for that project.
    let mut project_remotes: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    // worktree_id -> raw remote URL, this worktree's own (never
    // normalized: github_owner_repo parses either form).
    let mut worktree_remotes: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (group_key, mut members) in groups {
        members.sort_by(|a, b| a.path.cmp(&b.path));
        let project_id = id_for(&group_key);
        let mut name: Option<String> = None;
        let mut worktrees: Vec<WorktreeRow> = Vec::new();
        let mut main_assigned = false;
        for dw in members {
            let worktree_id = id_for(&dw.path.display().to_string());
            worktree_paths.push((dw.path.clone(), worktree_id.clone()));
            if let Some(remote) = &dw.remote_url {
                worktree_remotes.insert(worktree_id.clone(), remote.clone());
            }
            if let Some(remote) = dw.remote_url.as_deref().and_then(normalize_remote) {
                project_remotes.entry(project_id.clone()).or_insert(remote);
            }
            let kind = match dw.kind {
                WorktreeKind::Linked => WorktreeKind::Linked,
                WorktreeKind::Main | WorktreeKind::Clone => {
                    if main_assigned {
                        WorktreeKind::Clone
                    } else {
                        main_assigned = true;
                        WorktreeKind::Main
                    }
                }
            };
            if kind == WorktreeKind::Main || name.is_none() {
                name = Some(dw.project_name.clone());
            }
            worktrees.push(WorktreeRow {
                worktree_id,
                path: dw.path,
                kind,
                artifacts: Vec::new(),
                signals: Vec::new(),
                branch: None,
                github: None,
                merge_complete: None,
                idle_secs: None,
            });
        }
        let remote = project_remotes.get(&project_id).cloned();
        projects.push(ProjectRow {
            project_id,
            name: name.unwrap_or_default(),
            worktrees,
            remote,
        });
    }
    // Keep prior output ordering stable (by name) now that projects is a
    // plain Vec instead of a BTreeMap keyed by the old per-checkout id.
    projects.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.project_id.cmp(&b.project_id))
    });

    for project in &mut projects {
        for worktree in &mut project.worktrees {
            crate::attribution::apply_to_worktree(worktree, &mut attribution);
        }
    }

    // worktree_id -> raw signal values, kept alongside the rendered
    // `Signal` rows so the merge_complete composite and `filter.rs`'s
    // `idle >` predicate can use them without re-parsing rendered
    // strings.
    let mut raw_by_worktree: std::collections::HashMap<String, crate::signals::RawSignals> =
        std::collections::HashMap::new();
    let t_signals = std::time::Instant::now();
    let signal_paths: Vec<PathBuf> = projects
        .iter()
        .flat_map(|p| p.worktrees.iter().map(|w| w.path.clone()))
        .collect();
    let mut signals =
        crate::signals::compute_signals_raw_parallel(&signal_paths, observed_at).into_iter();
    for project in &mut projects {
        for worktree in &mut project.worktrees {
            let (rows, raw) = signals.next().unwrap_or_else(|| {
                (
                    Vec::new(),
                    crate::signals::RawSignals {
                        last_commit_age_secs: None,
                        dirty: None,
                        unpushed: None,
                        locked: None,
                        idle_for_secs: None,
                    },
                )
            });
            worktree.signals = rows;
            worktree.branch = crate::github::current_branch(&worktree.path);
            worktree.idle_secs = raw.idle_for_secs;
            raw_by_worktree.insert(worktree.worktree_id.clone(), raw);
        }
    }
    if trace {
        eprintln!(
            "[trace] signals ({} worktrees): {:?}",
            signal_paths.len(),
            t_signals.elapsed()
        );
    }

    // GitHub enrichment: only for worktrees whose remote is on
    // github.com. Cached in enrich.parquet under a volume-keyed
    // directory: `store_dir` when given (same directory `growth.rs`
    // uses), otherwise the same default `${SLOP_LIVIN_DIR}` the CLI/MCP
    // resolve on their own.
    //
    // `enrich` (distinct from `observe`, which only controls the growth
    // store) decides whether this call makes live `gh` calls at all:
    // `false` (plain `report`/`--view worktrees`) reads `enrich.parquet`
    // as-is via `github::read_cached` and never shells out; `true`
    // (`slop-livin observe`'s full walk, or `report --enrich`) refreshes
    // it live via `github::observe_all` (concurrent, coalesced per
    // repo) before reading it back. See `github.rs`'s module doc for the
    // full rationale.
    let mut github_notes: Vec<String> = Vec::new();
    let mut github_enrichment: Option<GithubEnrichmentSummary> = None;
    let github_dir_owned = store_dir
        .map(PathBuf::from)
        .or_else(default_github_cache_dir);
    if let Some(dir) = github_dir_owned.as_deref() {
        let volume_id = std::fs::metadata(root)
            .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
            .unwrap_or(0);

        // (worktree_id, owner, repo, branch, tip_sha) for every worktree
        // whose remote resolves to a github.com owner/repo.
        let mut owned: Vec<(String, String, String, Option<String>, String)> = Vec::new();
        for project in &projects {
            for worktree in &project.worktrees {
                let Some(remote) = worktree_remotes.get(&worktree.worktree_id) else {
                    continue;
                };
                let Some((owner, repo)) = crate::github::github_owner_repo(remote) else {
                    continue;
                };
                let tip_sha = crate::signals::tip_sha(&worktree.path).unwrap_or_default();
                owned.push((
                    worktree.worktree_id.clone(),
                    owner,
                    repo,
                    worktree.branch.clone(),
                    tip_sha,
                ));
            }
        }
        let inputs: Vec<crate::github::EnrichInput> = owned
            .iter()
            .map(
                |(worktree_id, owner, repo, branch, tip_sha)| crate::github::EnrichInput {
                    worktree_id,
                    tip_sha,
                    branch: branch.as_deref(),
                    owner,
                    repo,
                },
            )
            .collect();

        let t_github = std::time::Instant::now();
        let (facts_by_worktree, gh_notes) = if enrich {
            let responder = crate::github::GhCliResponder;
            let summary = crate::github::observe_all(
                &responder,
                dir,
                volume_id,
                &inputs,
                observed_at,
                crate::github::DEFAULT_GITHUB_TTL_SECS,
                crate::github::DEFAULT_RUN_BUDGET_SECS,
                crate::github::DEFAULT_CONCURRENCY,
            );
            let (facts, mut read_notes) = crate::github::read_cached(
                dir,
                volume_id,
                &inputs,
                observed_at,
                crate::github::DEFAULT_GITHUB_TTL_SECS,
            );
            read_notes.retain(|n| !n.contains("not enriched") && !n.contains("stale cached"));
            read_notes.extend(summary.notes);
            github_enrichment = Some(GithubEnrichmentSummary {
                calls_made: summary.calls_made,
                worktrees_enriched: summary.worktrees_enriched,
                elapsed_secs: t_github.elapsed().as_secs_f64(),
            });
            (facts, read_notes)
        } else {
            // Never shells out: reads whatever `enrich.parquet` already
            // has, marking stale/missing entries `Unknown` with a note
            // pointing at `slop-livin observe`.
            crate::github::read_cached(
                dir,
                volume_id,
                &inputs,
                observed_at,
                crate::github::DEFAULT_GITHUB_TTL_SECS,
            )
        };
        for project in &mut projects {
            for worktree in &mut project.worktrees {
                if let Some(facts) = facts_by_worktree.get(&worktree.worktree_id) {
                    let raw = raw_by_worktree.get(&worktree.worktree_id);
                    let mc = crate::github::merge_complete(
                        raw.and_then(|r| r.dirty),
                        raw.and_then(|r| r.unpushed),
                        &facts.merged,
                    );
                    worktree.signals.push(Signal {
                        name: "merge_complete".to_string(),
                        value: format!(
                            "{} ({})",
                            match mc.verdict {
                                crate::github::TriState::Yes => "yes",
                                crate::github::TriState::No => "no",
                                crate::github::TriState::Unknown => "unknown",
                            },
                            mc.terms.join(", ")
                        ),
                    });
                    worktree.signals.push(Signal {
                        name: "pull_request".to_string(),
                        value: render_pr_status(&facts.pull_request),
                    });
                    worktree.merge_complete = Some(mc);
                    worktree.github = Some(facts.clone());
                }
            }
        }
        github_notes = gh_notes;
    }

    let mut unowned = attribution.unowned;
    notes.extend(github_notes);
    let facts = crate::docker::load(docker_facts);
    if let Some(reason) = &facts.unavailable {
        notes.push(reason.clone());
    }
    // compose_name -> worktree_ids whose worktree contains a compose file
    // naming it (via its top-level `name:` or the file's directory
    // basename). Built once per report so `join_one` can match a
    // `com.docker.compose.project` label that names the compose project
    // rather than the discovered project/repo name (#28).
    let mut compose_index: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (path, worktree_id) in &worktree_paths {
        for name in crate::compose::discover_candidate_names(path) {
            compose_index
                .entry(name)
                .or_default()
                .push(worktree_id.clone());
        }
    }

    let join = join_docker_facts(
        &facts,
        &projects,
        &worktree_paths,
        &project_remotes,
        &compose_index,
    );
    for (worktree_id, mut rows) in join.rows_by_worktree {
        for project in &mut projects {
            for worktree in &mut project.worktrees {
                if worktree.worktree_id == worktree_id {
                    worktree.artifacts.append(&mut rows);
                    break;
                }
            }
        }
    }
    unowned.extend(join.unowned);
    let docker_attributed_bytes = join.attributed_bytes;
    let docker_unowned_bytes = join.unowned_bytes;

    let du = if verify_du {
        crate::attribution::du_total(root)
    } else {
        None
    };

    // R4c: `own_allocated` is set by the walk; roll it up into
    // `allocated_total` bottom-up (deepest directories first) before
    // persisting/annotating, so a parent's total reflects every Source
    // descendant without needing the walk itself to wait on children.
    aggregate_dir_totals(&mut dirs);

    let mut schedule_line = None;
    if let Some(dir) = store_dir {
        let volume_id = std::fs::metadata(root)
            .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
            .unwrap_or(0);
        let config = crate::growth::load_config(dir);
        let since_secs = since_override
            .and_then(crate::growth::parse_duration_secs)
            .or_else(|| crate::growth::parse_duration_secs(&config.since))
            .unwrap_or(24 * 3600);
        if observe {
            crate::growth::observe_and_annotate(
                dir,
                volume_id,
                &mut projects,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
            crate::growth::observe_and_annotate_dirs(
                dir,
                volume_id,
                &mut dirs,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
            crate::growth::observe_and_annotate_files(
                dir,
                volume_id,
                &mut files,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
        } else {
            crate::growth::annotate_readonly(
                dir,
                volume_id,
                &mut projects,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
            crate::growth::annotate_readonly_dirs(
                dir,
                volume_id,
                &mut dirs,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
            crate::growth::annotate_readonly_files(
                dir,
                volume_id,
                &mut files,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
        }
        schedule_line = Some(crate::schedule::header_line(dir, root, observed_at));
    }

    let (dirs_by_worktree, files_by_worktree) = if include_dirs {
        let mut by_dir: std::collections::HashMap<String, Vec<DirRollup>> =
            std::collections::HashMap::new();
        for d in dirs {
            by_dir.entry(d.worktree_id.clone()).or_default().push(d);
        }
        let mut by_file: std::collections::HashMap<String, Vec<FileRow>> =
            std::collections::HashMap::new();
        for f in files {
            by_file.entry(f.worktree_id.clone()).or_default().push(f);
        }
        (Some(by_dir), Some(by_file))
    } else {
        (None, None)
    };

    Ok(Report {
        observed_at,
        root: root.to_path_buf(),
        projects,
        unowned,
        reconciliation: Reconciliation {
            attributed: attribution.attributed_total,
            unowned: attribution.unowned_total,
            walked_total: attribution.walked_total,
            du_total: du,
            docker_attributed: docker_attributed_bytes,
            docker_unowned: docker_unowned_bytes,
        },
        notes,
        dirs_by_worktree,
        files_by_worktree,
        schedule_line,
        github_enrichment,
    })
}

/// Rolls `own_allocated` up into `allocated_total` bottom-up: deepest
/// directories (most path separators) are folded into their parent's
/// running total first, so a parent's `allocated_total` always includes
/// every descendant Source directory's total by the time it is visited.
/// Deliberately excludes bytes folded into a classified artifact row
/// directly beneath a directory -- those stay one `ArtifactRow`, never
/// decomposed into `DirRollup`s, per the folding contract this issue
/// requires.
fn aggregate_dir_totals(dirs: &mut [DirRollup]) {
    let mut totals: std::collections::HashMap<(String, String), u64> =
        std::collections::HashMap::new();
    for d in dirs.iter() {
        totals.insert((d.worktree_id.clone(), d.rel_path.clone()), d.own_allocated);
    }
    let mut order: Vec<usize> = (0..dirs.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(dirs[i].rel_path.matches('/').count()));
    for &i in &order {
        let key = (dirs[i].worktree_id.clone(), dirs[i].rel_path.clone());
        let value = *totals.get(&key).unwrap_or(&0);
        if let Some(parent_rel) = dirs[i].parent_rel_path.clone() {
            let pkey = (dirs[i].worktree_id.clone(), parent_rel);
            *totals.entry(pkey).or_insert(0) += value;
        }
    }
    for d in dirs.iter_mut() {
        let key = (d.worktree_id.clone(), d.rel_path.clone());
        d.allocated_total = *totals.get(&key).unwrap_or(&d.own_allocated);
    }
}

/// `${SLOP_LIVIN_DIR}` (or `~/.local/share/slop-livin`): the same
/// resolution the CLI and MCP server use on their own, duplicated here
/// only as a fallback for GitHub enrichment's cache when no `store_dir`
/// was supplied (see the call site in `report_with`).
fn default_github_cache_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SLOP_LIVIN_DIR") {
        return Some(PathBuf::from(dir));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".local/share/slop-livin"))
}

/// Renders a `PrStatus` for the `pull_request` signal row and the
/// `--view worktrees` printer, e.g. `PR #123 open (approved)`,
/// `PR #98 merged`, `no PR`, `unknown`.
fn render_pr_status(status: &crate::github::PrStatus) -> String {
    use crate::github::{PrState, PrStatus, ReviewDecision};
    match status {
        PrStatus::None => "no PR".to_string(),
        PrStatus::Unknown => "unknown".to_string(),
        PrStatus::Some(pr) => {
            let state = match pr.state {
                PrState::Open => "open",
                PrState::Closed => "closed",
                PrState::Merged => "merged",
            };
            let decision = match pr.review_decision {
                ReviewDecision::Approved => Some("approved"),
                ReviewDecision::ChangesRequested => Some("changes requested"),
                ReviewDecision::ReviewRequired => Some("review required"),
                ReviewDecision::None | ReviewDecision::Unknown => None,
            };
            let draft = if pr.draft { " draft" } else { "" };
            match decision {
                Some(d) => format!("PR #{}{draft} {state} ({d})", pr.number),
                None => format!("PR #{}{draft} {state}", pr.number),
            }
        }
    }
}

/// Normalizes a git remote URL for comparison: strips a trailing `.git`,
/// collapses the `git@host:path` scp-like ssh form and any `scheme://`
/// form down to `host/path`, and lowercases the result.
fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let stripped = url.strip_suffix(".git").unwrap_or(url);
    if let Some(rest) = stripped.strip_prefix("git@") {
        return Some(rest.replacen(':', "/", 1).to_lowercase());
    }
    if let Some(idx) = stripped.find("://") {
        let rest = &stripped[idx + 3..];
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        return Some(rest.to_lowercase());
    }
    Some(stripped.to_lowercase())
}

/// Result of joining Docker facts to discovered projects/worktrees.
struct DockerJoinResult {
    rows_by_worktree: std::collections::HashMap<String, Vec<ArtifactRow>>,
    unowned: Vec<UnownedRow>,
    attributed_bytes: u64,
    unowned_bytes: u64,
}

/// One candidate Docker object (image, build-cache entry, or volume)
/// being joined, in a shape common to all three.
struct JoinCandidate {
    reference: String,
    labels: std::collections::HashMap<String, String>,
    unique_bytes: u64,
    shared_bytes: u64,
    kind: ArtifactKind,
    created_at: Option<String>,
    containers: Vec<String>,
    shared_with: Vec<String>,
    dangling: bool,
}

enum JoinOutcome {
    Joined {
        worktree_id: String,
        confidence: Confidence,
        /// The join rule that matched, used as the artifact row's
        /// `Source.tool` (`docker.<rule>`) so different evidence stays
        /// distinguishable after the fact.
        rule: &'static str,
        /// Set when the rule needed a tie-break, e.g.
        /// `compose_ambiguous=<n>` when several worktrees of one project
        /// each contain a compose file naming the same project.
        note: Option<String>,
    },
    Unowned {
        note: Option<String>,
    },
}

/// Applies the R5 join rules plus the #28 compose-file-name rule,
/// explicit evidence only:
/// 1. `com.docker.compose.project` label == a discovered project name:
///    join that project's main worktree, High confidence.
/// 2. Otherwise, `com.docker.compose.project` label == a candidate name
///    (`name:` value or containing directory basename) read from a
///    compose file inside a discovered worktree: join that worktree,
///    High confidence, rule `compose_file_name`. A compose project name
///    frequently differs from the repo name (e.g. `hiphi-staging` for a
///    `hiphi-relay`/`hiphi-authorizer` checkout), which is exactly the
///    case rule 1 misses. If several worktrees of one project each
///    contain a matching compose file, a `com.docker.compose.project.working_dir`
///    label pointing at one of them breaks the tie; otherwise the
///    project's Main worktree is used and a `compose_ambiguous=<n>` note
///    is attached. If the label matches neither a discovered project
///    name nor any compose-file candidate, it is recorded as a
///    `compose_project=<name>` note instead of being silently dropped --
///    a later slice can group by it even when this slice can't
///    attribute it.
/// 3. Any label whose value is an absolute path inside a discovered
///    worktree (this covers `com.docker.compose.project.working_dir` as
///    well as any other path-shaped label): join that worktree, High
///    confidence.
/// 4. `org.opencontainers.image.source`, normalized, matches a project's
///    normalized git remote: join that project's main worktree, Medium
///    confidence. A non-matching source is recorded as a
///    `base_image_source=<url>` note, never used to attribute -- this is
///    the base-image trap: an image can be named exactly like a project
///    (or even be that project's own published image) while its
///    `image.source` legitimately points at an upstream base image
///    (e.g. `linuxcontainers/alpine`) it was built FROM, not the project
///    that built it.
/// 5. Otherwise: unowned, reason `DockerNoJoin`. Name similarity to a
///    project is never evidence for any of these rules.
fn join_one(
    candidate: &JoinCandidate,
    projects: &[ProjectRow],
    worktree_paths: &[(PathBuf, String)],
    project_remotes: &std::collections::HashMap<String, String>,
    compose_index: &std::collections::HashMap<String, Vec<String>>,
) -> JoinOutcome {
    fn main_worktree_id(project: &ProjectRow) -> Option<String> {
        project
            .worktrees
            .iter()
            .find(|w| w.kind == WorktreeKind::Main)
            .or_else(|| project.worktrees.first())
            .map(|w| w.worktree_id.clone())
    }

    fn project_for_worktree<'a>(
        projects: &'a [ProjectRow],
        worktree_id: &str,
    ) -> Option<&'a ProjectRow> {
        projects
            .iter()
            .find(|p| p.worktrees.iter().any(|w| w.worktree_id == worktree_id))
    }

    let mut notes: Vec<String> = Vec::new();

    if let Some(project_label) = candidate.labels.get("com.docker.compose.project") {
        // Rule 1: label matches a discovered project's own name.
        if let Some(worktree_id) = projects
            .iter()
            .find(|p| &p.name == project_label)
            .and_then(main_worktree_id)
        {
            return JoinOutcome::Joined {
                worktree_id,
                confidence: Confidence::High,
                rule: "compose_project_label",
                note: None,
            };
        }

        // Rule 2 (#28): label matches a compose file's `name:` value or
        // directory basename found inside a discovered worktree.
        if let Some(worktree_ids) = compose_index.get(project_label)
            && !worktree_ids.is_empty()
        {
            // The working_dir label is where compose was invoked from
            // (e.g. a `deploy/` subdirectory), not necessarily the
            // worktree root itself, so this matches by containment
            // (deepest worktree root that contains it) rather than exact
            // equality -- the same rule rule 3 below uses for path-shaped
            // labels in general.
            let working_dir = candidate
                .labels
                .get("com.docker.compose.project.working_dir")
                .map(Path::new);
            let tie_broken = working_dir.and_then(|wd| {
                worktree_paths
                    .iter()
                    .filter(|(p, pid)| wd.starts_with(p) && worktree_ids.contains(pid))
                    .max_by_key(|(p, _)| p.as_os_str().len())
                    .map(|(_, pid)| pid)
            });

            if let Some(worktree_id) = tie_broken {
                return JoinOutcome::Joined {
                    worktree_id: worktree_id.clone(),
                    confidence: Confidence::High,
                    rule: "compose_file_name",
                    note: None,
                };
            }

            let mut unique_ids: Vec<&String> = worktree_ids.iter().collect();
            unique_ids.sort();
            unique_ids.dedup();

            if let [only] = unique_ids.as_slice() {
                return JoinOutcome::Joined {
                    worktree_id: (*only).clone(),
                    confidence: Confidence::High,
                    rule: "compose_file_name",
                    note: None,
                };
            }

            if let Some(main_worktree_id) = unique_ids
                .first()
                .and_then(|id| project_for_worktree(projects, id))
                .and_then(main_worktree_id)
            {
                return JoinOutcome::Joined {
                    worktree_id: main_worktree_id,
                    confidence: Confidence::High,
                    rule: "compose_file_name",
                    note: Some(format!("compose_ambiguous={}", unique_ids.len())),
                };
            }
        }

        notes.push(format!("compose_project={project_label}"));
    }

    for value in candidate.labels.values() {
        if !value.starts_with('/') {
            continue;
        }
        let value_path = Path::new(value);
        if let Some((_, worktree_id)) = worktree_paths
            .iter()
            .filter(|(p, _)| value_path.starts_with(p))
            .max_by_key(|(p, _)| p.as_os_str().len())
        {
            return JoinOutcome::Joined {
                worktree_id: worktree_id.clone(),
                confidence: Confidence::High,
                rule: "working_dir_path",
                note: None,
            };
        }
    }

    if let Some(source) = candidate.labels.get("org.opencontainers.image.source") {
        let joined = normalize_remote(source).and_then(|normalized| {
            project_remotes
                .iter()
                .find(|(_, r)| **r == normalized)
                .and_then(|(project_id, _)| projects.iter().find(|p| &p.project_id == project_id))
                .and_then(main_worktree_id)
        });
        match joined {
            Some(worktree_id) => {
                return JoinOutcome::Joined {
                    worktree_id,
                    confidence: Confidence::Medium,
                    rule: "image_source",
                    note: None,
                };
            }
            None => notes.push(format!("base_image_source={source}")),
        }
    }

    let note = if notes.is_empty() {
        None
    } else {
        Some(notes.join(", "))
    };
    JoinOutcome::Unowned { note }
}

fn join_docker_facts(
    facts: &crate::docker::DockerFacts,
    projects: &[ProjectRow],
    worktree_paths: &[(PathBuf, String)],
    project_remotes: &std::collections::HashMap<String, String>,
    compose_index: &std::collections::HashMap<String, Vec<String>>,
) -> DockerJoinResult {
    let mut result = DockerJoinResult {
        rows_by_worktree: std::collections::HashMap::new(),
        unowned: Vec::new(),
        attributed_bytes: 0,
        unowned_bytes: 0,
    };

    let mut cache_notes: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    fn format_container(c: &crate::docker::ContainerRef) -> String {
        match &c.finished_at {
            Some(finished) => format!("{} ({}, finished {finished})", c.name, c.state),
            None => format!("{} ({})", c.name, c.state),
        }
    }

    let mut candidates: Vec<JoinCandidate> = Vec::new();
    for image in &facts.images {
        candidates.push(JoinCandidate {
            reference: image
                .repo_tags
                .first()
                .cloned()
                .unwrap_or_else(|| image.id.clone()),
            labels: image.labels.clone(),
            unique_bytes: image.unique_bytes,
            shared_bytes: image.shared_bytes,
            kind: ArtifactKind::DockerImage,
            created_at: image.created_at.clone(),
            containers: image.containers.iter().map(format_container).collect(),
            shared_with: image.shared_with.clone(),
            dangling: image.dangling,
        });
    }
    for cache in &facts.build_cache {
        let mut cache_note_bits = Vec::new();
        if let Some(last_used) = &cache.last_used {
            cache_note_bits.push(format!("last_used={last_used}"));
        }
        if let Some(count) = cache.usage_count {
            cache_note_bits.push(format!("usage_count={count}"));
        }
        cache_note_bits.push(format!("in_use={}", cache.in_use));
        cache_note_bits.push(format!("shared={}", cache.shared));
        candidates.push(JoinCandidate {
            reference: cache.id.clone(),
            labels: std::collections::HashMap::new(),
            unique_bytes: cache.bytes,
            shared_bytes: 0,
            kind: ArtifactKind::DockerBuildCache,
            created_at: None,
            containers: Vec::new(),
            shared_with: Vec::new(),
            dangling: false,
        });
        // Build-cache detail has no per-object join label to carry it on,
        // so it rides along as a note on the candidate's eventual row
        // instead of a dedicated field -- see the push sites below, which
        // attach `cache_note_bits` via a side table keyed by reference.
        cache_notes.insert(cache.id.clone(), cache_note_bits.join(", "));
    }
    for volume in &facts.volumes {
        candidates.push(JoinCandidate {
            reference: volume.name.clone(),
            labels: volume.labels.clone(),
            unique_bytes: volume.bytes,
            shared_bytes: 0,
            kind: ArtifactKind::DockerVolume,
            created_at: volume.created_at.clone(),
            containers: volume.containers.iter().map(format_container).collect(),
            shared_with: Vec::new(),
            dangling: false,
        });
    }

    let observed_at = crate::entities::now();
    for candidate in candidates {
        match join_one(
            &candidate,
            projects,
            worktree_paths,
            project_remotes,
            compose_index,
        ) {
            JoinOutcome::Joined {
                worktree_id,
                confidence,
                rule,
                note,
            } => {
                result.attributed_bytes += candidate.unique_bytes;
                let source = if rule == "compose_file_name" {
                    Source::new(format!("docker.{rule}"))
                } else {
                    Source::new("docker.system_df")
                };
                let note = match (note, cache_notes.get(&candidate.reference)) {
                    (Some(n), Some(cache_note)) => Some(format!("{n}, {cache_note}")),
                    (Some(n), None) => Some(n),
                    (None, Some(cache_note)) => Some(cache_note.clone()),
                    (None, None) => None,
                };
                result
                    .rows_by_worktree
                    .entry(worktree_id)
                    .or_default()
                    .push(ArtifactRow {
                        kind: candidate.kind,
                        path: PathBuf::from(candidate.reference),
                        bytes: candidate.unique_bytes,
                        local_bytes: 0,
                        growth_bytes: None,
                        regrowth_count: 0,
                        observed_at,
                        confidence,
                        source,
                        note,
                        created_at: candidate.created_at,
                        containers: candidate.containers,
                        shared_with: candidate.shared_with,
                        dangling: candidate.dangling,
                    });
            }
            JoinOutcome::Unowned { note } => {
                let docker_kind = match candidate.kind {
                    ArtifactKind::DockerImage => "image",
                    ArtifactKind::DockerBuildCache => "build-cache",
                    ArtifactKind::DockerVolume => "volume",
                    _ => "unknown",
                };
                let note = match (note, cache_notes.get(&candidate.reference)) {
                    (Some(n), Some(cache_note)) => Some(format!("{n}, {cache_note}")),
                    (Some(n), None) => Some(n),
                    (None, Some(cache_note)) => Some(cache_note.clone()),
                    (None, None) => None,
                };
                result.unowned_bytes += candidate.unique_bytes;
                result.unowned.push(UnownedRow {
                    path_or_object: candidate.reference,
                    bytes: candidate.unique_bytes,
                    reason: UnownedReason::DockerNoJoin,
                    shared_bytes: Some(candidate.shared_bytes),
                    note,
                    docker_kind: Some(docker_kind.to_string()),
                    created_at: candidate.created_at,
                    containers: candidate.containers,
                    shared_with: candidate.shared_with,
                    dangling: candidate.dangling,
                });
            }
        }
    }

    result
}

/// One root's outcome from an observe-only pass: walk + growth-store
/// write, no rendering. See [`observe_only`].
#[derive(Debug, Clone)]
pub struct ObserveSummary {
    pub observed_at: u64,
    pub walked_total: u64,
    pub projects: usize,
    /// "full" or "incremental" (#29): read straight off the
    /// `"fsevents: mode=.. reason=.. changed_dirs=.."` note this same
    /// pass records.
    pub mode: String,
    /// The `mode=.. reason=.. changed_dirs=..` line for the `observe` log
    /// (`schedule::RunOutcome`) and stdout, straight from that note.
    pub fsevents_line: String,
    /// Live GitHub enrichment stats for this same pass (#35): `observe`
    /// refreshes both the growth store and `enrich.parquet` in one walk,
    /// since it already has every worktree's path/branch/tip in hand.
    pub github: GithubEnrichmentSummary,
}

/// Observe-only entry point for `slop-livin observe`: walks `root`,
/// writes the growth store under `store_dir`, refreshes GitHub
/// enrichment live for every GitHub-remote worktree found (concurrent,
/// coalesced per repo -- see `github::observe_all`), and returns the
/// summary facts the caller prints/logs. Never renders a report.
/// `force_full` (`--full`) skips the FSEvents-driven incremental attempt.
pub fn observe_only(
    root: &Path,
    store_dir: &Path,
    since_override: Option<&str>,
    force_full: bool,
) -> Result<ObserveSummary> {
    let r = report_full_mode_with_source(
        root,
        None,
        false,
        Some(store_dir),
        since_override,
        true,
        false,
        true,
        force_full,
        crate::fs_events::platform_source().as_ref(),
    )?;
    let fsevents_line = r
        .notes
        .iter()
        .find_map(|n| n.strip_prefix("fsevents: "))
        .map(str::to_string)
        .unwrap_or_else(|| "mode=full reason=no_store changed_dirs=0".to_string());
    let mode = fsevents_line
        .strip_prefix("mode=")
        .and_then(|s| s.split(' ').next())
        .unwrap_or("full")
        .to_string();
    Ok(ObserveSummary {
        observed_at: r.observed_at,
        walked_total: r.reconciliation.walked_total,
        projects: r.projects.len(),
        mode,
        fsevents_line,
        github: r.github_enrichment.unwrap_or_default(),
    })
}

pub fn to_json(report: &Report) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}
