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
use crate::report::{ArtifactKind, ProjectRow};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
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
            if p.is_file() {
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
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|text| parse_build_paths(&text));
    if let Some((target, build)) = config_values {
        layout.target_dir = target.map(|p| resolve_config_path(config.as_deref(), p));
        layout.build_dir = build.map(|p| resolve_config_path(config.as_deref(), p));
    }
    if layout.target_dir.is_none() {
        layout.target_dir = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from);
        if layout.target_dir.is_none() {
            layout.target_dir = std::env::var_os("CARGO_BUILD_TARGET_DIR").map(PathBuf::from);
        }
    }
    if layout.build_dir.is_none() {
        layout.build_dir = std::env::var_os("CARGO_BUILD_BUILD_DIR").map(PathBuf::from);
    }
    for p in [&mut layout.target_dir, &mut layout.build_dir] {
        if let Some(path) = p.as_mut() {
            if path.is_relative() {
                *path = worktree.join(&*path);
            }
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
    let mut section = String::new();
    let mut target = None;
    let mut build = None;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if section == "build" {
            match key.trim() {
                "target-dir" => target = Some(PathBuf::from(value)),
                "build-dir" => build = Some(PathBuf::from(value)),
                _ => {}
            }
        }
    }
    (target, build)
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
    let mut units = Vec::new();
    let target_rel = target_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("target");
    let root_id = NestedArtifact::stable_id("", &ArtifactRole::Container);
    units.push(node(
        target_dir,
        workspace_root.unwrap_or(target_dir),
        Some(root_id.clone()),
        None,
        ArtifactRole::Container,
        0,
        0,
        ArtifactVariant::default(),
        vec![evidence(
            "path-layout",
            format!("Cargo target/build container `{target_rel}`"),
            Confidence::Medium,
        )],
        vec![],
        ArtifactCoverage {
            supported: true,
            limits: vec![
                "toolchain, features, and generation are unknown without build records".into(),
                "Cargo build-dir internals are subject to change".into(),
            ],
        },
        None,
        true,
    ));

    let mut seen = HashSet::new();
    if let Ok(entries) = fs::read_dir(target_dir) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "CACHEDIR.TAG" || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let role = if name.ends_with(".d") {
                    ArtifactRole::CompanionMetadata
                } else {
                    ArtifactRole::Residual
                };
                add_leaf(
                    &mut units,
                    &path,
                    target_dir,
                    Some(root_id.clone()),
                    role,
                    &mut seen,
                    &ArtifactVariant::default(),
                    &format!("target/{name}"),
                );
                continue;
            }
            let (role, variant) = top_level_role(&name);
            let profile_id = NestedArtifact::stable_id(
                &relative_path(target_dir, &path),
                &ArtifactRole::Profile,
            );
            units.push(node(
                &path,
                target_dir,
                Some(profile_id.clone()),
                Some(root_id.clone()),
                if role == ArtifactRole::Profile {
                    ArtifactRole::Profile
                } else {
                    role.clone()
                },
                0,
                0,
                variant.clone(),
                vec![evidence(
                    "path-layout",
                    format!("Cargo target child `{name}`"),
                    Confidence::High,
                )],
                vec![],
                coverage_for_role(&role),
                None,
                true,
            ));
            inspect_profile(
                &path,
                target_dir,
                &profile_id,
                role,
                variant,
                &mut seen,
                &mut units,
            );
        }
    }
    aggregate_units(&mut units);
    let total_bytes = units
        .iter()
        .find(|u| u.id == root_id)
        .map(|u| u.bytes)
        .unwrap_or_default();
    let physical_bytes: u64 = units.iter().map(|u| u.physical_bytes).sum();
    for unit in units.iter_mut() {
        if unit.id != root_id {
            unit.container_id = Some(root_id.clone());
        }
    }
    CargoInspection {
        target_dir: target_dir.to_path_buf(),
        build_dir: None,
        total_bytes,
        physical_bytes,
        units,
        coverage: ArtifactCoverage {
            supported: workspace_root.is_some(),
            limits: if workspace_root.is_some() {
                vec![
                    "crate/package ownership is not inferred from hashed filenames".into(),
                    "test-vs-library identity needs compiler-artifact JSON evidence".into(),
                ]
            } else {
                vec!["Cargo.toml was not established; ownership is unknown".into()]
            },
        },
    }
}

fn top_level_role(name: &str) -> (ArtifactRole, ArtifactVariant) {
    match name {
        "debug" => (ArtifactRole::Profile, profile_variant("debug")),
        "release" => (ArtifactRole::Profile, profile_variant("release")),
        "build" => (ArtifactRole::BuildScriptOutput, profile_variant("unknown")),
        "incremental" => (ArtifactRole::Incremental, profile_variant("unknown")),
        n if looks_like_target_triple(n) => {
            let mut v = ArtifactVariant::default();
            v.target = Some(n.to_string());
            v.architecture = architecture_from_target(n);
            (ArtifactRole::Container, v)
        }
        _ => (ArtifactRole::Residual, profile_variant("unknown")),
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

fn inspect_profile(
    profile_dir: &Path,
    target_dir: &Path,
    parent_id: &str,
    parent_role: ArtifactRole,
    mut variant: ArtifactVariant,
    seen: &mut HashSet<(u64, u64)>,
    units: &mut Vec<NestedArtifact>,
) {
    if let Some(name) = profile_dir.file_name().and_then(|n| n.to_str()) {
        if looks_like_target_triple(name) {
            variant.target = Some(name.into());
            variant.architecture = architecture_from_target(name);
        }
    }
    let Ok(entries) = fs::read_dir(profile_dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let (role, child_variant) = match name.as_str() {
            "deps" => (ArtifactRole::Dependency, variant.clone()),
            "examples" => (ArtifactRole::Example, variant.clone()),
            "build" => (ArtifactRole::BuildScriptOutput, variant.clone()),
            "incremental" => (ArtifactRole::Incremental, variant.clone()),
            _ if is_dir => (ArtifactRole::Residual, variant.clone()),
            _ if name.ends_with(".d") => (ArtifactRole::CompanionMetadata, variant.clone()),
            _ => (ArtifactRole::FinalOutput, variant.clone()),
        };
        if is_dir {
            let id = NestedArtifact::stable_id(&relative_path(target_dir, &path), &role);
            units.push(node(
                &path,
                target_dir,
                Some(id.clone()),
                Some(parent_id.to_string()),
                role.clone(),
                0,
                0,
                child_variant.clone(),
                vec![evidence(
                    "path-layout",
                    format!("Cargo profile child `{name}`"),
                    Confidence::High,
                )],
                vec![],
                coverage_for_role(&role),
                None,
                true,
            ));
            inspect_leaf_directory(&path, target_dir, &id, role, child_variant, seen, units);
        } else {
            let group = action_group_for_companion(target_dir, &path, &name);
            add_leaf(
                units,
                &path,
                target_dir,
                Some(parent_id.to_string()),
                role,
                seen,
                &child_variant,
                &group,
            );
        }
    }
    let _ = parent_role;
}

fn inspect_leaf_directory(
    dir: &Path,
    target_dir: &Path,
    parent_id: &str,
    role: ArtifactRole,
    variant: ArtifactVariant,
    seen: &mut HashSet<(u64, u64)>,
    units: &mut Vec<NestedArtifact>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_role = if role == ArtifactRole::Dependency && name.ends_with(".d") {
            ArtifactRole::CompanionMetadata
        } else if role == ArtifactRole::Example {
            ArtifactRole::Example
        } else if role == ArtifactRole::BuildScriptOutput {
            ArtifactRole::BuildScriptOutput
        } else if role == ArtifactRole::Incremental {
            ArtifactRole::Incremental
        } else if name.ends_with(".d") {
            ArtifactRole::CompanionMetadata
        } else {
            role.clone()
        };
        let group = action_group_for_companion(target_dir, &path, &name);
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let id = NestedArtifact::stable_id(&relative_path(target_dir, &path), &child_role);
            units.push(node(
                &path,
                target_dir,
                Some(id.clone()),
                Some(parent_id.to_string()),
                child_role.clone(),
                0,
                0,
                variant.clone(),
                vec![evidence(
                    "path-layout",
                    "nested Cargo output directory",
                    Confidence::Medium,
                )],
                vec![],
                coverage_for_role(&child_role),
                Some(group),
                true,
            ));
            inspect_leaf_directory(
                &path,
                target_dir,
                &id,
                child_role,
                variant.clone(),
                seen,
                units,
            );
        } else {
            add_leaf(
                units,
                &path,
                target_dir,
                Some(parent_id.to_string()),
                child_role,
                seen,
                &variant,
                &group,
            );
        }
    }
}

fn action_group_for_companion(target_dir: &Path, path: &Path, name: &str) -> String {
    let stem = name.strip_suffix(".d").unwrap_or(name);
    NestedArtifact::action_group(&format!(
        "{}:group:{stem}",
        relative_path(target_dir, path.parent().unwrap_or(target_dir))
    ))
}

fn add_leaf(
    units: &mut Vec<NestedArtifact>,
    path: &Path,
    target_dir: &Path,
    parent_id: Option<String>,
    role: ArtifactRole,
    seen: &mut HashSet<(u64, u64)>,
    variant: &ArtifactVariant,
    action_group: &str,
) {
    let bytes = file_bytes(path);
    let physical_bytes = if let Some(meta) = fs::symlink_metadata(path).ok() {
        if meta.file_type().is_file() && seen.insert((meta.dev(), meta.ino())) {
            meta.blocks() * 512
        } else {
            0
        }
    } else {
        0
    };
    let rel = relative_path(target_dir, path);
    let coverage = coverage_for_role(&role);
    let mut producer = vec![evidence(
        "path-layout",
        format!("Cargo `{}` layout", role.label()),
        Confidence::Medium,
    )];
    let mut unknowns = variant.unknowns.clone();
    if role == ArtifactRole::Dependency {
        producer.push(evidence(
            "filename",
            "hashed dependency filename; package ownership not inferred",
            Confidence::Low,
        ));
        unknowns.push("package (unless JSON build evidence is supplied)".into());
    }
    let mut variant = variant.clone();
    variant.unknowns = unknowns;
    let id = NestedArtifact::stable_id(&rel, &role);
    let mut unit = node(
        path,
        target_dir,
        Some(id.clone()),
        parent_id.clone(),
        role,
        bytes,
        physical_bytes,
        variant,
        producer,
        vec![],
        coverage,
        Some(action_group.to_string()),
        true,
    );
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|m| m.nlink() > 1)
    {
        unit.membership = Membership::SharedHardlink;
    }
    units.push(unit);
}

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
    let mtime_max = fs::symlink_metadata(path)
        .ok()
        .map(|m| m.mtime().max(0) as u64)
        .unwrap_or(0);
    NestedArtifact {
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
    }
}

fn evidence(source: &str, detail: impl Into<String>, confidence: Confidence) -> ArtifactEvidence {
    ArtifactEvidence {
        source: source.into(),
        detail: detail.into(),
        confidence,
    }
}

fn coverage_for_role(role: &ArtifactRole) -> ArtifactCoverage {
    let mut limits = vec!["toolchain, features, and generation are unknown".into()];
    if matches!(role, ArtifactRole::Dependency | ArtifactRole::FinalOutput) {
        limits.push("hashed filenames do not establish package or test ownership".into());
    }
    ArtifactCoverage {
        supported: true,
        limits,
    }
}

fn file_bytes(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.file_type().is_file())
        .map(|meta| meta.blocks() * 512)
        .unwrap_or_default()
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
        let child_mtime = units[child_index].mtime_max;
        units[parent_index].bytes += child_bytes;
        units[parent_index].mtime_max = units[parent_index].mtime_max.max(child_mtime);
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
            if message.target_kind.iter().any(|k| k == "test") {
                unit.role = ArtifactRole::TestExecutable;
            } else if message.target_kind.iter().any(|k| k == "example") {
                unit.role = ArtifactRole::Example;
            }
            unit.producer_evidence.push(evidence(
                "cargo-json",
                "existing compiler-artifact message",
                Confidence::High,
            ));
            unit.variant
                .unknowns
                .retain(|u| u != "package (unless JSON build evidence is supplied)");
        }
    }
}

/// Inspect a target and apply only pre-existing Cargo JSON output. The text
/// is supplied by the caller so observation never searches for or creates a
/// build log and never invokes Cargo.
pub fn inspect_target_with_json(
    target_dir: &Path,
    workspace_root: Option<&Path>,
    json_text: &str,
) -> CargoInspection {
    let mut inspection = inspect_target(target_dir, workspace_root);
    let messages = parse_json_messages(json_text);
    apply_message_evidence(&mut inspection, &messages);
    inspection
}

/// Parse existing JSON lines without executing Cargo. Unknown message kinds
/// are ignored, as Cargo may add message variants over time.
pub fn parse_json_messages(text: &str) -> Vec<CargoMessageEvidence> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("reason").and_then(|r| r.as_str()) == Some("compiler-artifact"))
        .map(|v| CargoMessageEvidence {
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

/// Finds Cargo target/build rows already discovered by the common walker and
/// annotates them. It intentionally does not add an out-of-scope path to a
/// report: an external shared target must be observed explicitly first.
pub fn inspect_projects(projects: &[ProjectRow]) -> Vec<NestedArtifact> {
    let mut out = Vec::new();
    for project in projects {
        for wt in &project.worktrees {
            let layout = layout_for(&wt.path);
            let cargo_present = wt.path.join("Cargo.toml").is_file();
            for row in &wt.artifacts {
                if row.kind != ArtifactKind::BuildOutput || !row.path.is_dir() {
                    continue;
                }
                let matches_layout = layout.target_dir.as_ref().is_some_and(|p| p == &row.path)
                    || layout.build_dir.as_ref().is_some_and(|p| p == &row.path)
                    || row.path.file_name().and_then(|n| n.to_str()) == Some("target");
                if !matches_layout {
                    continue;
                }
                let mut inspection =
                    inspect_target(&row.path, cargo_present.then_some(wt.path.as_path()));
                inspection.build_dir = layout.build_dir.clone();
                if !cargo_present {
                    inspection.coverage.supported = false;
                    inspection
                        .coverage
                        .limits
                        .push("Cargo.toml absent at the worktree root".into());
                    for unit in &mut inspection.units {
                        unit.variant.unknowns.push("Cargo ownership".into());
                    }
                }
                out.extend(inspection.units);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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
