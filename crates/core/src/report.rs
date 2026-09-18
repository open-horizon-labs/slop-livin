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
/// (`--since`).
pub fn report_with(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
) -> Result<Report> {
    report_with_observe(
        root,
        docker_facts,
        verify_du,
        store_dir,
        since_override,
        true,
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
    let trace = std::env::var("SLOP_LIVIN_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
    let observed_at = crate::entities::now();

    // Discovery and attribution are each still their own recursive pass
    // over the tree; `walk::discover_and_attribute` runs both over a
    // bounded thread pool instead of one directory at a time per pass.
    // See its module docs for why the two walks are kept separate rather
    // than fused into one (they stop recursion on different directories).
    let t0 = std::time::Instant::now();
    let (discovered, mut attribution) = crate::walk::discover_and_attribute(root, observed_at)?;
    if trace {
        eprintln!("[trace] discover_and_attribute: {:?}", t0.elapsed());
    }

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
    for (group_key, mut members) in groups {
        members.sort_by(|a, b| a.path.cmp(&b.path));
        let project_id = id_for(&group_key);
        let mut name: Option<String> = None;
        let mut worktrees: Vec<WorktreeRow> = Vec::new();
        let mut main_assigned = false;
        for dw in members {
            let worktree_id = id_for(&dw.path.display().to_string());
            worktree_paths.push((dw.path.clone(), worktree_id.clone()));
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

    let t_signals = std::time::Instant::now();
    let signal_paths: Vec<PathBuf> = projects
        .iter()
        .flat_map(|p| p.worktrees.iter().map(|w| w.path.clone()))
        .collect();
    let mut signals =
        crate::signals::compute_signals_parallel(&signal_paths, observed_at).into_iter();
    for project in &mut projects {
        for worktree in &mut project.worktrees {
            worktree.signals = signals.next().unwrap_or_default();
        }
    }
    if trace {
        eprintln!(
            "[trace] signals ({} worktrees): {:?}",
            signal_paths.len(),
            t_signals.elapsed()
        );
    }

    let mut unowned = attribution.unowned;
    let mut notes: Vec<String> = Vec::new();
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
        } else {
            crate::growth::annotate_readonly(
                dir,
                volume_id,
                &mut projects,
                observed_at,
                config.retention_days,
                since_secs,
            )?;
        }
    }

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
    })
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
        });
    }
    for cache in &facts.build_cache {
        candidates.push(JoinCandidate {
            reference: cache.id.clone(),
            labels: std::collections::HashMap::new(),
            unique_bytes: cache.bytes,
            shared_bytes: 0,
            kind: ArtifactKind::DockerBuildCache,
        });
    }
    for volume in &facts.volumes {
        candidates.push(JoinCandidate {
            reference: volume.name.clone(),
            labels: volume.labels.clone(),
            unique_bytes: volume.bytes,
            shared_bytes: 0,
            kind: ArtifactKind::DockerVolume,
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
                result
                    .rows_by_worktree
                    .entry(worktree_id)
                    .or_default()
                    .push(ArtifactRow {
                        kind: candidate.kind,
                        path: PathBuf::from(candidate.reference),
                        bytes: candidate.unique_bytes,
                        growth_bytes: None,
                        regrowth_count: 0,
                        observed_at,
                        confidence,
                        source,
                        note,
                    });
            }
            JoinOutcome::Unowned { note } => {
                let docker_kind = match candidate.kind {
                    ArtifactKind::DockerImage => "image",
                    ArtifactKind::DockerBuildCache => "build-cache",
                    ArtifactKind::DockerVolume => "volume",
                    _ => "unknown",
                };
                result.unowned_bytes += candidate.unique_bytes;
                result.unowned.push(UnownedRow {
                    path_or_object: candidate.reference,
                    bytes: candidate.unique_bytes,
                    reason: UnownedReason::DockerNoJoin,
                    shared_bytes: Some(candidate.shared_bytes),
                    note,
                    docker_kind: Some(docker_kind.to_string()),
                });
            }
        }
    }

    result
}

pub fn to_json(report: &Report) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}
