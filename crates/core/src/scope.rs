//! Effective scan scope (#41): which roots swamp actually looks at, and
//! why -- built-in defaults, detector results (`crate::locations`), and
//! config `include`/`exclude`/`disabled_detectors`, folded into one
//! reusable, serde-serializable [`EffectiveScope`] every command shares.
//!
//! This module is pure and fixture-injectable: [`resolve_effective_scope`]
//! takes an explicit [`crate::locations::Environment`], [`ScanConfig`],
//! optional explicit command roots, and a detector [`crate::locations::Registry`],
//! and returns a value -- no home-directory or filesystem access happens
//! anywhere else in this file except the read-only presence/readability
//! check in [`stat_root`]. Tests inject a temp-dir "home" so they never
//! touch the real one.
//!
//! Explicitly **not** this chunk's job (left for #42/#50, see
//! `.oh/handoffs/2026-09-21-claude-full-scope.md` section 3 and
//! `.oh/sessions/2026-09-21-scope-and-detector-registry.md`): making the
//! walker/observation pipeline actually consume more than one root
//! coherently against shared per-volume history. `resolve_effective_scope`
//! only decides *which* roots are in scope; #42 makes multi-root
//! *observation* of that scope coherent.

use crate::locations::{
    Environment, LocationStatus, ProposedLocation, Provenance, Registry, StorageCategory,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The `[scan]` table in `config.toml`. Every field optional; a missing
/// `[scan]` table is the all-defaults value (`defaults = true`, no
/// includes/excludes/disabled detectors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Whether the built-in default roots (`~/src`, `~/Library/Developer`,
    /// `~/Library/Caches` on macOS) are in scope. This flag controls
    /// *only* the `builtin-defaults` detector (see
    /// `crate::locations::builtin`); every other detector is controlled
    /// independently by `disabled_detectors`. `defaults = false` gives
    /// "explicit-only scope": `include` entries plus whatever detectors
    /// remain enabled (i.e. not separately named in `disabled_detectors`).
    /// This is a documented decision, not the only defensible reading of
    /// the issue text -- see the session note for the alternative
    /// considered and why it was rejected.
    pub defaults: bool,
    /// Additional roots, always in scope regardless of `defaults`.
    /// `~` and relative-to-home paths are resolved against the
    /// environment's home directory.
    pub include: Vec<String>,
    /// Roots (or root prefixes) pruned from scope, applied *after*
    /// defaults/detectors/includes are resolved. Always wins: an
    /// exclusion removes a root regardless of source, including an
    /// explicit command-line root.
    pub exclude: Vec<String>,
    /// Detector IDs to exclude from resolution entirely (the detector's
    /// `detect()` is still called by the registry only to be labelled
    /// `Disabled` for transparency -- see `crate::locations::Registry::resolve`).
    pub disabled_detectors: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            defaults: true,
            include: Vec::new(),
            exclude: Vec::new(),
            disabled_detectors: Vec::new(),
        }
    }
}

impl ScanConfig {
    /// The `[scan]` table body `config init` writes, documenting every
    /// key with its meaning and default.
    pub fn to_toml_table(&self) -> String {
        let include = toml_string_array(&self.include);
        let exclude = toml_string_array(&self.exclude);
        let disabled = toml_string_array(&self.disabled_detectors);
        format!(
            "\n[scan]\n\
# Built-in default roots (~/src, ~/Library/Developer, ~/Library/Caches on\n\
# macOS) plus every enabled detector's results. false = explicit-only\n\
# scope: only `include` plus detectors not named in `disabled_detectors`.\n\
defaults = {}\n\
# Extra roots always in scope, e.g. [\"~/code\", \"/Volumes/data/src\"].\n\
include = {}\n\
# Roots (or root prefixes) pruned from scope; always wins over defaults,\n\
# detectors, and include, e.g. [\"~/src/scratch\"].\n\
exclude = {}\n\
# Detector IDs to turn off without excluding a path another enabled\n\
# root already reaches, e.g. [\"homebrew\"]. `swamp scope --json` lists ids.\n\
disabled_detectors = {}\n",
            self.defaults, include, exclude, disabled
        )
    }
}

fn toml_string_array(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let quoted: Vec<String> = items.iter().map(|s| format!("{s:?}")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Why a root ended up in scope. A root can have more than one reason
/// (e.g. it is both a built-in default and something the user also
/// listed under `include`); every reason is retained, never collapsed
/// to "the first one found".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "kebab-case")]
pub enum RootReason {
    BuiltinDefault,
    Detector {
        detector_id: String,
        category: StorageCategory,
        provenance: Provenance,
    },
    Included,
    ExplicitCommand,
    /// Propagated up from a root that was folded into this one because
    /// it is a subdirectory of it (see `RootStatus::SkippedAsNested`).
    NestedFrom {
        path: PathBuf,
    },
}

/// The filesystem-facing state of a root already selected as an
/// in-scope candidate (i.e. it survived detector/defaults/include
/// selection). This is a different vocabulary from
/// `crate::locations::LocationStatus`, which is about whether a
/// *detector* could resolve a candidate at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum RootStatus {
    Present,
    Missing,
    Unreadable {
        reason: String,
    },
    /// Folded into `parent` because it is a subdirectory of another
    /// in-scope root; not scanned separately. `parent`'s `reasons` gains
    /// a `NestedFrom` entry for this path, per #41's "retained as an
    /// inclusion reason".
    SkippedAsNested {
        parent: PathBuf,
    },
    /// Pruned by a config `exclude` entry; always wins over every other
    /// source, including an explicit command-line root.
    Excluded {
        pattern: String,
    },
}

/// One root in the effective scope, with every reason it is there and
/// its current filesystem status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeRoot {
    pub path: PathBuf,
    pub reasons: Vec<RootReason>,
    pub status: RootStatus,
}

impl ScopeRoot {
    pub fn is_scan_target(&self) -> bool {
        matches!(self.status, RootStatus::Present | RootStatus::Missing)
    }
}

/// One detector's full output for `swamp scope` transparency, including
/// entries that never became a root (not-present, disabled, unresolved).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorSummary {
    pub detector_id: String,
    pub name: String,
    pub locations: Vec<ProposedLocation>,
}

/// A reusable, serde-serializable description of exactly what swamp will
/// scan for one invocation, and why -- shared by every command
/// (`scope`, `report`, `observe`, `ui`, `schedule`) through
/// [`resolve_effective_scope`], never recomputed ad hoc per command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveScope {
    pub catalog_version: String,
    pub generated_at: u64,
    pub defaults_enabled: bool,
    pub disabled_detectors: Vec<String>,
    pub configured_include: Vec<String>,
    pub configured_exclude: Vec<String>,
    /// True when this scope was resolved from explicit command-line
    /// roots (which replace inferred/default roots for the invocation,
    /// though configured exclusions still apply).
    pub explicit: bool,
    /// Every root considered, with reasons and status. Filter with
    /// [`ScopeRoot::is_scan_target`] and `status == Present` for the
    /// paths an observation should actually walk.
    pub roots: Vec<ScopeRoot>,
    /// Full per-detector transparency, independent of whether a
    /// detector's output became a root (see `DetectorSummary`).
    pub detectors: Vec<DetectorSummary>,
    /// Subtrees an `exclude` entry names that fall *inside* an in-scope
    /// root, i.e. a subtree the walker should prune once #50 wires
    /// exclusion patterns into the walk itself. Exposed now so the
    /// contract exists before that integration lands.
    pub pruned_subtrees: Vec<PruneNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneNote {
    pub root: PathBuf,
    pub pattern: String,
}

impl EffectiveScope {
    /// Paths an observation should actually walk: present, in-scope,
    /// not folded into a parent, not excluded.
    pub fn scan_paths(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .filter(|r| matches!(r.status, RootStatus::Present))
            .map(|r| r.path.clone())
            .collect()
    }

    /// True when nothing at all is in scope: no default, detector,
    /// include, or explicit root survived resolution. Distinct from
    /// "every candidate root happens to be missing on disk" -- that is
    /// still an explicit, non-empty scope. Callers must treat this as an
    /// error/notice, never fall back to cwd or home (#41).
    pub fn is_empty_scope(&self) -> bool {
        !self.roots.iter().any(|r| {
            matches!(
                r.status,
                RootStatus::Present | RootStatus::Missing | RootStatus::Unreadable { .. }
            )
        })
    }
}

fn expand_tilde(home: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

/// Lexically normalizes `.`/`..` components and trailing slashes without
/// touching the filesystem -- deliberately not `fs::canonicalize`, which
/// would follow symlinks. Root *resolution* here only ever dereferences
/// a root path itself once, when checking presence in [`stat_root`];
/// it never recursively follows symlinks found while normalizing or
/// walking (see #41's "never recursively follow symlinks").
fn lexically_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn normalize(home: &Path, raw: &str) -> PathBuf {
    lexically_normalize(&expand_tilde(home, raw))
}

fn normalize_path(home: &Path, p: &Path) -> PathBuf {
    match p.to_str() {
        Some(s) => normalize(home, s),
        None => lexically_normalize(p),
    }
}

/// Read-only presence/readability check. A single, non-recursive stat:
/// this dereferences the root path itself (ordinary `stat` behavior for
/// a symlinked root, matching how the walker will open it) but never
/// follows anything found *inside* the directory.
fn stat_root(path: &Path) -> RootStatus {
    match fs::metadata(path) {
        Ok(_) => RootStatus::Present,
        Err(e) if e.kind() == ErrorKind::NotFound => RootStatus::Missing,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => RootStatus::Unreadable {
            reason: "permission denied".to_string(),
        },
        Err(e) => RootStatus::Unreadable {
            reason: e.to_string(),
        },
    }
}

fn is_excluded(path: &Path, excludes: &[PathBuf]) -> Option<PathBuf> {
    excludes
        .iter()
        .find(|ex| path == ex.as_path() || path.starts_with(ex))
        .cloned()
}

/// Resolves the effective scan scope for one invocation. Pure: every
/// filesystem/environment input arrives through `env`; the only I/O is
/// the read-only presence/readability stat in [`stat_root`].
///
/// `explicit_roots`: when non-empty, these entirely replace inferred
/// defaults/detectors/includes for this invocation (#41's explicit-root
/// precedence); configured `exclude` entries still apply.
pub fn resolve_effective_scope(
    env: &Environment,
    config: &ScanConfig,
    explicit_roots: &[PathBuf],
    registry: &Registry,
    generated_at: u64,
) -> EffectiveScope {
    let mut effective_disabled = config.disabled_detectors.clone();
    if !config.defaults {
        effective_disabled
            .push(crate::locations::builtin::BUILTIN_DEFAULTS_DETECTOR_ID.to_string());
    }
    effective_disabled.sort();
    effective_disabled.dedup();

    let per_detector = registry.resolve(env, &effective_disabled);

    let detectors: Vec<DetectorSummary> = registry
        .detectors()
        .iter()
        .filter(|d| d.platforms().contains(&env.platform))
        .filter_map(|d| {
            per_detector
                .iter()
                .find(|(id, _)| id == d.id())
                .map(|(id, locs)| DetectorSummary {
                    detector_id: id.clone(),
                    name: d.name().to_string(),
                    locations: locs.clone(),
                })
        })
        .collect();

    // candidate: (normalized path, reason)
    let mut candidates: Vec<(PathBuf, RootReason)> = Vec::new();

    if !explicit_roots.is_empty() {
        for r in explicit_roots {
            candidates.push((normalize_path(&env.home, r), RootReason::ExplicitCommand));
        }
    } else {
        for summary in &detectors {
            for loc in &summary.locations {
                if loc.status != LocationStatus::Resolved {
                    continue;
                }
                let Some(path) = &loc.path else { continue };
                candidates.push((
                    normalize_path(&env.home, path),
                    RootReason::Detector {
                        detector_id: summary.detector_id.clone(),
                        category: loc.category,
                        provenance: loc.provenance.clone(),
                    },
                ));
            }
        }
        for inc in &config.include {
            candidates.push((normalize(&env.home, inc), RootReason::Included));
        }
    }

    // `BuiltinDefault` reasons are folded onto the same detector-sourced
    // candidates below (the built-in-defaults detector's own
    // `RootReason::Detector` entries already carry that information);
    // `RootReason::BuiltinDefault` is kept as a type for forward
    // compatibility / doc clarity but every current builtin-defaults
    // candidate arrives tagged `Detector { detector_id: "builtin-defaults", .. }`,
    // which is more informative (it also carries provenance/category).

    let excludes: Vec<PathBuf> = config
        .exclude
        .iter()
        .map(|e| normalize(&env.home, e))
        .collect();

    // Merge duplicate normalized paths, retaining every reason.
    let mut order: Vec<PathBuf> = Vec::new();
    let mut reasons_by_path: HashMap<PathBuf, Vec<RootReason>> = HashMap::new();
    for (path, reason) in candidates {
        if !reasons_by_path.contains_key(&path) {
            order.push(path.clone());
        }
        reasons_by_path.entry(path).or_default().push(reason);
    }

    // Exclusion first: wins over everything, including explicit roots.
    let mut roots: Vec<ScopeRoot> = Vec::new();
    let mut kept: Vec<PathBuf> = Vec::new(); // survives exclusion, candidate for nesting
    let mut pruned_subtrees: Vec<PruneNote> = Vec::new();

    for path in &order {
        let reasons = reasons_by_path.remove(path).unwrap_or_default();
        if let Some(pattern) = is_excluded(path, &excludes) {
            roots.push(ScopeRoot {
                path: path.clone(),
                reasons,
                status: RootStatus::Excluded {
                    pattern: pattern.display().to_string(),
                },
            });
        } else {
            kept.push(path.clone());
            roots.push(ScopeRoot {
                path: path.clone(),
                reasons,
                status: stat_root(path), // provisional; nesting pass below may override
            });
        }
    }

    // Exclusion patterns that fall *inside* a kept root: exposed as
    // pruned subtrees for the future walker integration, per #41's
    // "exclusions prune matching subtrees".
    for ex in &excludes {
        for root in &kept {
            if ex != root && ex.starts_with(root) {
                pruned_subtrees.push(PruneNote {
                    root: root.clone(),
                    pattern: ex.display().to_string(),
                });
            }
        }
    }

    // Nested folding: shallowest-first, a kept root that is a
    // subdirectory of another kept root is folded into the parent.
    // Parent's reasons gain a `NestedFrom` entry.
    let mut by_depth: Vec<&PathBuf> = kept.iter().collect();
    by_depth.sort_by_key(|p| p.components().count());

    let mut top_level: Vec<PathBuf> = Vec::new();
    let mut nested_of: HashMap<PathBuf, PathBuf> = HashMap::new();
    for p in by_depth {
        if let Some(parent) = top_level.iter().find(|tl| *p != **tl && p.starts_with(tl)) {
            nested_of.insert(p.clone(), parent.clone());
        } else {
            top_level.push(p.clone());
        }
    }

    for root in &mut roots {
        if let Some(parent) = nested_of.get(&root.path) {
            root.status = RootStatus::SkippedAsNested {
                parent: parent.clone(),
            };
        }
    }
    // Propagate nested reasons onto their parent roots.
    let nested_reasons: Vec<(PathBuf, PathBuf)> = nested_of.into_iter().collect();
    for (child, parent) in nested_reasons {
        if let Some(parent_root) = roots.iter_mut().find(|r| r.path == parent) {
            parent_root
                .reasons
                .push(RootReason::NestedFrom { path: child });
        }
    }

    EffectiveScope {
        catalog_version: crate::locations::CATALOG_VERSION.to_string(),
        generated_at,
        defaults_enabled: config.defaults,
        disabled_detectors: effective_disabled,
        configured_include: config.include.clone(),
        configured_exclude: config.exclude.clone(),
        explicit: !explicit_roots.is_empty(),
        roots,
        detectors,
        pruned_subtrees,
    }
}

/// A change to what's in scope between two resolutions -- never a claim
/// about bytes on disk (see
/// `.oh/guardrails/coverage-changes-are-not-storage-changes.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageChange {
    pub path: PathBuf,
    pub kind: CoverageChangeKind,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageChangeKind {
    Added,
    Removed,
}

/// Compares two effective scopes' *in-scope* root sets (present/missing,
/// not nested/excluded) and reports what was added or removed, with a
/// one-line reason drawn from the root's reasons/status. Callers use
/// this for a report note like "coverage changed since last observation:
/// +root X (detector Y), -root Z (excluded)" -- this function never
/// touches byte history, and its output must never be mistaken for a
/// storage change.
pub fn coverage_changes(
    previous: &EffectiveScope,
    current: &EffectiveScope,
) -> Vec<CoverageChange> {
    let prev_paths: std::collections::HashSet<&PathBuf> = previous
        .roots
        .iter()
        .filter(|r| r.is_scan_target())
        .map(|r| &r.path)
        .collect();
    let cur_paths: std::collections::HashSet<&PathBuf> = current
        .roots
        .iter()
        .filter(|r| r.is_scan_target())
        .map(|r| &r.path)
        .collect();

    let mut changes = Vec::new();
    for root in &current.roots {
        if root.is_scan_target() && !prev_paths.contains(&root.path) {
            changes.push(CoverageChange {
                path: root.path.clone(),
                kind: CoverageChangeKind::Added,
                reason: reason_label(&root.reasons),
            });
        }
    }
    for root in &previous.roots {
        if root.is_scan_target() && !cur_paths.contains(&root.path) {
            // Explain *why* it's gone using the current scope's record of
            // it, if any (e.g. now excluded); otherwise "no longer in scope".
            let reason = current
                .roots
                .iter()
                .find(|r| r.path == root.path)
                .map(|r| status_label(&r.status))
                .unwrap_or_else(|| "no longer in scope".to_string());
            changes.push(CoverageChange {
                path: root.path.clone(),
                kind: CoverageChangeKind::Removed,
                reason,
            });
        }
    }
    changes
}

fn reason_label(reasons: &[RootReason]) -> String {
    reasons
        .iter()
        .map(|r| match r {
            RootReason::BuiltinDefault => "built-in default".to_string(),
            RootReason::Detector { detector_id, .. } => format!("detector {detector_id}"),
            RootReason::Included => "configured include".to_string(),
            RootReason::ExplicitCommand => "explicit command root".to_string(),
            RootReason::NestedFrom { path } => format!("covers nested {}", path.display()),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn status_label(status: &RootStatus) -> String {
    match status {
        RootStatus::Present => "present".to_string(),
        RootStatus::Missing => "missing".to_string(),
        RootStatus::Unreadable { reason } => format!("unreadable ({reason})"),
        RootStatus::SkippedAsNested { parent } => format!("folded into {}", parent.display()),
        RootStatus::Excluded { pattern } => format!("excluded ({pattern})"),
    }
}

/// Persists the resolved scope as small JSON under `<store_dir>/scope.json`
/// so the next invocation can compute [`coverage_changes`] against it.
/// This file is coverage bookkeeping only -- never byte history, never
/// consulted by the growth store.
pub fn persist_effective_scope(store_dir: &Path, scope: &EffectiveScope) -> std::io::Result<()> {
    fs::create_dir_all(store_dir)?;
    let json = serde_json::to_string_pretty(scope).unwrap_or_default();
    fs::write(store_dir.join("scope.json"), json)
}

pub fn load_last_effective_scope(store_dir: &Path) -> Option<EffectiveScope> {
    let text = fs::read_to_string(store_dir.join("scope.json")).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locations::{Environment, Platform, Registry};
    use std::collections::HashMap;

    fn env_at(home: &Path) -> Environment {
        Environment::fixture(home.to_path_buf(), HashMap::new(), Platform::MacOS)
    }

    fn mk(dir: &Path, rel: &str) {
        std::fs::create_dir_all(dir.join(rel)).unwrap();
    }

    #[test]
    fn no_config_resolves_builtin_defaults_plus_detectors() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let scope = resolve_effective_scope(&env, &ScanConfig::default(), &[], &registry, 1000);
        let paths: Vec<_> = scope.scan_paths();
        assert!(paths.contains(&home.join("src")));
        // Missing candidates (e.g. ~/.cargo, never created in this
        // fixture) remain explicit scope, not silently dropped.
        assert!(
            scope
                .roots
                .iter()
                .any(|r| r.path == home.join(".cargo") && r.status == RootStatus::Missing)
        );
    }

    #[test]
    fn defaults_false_is_explicit_only_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        mk(home, "code");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let mut cfg = ScanConfig {
            defaults: false,
            ..ScanConfig::default()
        };
        cfg.include.push("~/code".to_string());
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        // ~/src is a builtin default, not an included/detected root:
        // defaults=false drops it.
        assert!(!scope.roots.iter().any(|r| r.path == home.join("src")));
        // ~/code came from `include`, so it survives defaults=false.
        assert!(scope.scan_paths().contains(&home.join("code")));
        // Cargo/rustup/homebrew detectors are untouched by defaults=false
        // (only disabled_detectors controls them) -- explicit-only scope
        // still includes every *enabled* detector.
        assert!(scope.roots.iter().any(|r| r.path == home.join(".cargo")));
    }

    #[test]
    fn disabled_detector_does_not_hide_path_reachable_via_another_root() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // ~/.cargo lives *inside* ~/src for this fixture, so even with
        // the cargo-home detector disabled, ~/src (a builtin default)
        // still reaches it -- disabling discovery must not hide a path
        // covered by another enabled parent root.
        std::fs::create_dir_all(home.join("src/.cargo")).unwrap();
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            disabled_detectors: vec!["cargo-home".to_string()],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        assert!(scope.scan_paths().contains(&home.join("src")));
        assert!(
            !scope
                .detectors
                .iter()
                .find(|d| d.detector_id == "cargo-home")
                .unwrap()
                .locations
                .iter()
                .any(|l| l.status == LocationStatus::Resolved),
            "disabled detector must report Disabled, not Resolved"
        );
    }

    #[test]
    fn exclusion_wins_over_builtin_default() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            exclude: vec!["~/src".to_string()],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        assert!(!scope.scan_paths().contains(&home.join("src")));
        let root = scope
            .roots
            .iter()
            .find(|r| r.path == home.join("src"))
            .unwrap();
        assert!(matches!(root.status, RootStatus::Excluded { .. }));
    }

    #[test]
    fn exclusion_wins_over_explicit_command_root() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "proj");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            exclude: vec![home.join("proj").display().to_string()],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[home.join("proj")], &registry, 1000);
        assert!(scope.scan_paths().is_empty());
        assert!(matches!(scope.roots[0].status, RootStatus::Excluded { .. }));
    }

    #[test]
    fn explicit_roots_replace_defaults_and_detectors() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        mk(home, "elsewhere");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let scope = resolve_effective_scope(
            &env,
            &ScanConfig::default(),
            &[home.join("elsewhere")],
            &registry,
            1000,
        );
        assert_eq!(scope.scan_paths(), vec![home.join("elsewhere")]);
        assert!(scope.explicit);
    }

    #[test]
    fn empty_scope_is_explicit_never_a_cwd_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            defaults: false,
            disabled_detectors: vec![
                "cargo-home".to_string(),
                "rustup".to_string(),
                "homebrew".to_string(),
            ],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        assert!(scope.is_empty_scope());
        assert!(scope.scan_paths().is_empty());
    }

    #[test]
    fn aliases_tilde_absolute_and_trailing_slash_all_dedupe() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            include: vec![
                "~/src".to_string(),
                format!("{}/", home.join("src").display()),
            ],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        let matches: Vec<_> = scope
            .roots
            .iter()
            .filter(|r| r.path == home.join("src"))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "aliases of the same path must merge into one root"
        );
        // Two `Included` reasons (once per config entry) plus the
        // builtin-defaults reason, since ~/src is also a default.
        assert!(matches[0].reasons.len() >= 2);
    }

    #[test]
    fn nested_root_folds_into_parent_and_retains_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src/nested");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            include: vec!["~/src/nested".to_string()],
            // Isolate this test from whatever real /opt/homebrew or
            // /usr/local happen to exist on the machine running it --
            // this test is about nested folding, not Homebrew.
            disabled_detectors: vec![
                "cargo-home".to_string(),
                "rustup".to_string(),
                "homebrew".to_string(),
            ],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        let nested = scope
            .roots
            .iter()
            .find(|r| r.path == home.join("src/nested"))
            .unwrap();
        assert!(matches!(nested.status, RootStatus::SkippedAsNested { .. }));
        let parent = scope
            .roots
            .iter()
            .find(|r| r.path == home.join("src"))
            .unwrap();
        assert!(parent.reasons.iter().any(
            |r| matches!(r, RootReason::NestedFrom { path } if path == &home.join("src/nested"))
        ));
        // The nested root's own bytes are only reachable via the parent
        // now; only the parent is a scan target.
        assert_eq!(scope.scan_paths(), vec![home.join("src")]);
    }

    #[test]
    fn nested_exclusion_is_recorded_as_a_pruned_subtree() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            exclude: vec!["~/src/scratch".to_string()],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        assert!(scope.scan_paths().contains(&home.join("src")));
        assert!(
            scope
                .pruned_subtrees
                .iter()
                .any(|p| p.root == home.join("src") && p.pattern.ends_with("src/scratch"))
        );
    }

    #[test]
    fn missing_optional_paths_remain_explicit_scope_not_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path(); // nothing created at all
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let scope = resolve_effective_scope(&env, &ScanConfig::default(), &[], &registry, 1000);
        assert!(
            !scope.is_empty_scope(),
            "candidates that are simply absent are still explicit scope"
        );
        assert!(
            scope
                .roots
                .iter()
                .any(|r| r.path == home.join("src") && r.status == RootStatus::Missing)
        );
    }

    #[test]
    fn escaped_and_spaced_paths_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("My Projects/a b")).unwrap();
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let cfg = ScanConfig {
            include: vec!["~/My Projects/a b".to_string()],
            ..ScanConfig::default()
        };
        let scope = resolve_effective_scope(&env, &cfg, &[], &registry, 1000);
        assert!(scope.scan_paths().contains(&home.join("My Projects/a b")));
    }

    #[test]
    fn coverage_changes_report_add_and_remove_without_touching_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(home, "src");
        mk(home, "extra");
        let env = env_at(home);
        let registry = Registry::with_builtins();

        let before = resolve_effective_scope(&env, &ScanConfig::default(), &[], &registry, 1000);
        let cfg_after = ScanConfig {
            include: vec!["~/extra".to_string()],
            exclude: vec!["~/Library/Caches".to_string()],
            ..ScanConfig::default()
        };
        let after = resolve_effective_scope(&env, &cfg_after, &[], &registry, 2000);

        let changes = coverage_changes(&before, &after);
        assert!(
            changes
                .iter()
                .any(|c| c.path == home.join("extra") && c.kind == CoverageChangeKind::Added)
        );
        assert!(changes.iter().any(
            |c| c.path == home.join("Library/Caches") && c.kind == CoverageChangeKind::Removed
        ));
        // This function's contract: it never reports bytes, only path/kind/reason.
    }

    #[test]
    fn persisted_scope_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let store = tmp.path().join("store");
        let env = env_at(home);
        let registry = Registry::with_builtins();
        let scope = resolve_effective_scope(&env, &ScanConfig::default(), &[], &registry, 1000);
        persist_effective_scope(&store, &scope).unwrap();
        let loaded = load_last_effective_scope(&store).expect("round trip");
        assert_eq!(loaded.catalog_version, scope.catalog_version);
        assert_eq!(loaded.roots.len(), scope.roots.len());
    }
}
