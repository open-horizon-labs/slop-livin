//! Read-only Cargo build-layout inspection.
//!
//! Cargo's build directory is intentionally an implementation detail.  This
//! adapter only makes claims that are supported by path layout, companion
//! metadata, and optionally existing `--message-format=json` records.  It
//! never invokes Cargo, reads a build script, or executes project code.

use crate::artifact::{
    ArtifactCoverage, ArtifactEvidence, ArtifactRole, ArtifactVariant, Membership, NestedArtifact,
    architecture_from_target, relative_path,
};
use crate::entities::Confidence;
use crate::fs_gate::MetadataExt;
use crate::report::{ArtifactKind, ProjectRow};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CargoInspection {
    pub target_dir: PathBuf,
    pub build_dir: Option<PathBuf>,
    pub total_bytes: u64,
    pub physical_bytes: u64,
    pub units: Vec<NestedArtifact>,
    pub coverage: ArtifactCoverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CargoMessageEvidence {
    #[serde(default)]
    pub profile_test: bool,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    pub target_name: Option<String>,
    pub target_kind: Vec<String>,
    pub package_id: Option<String>,
    pub filenames: Vec<PathBuf>,
}

/// The effective local Cargo directories that can be established without
/// running Cargo. Environment variables are process facts; config values are
/// read-only local metadata. Command-line overrides are necessarily unknown
/// to an observer and are called out in coverage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoLayout {
    pub target_dir: Option<PathBuf>,
    pub build_dir: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub notes: Vec<String>,
}

use serde::{Deserialize, Serialize};

pub fn layout_for(worktree: &Path) -> CargoLayout {
    let mut layout = CargoLayout::default();
    let mut config = None;
    let mut cursor = Some(worktree);
    while let Some(dir) = cursor {
        for name in ["config", "config.toml"] {
            let p = dir.join(".cargo").join(name);
            if crate::fs_gate::is_file(&p) {
                config = Some(p);
                break;
            }
        }
        if config.is_some() {
            break;
        }
        cursor = dir.parent();
    }
    layout.config_path = config.clone();

    let config_values = config
        .as_ref()
        .and_then(|p| {
            crate::fs_gate::read::bounded_string(p, crate::fs_gate::read::BoundedCap::MANIFEST).ok()
        })
        .map(|text| parse_build_paths(&text));
    if let Some((target, build)) = config_values {
        layout.target_dir = target.map(|p| resolve_config_path(config.as_deref(), p));
        layout.build_dir = build.map(|p| resolve_config_path(config.as_deref(), p));
    }
    if let Some(p) =
        std::env::var_os("CARGO_TARGET_DIR").or_else(|| std::env::var_os("CARGO_BUILD_TARGET_DIR"))
    {
        layout.target_dir = Some(p.into());
    }
    if let Some(p) = std::env::var_os("CARGO_BUILD_BUILD_DIR") {
        layout.build_dir = Some(p.into());
    }
    for p in [&mut layout.target_dir, &mut layout.build_dir] {
        if let Some(path) = p.as_mut()
            && path.is_relative()
        {
            *path = worktree.join(&*path);
        }
    }
    if layout.target_dir.is_none() {
        layout.target_dir = Some(worktree.join("target"));
    }
    if layout.build_dir.is_none() {
        layout.build_dir = layout.target_dir.clone();
    }
    if layout.config_path.is_none() {
        layout
            .notes
            .push("no local Cargo config was observed".into());
    }
    layout
}

fn parse_build_paths(text: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let Ok(value) = text.parse::<toml::Value>() else {
        return (None, None);
    };
    let path = |key| {
        value
            .get("build")
            .and_then(|b| b.get(key))
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
    };
    (path("target-dir"), path("build-dir"))
}

fn resolve_config_path(config: Option<&Path>, value: PathBuf) -> PathBuf {
    if value.is_absolute() {
        value
    } else {
        config
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(|p| p.join(&value))
            .unwrap_or(value)
    }
}

/// Inspect a target/build directory directly. This is useful when the build
/// root is shared or custom and is not beneath the checkout being reported.
pub fn inspect_target(target_dir: &Path, workspace_root: Option<&Path>) -> CargoInspection {
    inspect_target_incremental(target_dir, workspace_root, &[], None)
}

/// Replay-driven refresh. Only directories named by a complete event batch
/// (and their ancestors) are listed again; unaffected subtrees use saved facts.
pub fn inspect_target_incremental(
    target_dir: &Path,
    workspace_root: Option<&Path>,
    cached: &[NestedArtifact],
    changed: Option<&[PathBuf]>,
) -> CargoInspection {
    let scope = NestedArtifact::storage_id(target_dir, "");
    let mut units = Vec::new();
    let mut limits = Vec::new();
    let old: HashMap<PathBuf, &NestedArtifact> =
        cached.iter().map(|u| (u.path.clone(), u)).collect();
    let mut children: HashMap<PathBuf, Vec<&NestedArtifact>> = HashMap::new();
    for u in cached {
        if u.path != target_dir
            && let Some(p) = u.path.parent()
        {
            children.entry(p.to_path_buf()).or_default().push(u);
        }
    }
    fn copy_tree(
        path: &Path,
        old: &HashMap<PathBuf, &NestedArtifact>,
        children: &HashMap<PathBuf, Vec<&NestedArtifact>>,
        out: &mut Vec<NestedArtifact>,
    ) {
        if let Some(u) = old.get(path) {
            let mut u = (*u).clone();
            if u.is_dir {
                u.bytes = 0;
                u.logical_bytes = 0;
            }
            u.physical_bytes = 0;
            out.push(u);
            if let Some(kids) = children.get(path) {
                for k in kids {
                    copy_tree(&k.path, old, children, out);
                }
            }
        }
    }
    // Existing recursive scanner helper keeps traversal state explicit across calls.
    #[allow(clippy::too_many_arguments)]
    fn visit(
        path: &Path,
        root: &Path,
        scope: &str,
        old: &HashMap<PathBuf, &NestedArtifact>,
        children: &HashMap<PathBuf, Vec<&NestedArtifact>>,
        changed: Option<&[PathBuf]>,
        out: &mut Vec<NestedArtifact>,
        limits: &mut Vec<String>,
    ) {
        if path.to_str().is_none() {
            limits.push("non-UTF8 Cargo path unsupported; observation incomplete".into());
            return;
        }
        if let Some(changed) = changed
            && old.contains_key(path)
            && !changed
                .iter()
                .any(|c| c.starts_with(path) || c == path.parent().unwrap_or(path))
        {
            copy_tree(path, old, children, out);
            return;
        }
        let meta = match crate::fs_gate::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                limits.push(format!("{}: {e}", path.display()));
                return;
            }
        };
        let rel = relative_path(root, path);
        let (role, variant) = classify_path(&rel, meta.is_dir());
        let id = NestedArtifact::within(scope, &rel);
        let parent = if path == root {
            None
        } else {
            Some(NestedArtifact::within(
                scope,
                &relative_path(root, path.parent().unwrap()),
            ))
        };
        let mut u = node(
            path,
            root,
            Some(id),
            parent,
            role,
            if meta.is_file() {
                meta.blocks() * 512
            } else {
                0
            },
            0,
            variant,
            vec![evidence(
                "cargo-layout",
                "observed layout; not evidence of last use or obsolescence",
                Confidence::Medium,
            )],
            vec![],
            ArtifactCoverage {
                supported: true,
                complete: true,
                limits: vec!["Cargo intermediate layout is version-dependent".into()],
            },
            None,
            true,
        );
        u.is_dir = meta.is_dir();
        u.mtime_max = meta.mtime().max(0) as u64;
        u.device = meta.dev();
        u.inode = meta.ino();
        if meta.file_type().is_symlink() {
            u.role = ArtifactRole::Unknown;
            u.coverage.limits.push("symlink not followed".into());
        }
        u.logical_bytes = if meta.is_file() { meta.len() } else { 0 };
        u.membership = if meta.nlink() > 1 && meta.is_file() {
            Membership::SharedHardlink
        } else {
            Membership::Exclusive
        };
        if path != root {
            u.container_id = Some(scope.to_string());
        }
        out.push(u);
        if meta.is_dir() {
            match crate::fs_gate::read_dir(path) {
                Err(e) => limits.push(format!("{}: {e}", path.display())),
                Ok(entries) => {
                    let mut paths = Vec::new();
                    for entry in entries {
                        match entry {
                            Ok(e) => paths.push(e.path()),
                            Err(e) => limits.push(format!("{}: {e}", path.display())),
                        }
                    }
                    paths.sort();
                    for p in paths {
                        visit(&p, root, scope, old, children, changed, out, limits);
                    }
                }
            }
        } else if path == root {
            limits.push("build root is not a directory".into());
        }
    }
    visit(
        target_dir,
        target_dir,
        &scope,
        &old,
        &children,
        changed,
        &mut units,
        &mut limits,
    );
    let complete = limits.is_empty();
    if workspace_root.is_none() {
        limits.push("Cargo.toml was not established; ownership unknown".into());
    }
    let mut seen = HashSet::new();
    for u in &mut units {
        if !u.is_dir && seen.insert((u.device, u.inode)) {
            u.physical_bytes = u.bytes;
        }
        u.coverage.complete = complete;
        u.coverage.supported = workspace_root.is_some() && complete;
        u.coverage.limits.extend(limits.iter().cloned());
    }
    aggregate_units(&mut units);
    charge_physical(&mut units);
    enrich_fingerprints(target_dir, &mut units);
    let physical_bytes = units.iter().map(|u| u.physical_bytes).sum();
    let total_bytes = units.first().map_or(0, |u| u.bytes);
    CargoInspection {
        target_dir: target_dir.into(),
        build_dir: None,
        total_bytes,
        physical_bytes,
        units,
        coverage: ArtifactCoverage {
            supported: workspace_root.is_some() && complete,
            complete,
            limits,
        },
    }
}

/// Project decision-relevant units from the existing folded directory measurements.
/// Only fingerprint metadata and example executables receive shallow extra reads.
pub(crate) fn folded_units(
    root: &Path,
    projects: &[ProjectRow],
    dirs: &[crate::report::DirRollup],
) -> anyhow::Result<Vec<NestedArtifact>> {
    let scope = NestedArtifact::storage_id(root, "");
    let worktrees: HashMap<_, _> = projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .map(|w| (w.worktree_id.as_str(), w.path.as_path()))
        .collect();
    let mut units = Vec::new();
    let mut fingerprints = Vec::new();
    let mut examples = Vec::new();
    let mut profiles = Vec::new();
    let mut measurements = Vec::new();
    for d in dirs {
        let Some(worktree) = worktrees.get(d.worktree_id.as_str()) else {
            continue;
        };
        let path = worktree.join(&d.rel_path);
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let rel = relative.to_string_lossy();
        measurements.push((path.clone(), d.mod_time_min.max(0) as u64 * 60, d.complete));
        let parts: Vec<_> = rel.split('/').filter(|s| !s.is_empty()).collect();
        let offset = usize::from(parts.first().is_some_and(|p| looks_like_target_triple(p)));
        if parts.len() == offset + 1 {
            profiles.push(path.clone());
        }
        if parts.len() == offset + 3 && parts.get(offset + 1) == Some(&".fingerprint") {
            fingerprints.push(path.clone());
        }
        if parts.len() == offset + 2 && parts.get(offset + 1) == Some(&"examples") {
            examples.push(path.clone());
        }
        let keep = parts.len() <= offset + 2
            || (parts.len() == offset + 3
                && matches!(parts.get(offset + 1), Some(&"incremental" | &"build")));
        if !keep {
            continue;
        }
        let mut u = folded_node(root, &scope, &path, true, d.allocated_total);
        u.coverage.complete = d.complete;
        u.mtime_max = d.mod_time_min.max(0) as u64 * 60;
        units.push(u);
    }
    // A folded group's last change includes its deeper directories. Reuse their
    // measured metadata; do not stat files to reconstruct an exact inventory.
    let indexes: HashMap<_, _> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.path.clone(), i))
        .collect();
    for (path, mtime, complete) in measurements {
        for ancestor in path.ancestors().take_while(|p| p.starts_with(root)) {
            if let Some(&i) = indexes.get(ancestor) {
                units[i].mtime_max = units[i].mtime_max.max(mtime);
                units[i].coverage.complete &= complete;
            }
        }
    }
    // Temporary metadata nodes feed the shared parser, but are never retained.
    // Ordinary compiler outputs in deps are not listed or statted here.
    for dir in fingerprints {
        for entry in crate::fs_gate::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            let target = stem
                .strip_prefix("test-lib-")
                .or_else(|| stem.strip_prefix("test-bin-"))
                .or_else(|| stem.strip_prefix("test-integration-test-"));
            let Some(target) = target else { continue };
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            let dir = path.parent().unwrap();
            let Some((_, hash)) = dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.rsplit_once('-'))
            else {
                continue;
            };
            let executable = dir
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("deps")
                .join(format!("{target}-{hash}"));
            if let Some(u) = folded_executable(root, &scope, &executable)? {
                units.push(u);
                units.push(folded_node(root, &scope, &path, false, 0));
            }
        }
    }
    for dir in examples {
        for entry in crate::fs_gate::read_dir(dir)? {
            if let Some(u) = folded_executable(root, &scope, &entry?.path())? {
                units.push(u);
            }
        }
    }
    // Final outputs live immediately inside profiles, not inside deps. Keep
    // executables and library products, but leave metadata and internal files
    // folded. This is shallow even when the target contains millions of files.
    for dir in profiles {
        for entry in crate::fs_gate::read_dir(dir)? {
            let path = entry?.path();
            let library = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rlib" | "a" | "so" | "dylib")
            );
            if let Some(u) = folded_output(root, &scope, &path, !library)? {
                units.push(u);
            }
        }
    }
    enrich_fingerprints(root, &mut units);
    units.retain(|u| {
        u.is_dir
            || matches!(
                u.role,
                ArtifactRole::TestExecutable | ArtifactRole::Example | ArtifactRole::FinalOutput
            )
    });
    units.sort_by(|a, b| a.path.cmp(&b.path));
    units.dedup_by(|a, b| a.path == b.path);
    Ok(units)
}

fn folded_node(root: &Path, scope: &str, path: &Path, is_dir: bool, bytes: u64) -> NestedArtifact {
    let rel = relative_path(root, path);
    let (role, variant) = classify_path(&rel, is_dir);
    let mut u = node(
        path,
        root,
        Some(NestedArtifact::within(scope, &rel)),
        (path != root)
            .then(|| NestedArtifact::within(scope, &relative_path(root, path.parent().unwrap()))),
        role,
        bytes,
        0,
        variant,
        vec![evidence(
            "cargo-folded-v2",
            "directory aggregates; internal files are not retained",
            Confidence::High,
        )],
        vec![],
        ArtifactCoverage {
            supported: true,
            complete: true,
            limits: vec![
                "internal file history and subgroup hardlink attribution are not retained".into(),
            ],
        },
        None,
        true,
    );
    u.container_id = Some(scope.into());
    u.is_dir = is_dir;
    u
}

fn folded_executable(
    root: &Path,
    scope: &str,
    path: &Path,
) -> anyhow::Result<Option<NestedArtifact>> {
    folded_output(root, scope, path, true)
}

fn folded_output(
    root: &Path,
    scope: &str,
    path: &Path,
    executable_required: bool,
) -> anyhow::Result<Option<NestedArtifact>> {
    let m = match crate::fs_gate::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !m.is_file() || (executable_required && m.mode() & 0o111 == 0) {
        return Ok(None);
    }
    let mut u = folded_node(root, scope, path, false, m.blocks() * 512);
    u.device = m.dev();
    u.inode = m.ino();
    u.logical_bytes = m.len();
    u.mtime_max = m.mtime().max(0) as u64;
    if m.nlink() == 1 {
        u.membership = Membership::Exclusive;
        u.physical_total = u.bytes;
        u.physical_bytes = u.bytes;
    } else {
        u.membership = Membership::SharedHardlink;
    }
    Ok(Some(u))
}

/// Recheck only the selected unit and its recorded producer evidence.
pub(crate) fn reviewed_role(
    root: &Path,
    selected: &Path,
    fingerprints: &[PathBuf],
) -> anyhow::Result<(ArtifactRole, bool)> {
    let meta = crate::fs_gate::symlink_metadata(selected)?;
    let scope = NestedArtifact::storage_id(root, "");
    let mut units = vec![folded_node(root, &scope, selected, meta.is_dir(), 0)];
    for path in fingerprints {
        units.push(folded_node(root, &scope, path, false, 0));
    }
    enrich_fingerprints(root, &mut units);
    Ok((units[0].role.clone(), meta.is_dir()))
}

fn classify_path(rel: &str, is_dir: bool) -> (ArtifactRole, ArtifactVariant) {
    if rel.is_empty() {
        return (ArtifactRole::Container, ArtifactVariant::default());
    }
    let parts: Vec<_> = rel.split('/').collect();
    let triple = looks_like_target_triple(parts[0]);
    let offset = usize::from(triple);
    let mut variant = profile_variant(parts.get(offset).copied().unwrap_or("unknown"));
    if triple {
        variant.architecture = architecture_from_target(parts[0]);
        variant.configuration = Some(parts[0].into());
        variant.unknowns.retain(|s| s != "architecture");
    }
    if parts.len() <= offset {
        return (ArtifactRole::Container, variant);
    }
    if parts.len() == offset + 1 && is_dir {
        return (ArtifactRole::Profile, variant);
    }
    let role = match parts.get(offset + 1).copied() {
        Some("deps") => ArtifactRole::Dependency,
        Some("examples") => ArtifactRole::Example,
        Some("incremental") => ArtifactRole::Incremental,
        Some("build") => ArtifactRole::BuildScriptOutput,
        Some(".fingerprint") => ArtifactRole::CompanionMetadata,
        _ if !is_dir && parts.len() == offset + 2 => ArtifactRole::FinalOutput,
        _ => ArtifactRole::Residual,
    };
    let role = if !is_dir && rel.ends_with(".d") {
        ArtifactRole::CompanionMetadata
    } else {
        role
    };
    (role, variant)
}

/// Fingerprints provide a baseline test/executable distinction without running
/// Cargo. They describe an observed build variant, never current project intent.
fn enrich_fingerprints(root: &Path, units: &mut [NestedArtifact]) {
    let mut facts = HashMap::new();
    for u in units.iter().filter(|u| {
        !u.is_dir
            && u.role == ArtifactRole::CompanionMetadata
            && u.relative_path.contains("/.fingerprint/")
            && u.path.extension().is_some_and(|e| e == "json")
    }) {
        let name = u.path.file_name().unwrap().to_string_lossy();
        if !name.starts_with("test-") {
            continue;
        }
        let Some(dir) = u.path.parent() else { continue };
        let Some(crate_hash) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((_, hash)) = crate_hash.rsplit_once('-') else {
            continue;
        };
        let stem = name.trim_end_matches(".json");
        let target = stem
            .strip_prefix("test-lib-")
            .or_else(|| stem.strip_prefix("test-bin-"))
            .or_else(|| stem.strip_prefix("test-integration-test-"));
        let Some(target) = target else { continue };
        let Some(profile) = dir.parent().and_then(Path::parent) else {
            continue;
        };
        let Ok(text) = crate::fs_gate::read::bounded_string(
            &u.path,
            crate::fs_gate::read::BoundedCap::MANIFEST,
        ) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        facts.insert(
            profile.join("deps").join(format!("{target}-{hash}")),
            (target.to_string(), json, u.path.clone()),
        );
    }
    for u in units.iter_mut() {
        if u.role == ArtifactRole::TestExecutable {
            // Cached enrichment is current evidence, not permanent identity.
            u.role = ArtifactRole::Dependency;
            u.variant.target = None;
            u.variant.features = None;
            u.variant.toolchain = None;
            u.action_group = None;
            u.producer_evidence
                .retain(|e| e.source != "cargo-fingerprint");
        }
        if let Some((target, json, path)) = facts.get(&u.path) {
            u.role = ArtifactRole::TestExecutable;
            u.variant.target = Some(target.clone());
            u.variant.features = json.get("features").map(|v| v.to_string());
            u.variant.toolchain = json.get("rustc").map(|v| format!("fingerprint:{v}"));
            u.producer_evidence.push(evidence(
                "cargo-fingerprint",
                path.display().to_string(),
                Confidence::Medium,
            ));
            u.action_group = Some(NestedArtifact::storage_id(
                root,
                &format!("action:{}", u.relative_path),
            ));
        }
    }
}

fn profile_variant(profile: &str) -> ArtifactVariant {
    let mut v = ArtifactVariant::default();
    if profile != "unknown" {
        v.profile = Some(profile.into());
    } else {
        v.unknowns.push("profile".into());
    }
    v.unknowns.extend(
        ["architecture", "toolchain", "features", "generation"]
            .into_iter()
            .map(String::from),
    );
    v
}

fn looks_like_target_triple(name: &str) -> bool {
    name.matches('-').count() >= 2 && !name.contains('.')
}

// Existing node-construction helper centralizes the full artifact record shape.
#[allow(clippy::too_many_arguments)]
fn node(
    path: &Path,
    root: &Path,
    id: Option<String>,
    parent_id: Option<String>,
    role: ArtifactRole,
    bytes: u64,
    physical_bytes: u64,
    variant: ArtifactVariant,
    producer_evidence: Vec<ArtifactEvidence>,
    consumer_evidence: Vec<ArtifactEvidence>,
    coverage: ArtifactCoverage,
    action_group: Option<String>,
    present: bool,
) -> NestedArtifact {
    let relative_path = relative_path(root, path);
    let id = id.unwrap_or_else(|| NestedArtifact::stable_id(&relative_path, &role));
    let mtime_max = 0;
    NestedArtifact {
        physical_total: 0,
        is_dir: false,
        device: 0,
        inode: 0,
        logical_bytes: 0,
        id: id.clone(),
        path: path.to_path_buf(),
        relative_path,
        parent_id,
        container_id: None,
        role,
        membership: if physical_bytes == 0 {
            Membership::Unknown
        } else {
            Membership::Exclusive
        },
        bytes,
        physical_bytes,
        mtime_max,
        variant,
        producer_evidence,
        consumer_evidence,
        coverage,
        action_group,
        present,
        growth_bytes: None,
        regrowth_count: 0,
        decision_evidence: Vec::new(),
    }
}

fn evidence(source: &str, detail: impl Into<String>, confidence: Confidence) -> ArtifactEvidence {
    ArtifactEvidence {
        source: source.into(),
        detail: detail.into(),
        confidence,
    }
}

/// Every directory is emitted before its descendants. Fold child logical
/// totals and newest mtimes upward once, after the single layout traversal.
/// This keeps a 250k-file target from being recursively sized once per
/// container/profile in addition to enumerating it.
fn aggregate_units(units: &mut [NestedArtifact]) {
    let indexes: HashMap<String, usize> = units
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.id.clone(), index))
        .collect();
    for child_index in (0..units.len()).rev() {
        let Some(parent_id) = units[child_index].parent_id.clone() else {
            continue;
        };
        let Some(&parent_index) = indexes.get(&parent_id) else {
            continue;
        };
        if parent_index == child_index {
            continue;
        }
        let child_bytes = units[child_index].bytes;
        let child_logical = units[child_index].logical_bytes;
        let child_mtime = units[child_index].mtime_max;
        units[parent_index].bytes += child_bytes;
        units[parent_index].logical_bytes += child_logical;
        units[parent_index].mtime_max = units[parent_index].mtime_max.max(child_mtime);
    }
}

/// One deterministic charge per inode across all observed Cargo roots.
pub fn charge_physical(units: &mut [NestedArtifact]) {
    let mut seen = HashSet::new();
    for u in units.iter_mut() {
        u.physical_bytes = if !u.is_dir && seen.insert((u.device, u.inode)) {
            u.bytes
        } else {
            0
        };
        u.physical_total = u.physical_bytes;
    }
    let indexes: HashMap<_, _> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.id.clone(), i))
        .collect();
    for i in (0..units.len()).rev() {
        if let Some(parent) = units[i]
            .parent_id
            .as_ref()
            .and_then(|p| indexes.get(p))
            .copied()
            && parent != i
        {
            units[parent].physical_total += units[i].physical_total;
        }
    }
}

/// Apply optional existing Cargo JSON messages to an inspection. A malformed
/// or stale record is retained as a limitation; it never upgrades a path-only
/// guess into an ownership verdict.
pub fn apply_message_evidence(inspection: &mut CargoInspection, messages: &[CargoMessageEvidence]) {
    for message in messages {
        for filename in &message.filenames {
            let Some(unit) = inspection.units.iter_mut().find(|u| u.path == *filename) else {
                continue;
            };
            if let Some(name) = &message.target_name {
                unit.variant.target = Some(name.clone());
            }
            if let Some(package_id) = &message.package_id {
                unit.variant.package = Some(package_id.clone());
            }
            if message.profile_test || message.target_kind.iter().any(|k| k == "test") {
                unit.role = ArtifactRole::TestExecutable;
            } else if message.target_kind.iter().any(|k| k == "example") {
                unit.role = ArtifactRole::Example;
            }
            unit.producer_evidence.push(evidence(
                "cargo-json",
                "caller-supplied historical compiler-artifact message; path match does not prove freshness or execution",
                Confidence::Low,
            ));
            if let Some(features) = &message.features {
                unit.variant.features = Some(features.join(","));
            }
            unit.coverage.limits.push("JSON build record is not bound to current file content; cleanup requires fresh native evidence".into());
            unit.variant
                .unknowns
                .retain(|u| u != "package (unless JSON build evidence is supplied)");
        }
    }
}

/// Parse existing JSON lines without executing Cargo. Unknown message kinds
/// are ignored, as Cargo may add message variants over time.
pub fn parse_json_messages(text: &str) -> Vec<CargoMessageEvidence> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("reason").and_then(|r| r.as_str()) == Some("compiler-artifact"))
        .map(|v| CargoMessageEvidence {
            profile_test: v
                .get("profile")
                .and_then(|p| p.get("test"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            features: v.get("features").and_then(|v| v.as_array()).map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            }),
            target_name: v
                .get("target")
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(str::to_string),
            target_kind: v
                .get("target")
                .and_then(|t| t.get("kind"))
                .and_then(|k| k.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            package_id: v
                .get("package_id")
                .and_then(|p| p.as_str())
                .map(str::to_string),
            filenames: v
                .get("filenames")
                .and_then(|f| f.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(PathBuf::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

/// Already observed Cargo build boundaries only, deduplicated across owners.
pub fn project_roots(projects: &[ProjectRow]) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for project in projects {
        for wt in &project.worktrees {
            let layout = layout_for(&wt.path);
            let cargo_present = crate::fs_gate::is_file(wt.path.join("Cargo.toml"));
            if !cargo_present && !project.ecosystems.iter().any(|t| t == "rs") {
                continue;
            }
            for row in &wt.artifacts {
                if row.kind != ArtifactKind::BuildOutput || !crate::fs_gate::is_dir(&row.path) {
                    continue;
                }
                let matches_layout = layout.target_dir.as_ref().is_some_and(|p| p == &row.path)
                    || layout.build_dir.as_ref().is_some_and(|p| p == &row.path)
                    || row.path.file_name().and_then(|n| n.to_str()) == Some("target");
                if !matches_layout {
                    continue;
                }
                let canonical =
                    crate::fs_gate::canonicalize(&row.path).unwrap_or_else(|_| row.path.clone());
                if seen.insert(canonical) {
                    out.push((row.path.clone(), wt.path.clone()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn identifies_profiles_roles_companions_incremental_and_hardlinks() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        fs::create_dir_all(target.join("debug/examples")).unwrap();
        fs::create_dir_all(target.join("debug/incremental/unit")).unwrap();
        fs::create_dir_all(target.join("debug/build/pkg-hash/out")).unwrap();
        fs::write(target.join("debug/deps/libfoo-abc123.rlib"), b"dep").unwrap();
        fs::write(target.join("debug/deps/libfoo-abc123.d"), b"dep").unwrap();
        fs::hard_link(
            target.join("debug/deps/libfoo-abc123.rlib"),
            target.join("debug/examples/shared-output"),
        )
        .unwrap();
        fs::write(target.join("debug/examples/demo"), b"example").unwrap();
        fs::write(target.join("debug/incremental/unit/cache"), b"incremental").unwrap();
        fs::write(
            target.join("debug/build/pkg-hash/out/generated.rs"),
            b"generated",
        )
        .unwrap();
        let report = inspect_target(&target, Some(tmp.path()));
        assert!(
            report
                .units
                .iter()
                .any(|u| u.role == ArtifactRole::Dependency)
        );
        assert!(
            report
                .units
                .iter()
                .any(|u| u.role == ArtifactRole::CompanionMetadata)
        );
        assert!(report.units.iter().any(|u| u.role == ArtifactRole::Example));
        assert!(
            report
                .units
                .iter()
                .any(|u| u.role == ArtifactRole::Incremental)
        );
        assert!(
            report
                .units
                .iter()
                .any(|u| u.role == ArtifactRole::BuildScriptOutput)
        );
        assert!(report.physical_bytes <= report.total_bytes);
        assert!(
            report
                .units
                .iter()
                .any(|u| u.membership == Membership::SharedHardlink)
        );
        assert!(report.units.iter().any(|u| u.parent_id.is_some()));
    }

    #[test]
    fn config_paths_are_read_without_running_cargo() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".cargo")).unwrap();
        let mut f = fs::File::create(tmp.path().join(".cargo/config.toml")).unwrap();
        writeln!(
            f,
            "[build]\ntarget-dir = \"../shared-target\"\nbuild-dir = \"../shared-build\""
        )
        .unwrap();
        let layout = layout_for(tmp.path());
        assert_eq!(layout.target_dir, Some(tmp.path().join("../shared-target")));
        assert_eq!(layout.build_dir, Some(tmp.path().join("../shared-build")));
    }

    #[test]
    fn json_evidence_distinguishes_test_and_renamed_package_without_guessing() {
        let text = r#"{"reason":"compiler-artifact","package_id":"pkg 1.0.0","target":{"name":"renamed-test","kind":["test"]},"filenames":["/tmp/target/debug/deps/renamed-test"]}"#;
        let messages = parse_json_messages(text);
        assert_eq!(messages[0].target_name.as_deref(), Some("renamed-test"));
        assert_eq!(messages[0].target_kind, vec!["test"]);

        let tmp = tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        let output = target.join("debug/deps/renamed-test");
        fs::write(&output, b"test").unwrap();
        let mut inspection = inspect_target(&target, Some(tmp.path()));
        apply_message_evidence(
            &mut inspection,
            &[CargoMessageEvidence {
                profile_test: true,
                features: None,
                target_name: Some("renamed-test".into()),
                target_kind: vec!["test".into()],
                package_id: Some("pkg 1.0.0".into()),
                filenames: vec![output.clone()],
            }],
        );
        let unit = inspection.units.iter().find(|u| u.path == output).unwrap();
        assert_eq!(unit.role, ArtifactRole::TestExecutable);
        assert_eq!(unit.variant.package.as_deref(), Some("pkg 1.0.0"));
        assert!(
            unit.producer_evidence
                .iter()
                .any(|e| e.source == "cargo-json")
        );
    }

    #[test]
    fn absent_cargo_keeps_layout_visible_but_coverage_unknown() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::write(target.join("debug/app"), b"output").unwrap();
        let inspection = inspect_target(&target, None);
        assert!(!inspection.coverage.supported);
        assert!(
            inspection
                .coverage
                .limits
                .iter()
                .any(|limit| limit.contains("Cargo.toml"))
        );
        assert!(
            inspection
                .units
                .iter()
                .any(|u| u.role == ArtifactRole::FinalOutput)
        );
    }

    #[test]
    fn identity_survives_role_reclassification() {
        assert_eq!(
            NestedArtifact::stable_id("debug/deps/renamed-test", &ArtifactRole::Dependency),
            NestedArtifact::stable_id("debug/deps/renamed-test", &ArtifactRole::TestExecutable)
        );
    }
}
