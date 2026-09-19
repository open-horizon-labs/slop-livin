//! Shared nested-artifact facts.
//!
//! An artifact row is an accounting boundary.  The units in this module are
//! identification facts inside that boundary; they are not, by themselves,
//! permission to remove anything.  In particular, ownership is deliberately
//! absent from an identity.  A later attribution pass may attach a unit to a
//! project, but changing that guess must not change the unit's identity or its
//! history.

use crate::entities::{Confidence, id_for};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactRole {
    Container,
    Profile,
    Dependency,
    TestExecutable,
    Example,
    BuildScriptOutput,
    Incremental,
    FinalOutput,
    CompanionMetadata,
    Residual,
    Unknown,
}

impl ArtifactRole {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Profile => "profile",
            Self::Dependency => "dependency",
            Self::TestExecutable => "test-executable",
            Self::Example => "example",
            Self::BuildScriptOutput => "build-script-output",
            Self::Incremental => "incremental",
            Self::FinalOutput => "final-output",
            Self::CompanionMetadata => "companion-metadata",
            Self::Residual => "residual",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Membership {
    Exclusive,
    SharedHardlink,
    Residual,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactEvidence {
    pub source: String,
    pub detail: String,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactVariant {
    pub package: Option<String>,
    pub target: Option<String>,
    pub profile: Option<String>,
    pub architecture: Option<String>,
    pub toolchain: Option<String>,
    pub features: Option<String>,
    pub configuration: Option<String>,
    pub generation: Option<String>,
    #[serde(default)]
    pub unknowns: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactCoverage {
    pub supported: bool,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NestedArtifact {
    /// Stable identity based on canonical container and relative path, not
    /// role or an inferred project/package owner.
    pub id: String,
    pub path: PathBuf,
    pub relative_path: String,
    pub parent_id: Option<String>,
    pub container_id: Option<String>,
    pub role: ArtifactRole,
    pub membership: Membership,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub device: u64,
    #[serde(default)]
    pub inode: u64,
    #[serde(default)]
    pub logical_bytes: u64,
    /// Aggregate allocated bytes for this node. Container values include descendants;
    /// only leaf `physical_bytes` are charged to the containing report row.
    pub bytes: u64,
    /// Physical bytes charged by this node after hardlink de-duplication.
    /// This is zero for aggregate/container nodes.
    pub physical_bytes: u64,
    /// Aggregate charge, for display only; do not sum with descendants.
    /// Folded groups with Unknown membership have no subgroup charge estimate;
    /// their zero placeholder must be displayed as unknown, not measured zero.
    #[serde(default)]
    pub physical_total: u64,
    pub mtime_max: u64,
    pub variant: ArtifactVariant,
    #[serde(default)]
    pub producer_evidence: Vec<ArtifactEvidence>,
    #[serde(default)]
    pub consumer_evidence: Vec<ArtifactEvidence>,
    pub coverage: ArtifactCoverage,
    /// Exact filesystem members that would have to be considered together.
    /// This does not make the group actionable; the action layer must still
    /// authorize and re-check it.
    pub action_group: Option<String>,
    pub present: bool,
    #[serde(default)]
    pub growth_bytes: Option<i64>,
    #[serde(default)]
    pub regrowth_count: u32,
}

impl NestedArtifact {
    pub fn storage_id(root: &Path, relative: &str) -> String {
        use std::os::unix::ffi::OsStrExt;
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut hash = blake3::Hasher::new();
        hash.update(b"nested-storage:v2\0");
        hash.update(root.as_os_str().as_bytes());
        hash.update(b"\0");
        Self::within(&hash.finalize().to_hex().to_string(), relative)
    }
    pub fn within(container_id: &str, relative: &str) -> String {
        if relative.is_empty() {
            return container_id.to_string();
        }
        id_for(&format!("nested-entry:v2:{container_id}:{relative}"))
    }
    pub fn stable_id(relative_path: &str, _role: &ArtifactRole) -> String {
        // Role is evidence and may be reclassified when a build message
        // arrives. The physical boundary is the identity.
        id_for(&format!("nested-artifact:v1:{relative_path}"))
    }

    pub fn action_group(relative_path: &str) -> String {
        id_for(&format!("nested-action:v1:{relative_path}"))
    }
}

pub fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

pub fn architecture_from_target(target: &str) -> Option<String> {
    let arch = target.split('-').next()?.trim();
    (!arch.is_empty()).then(|| arch.to_string())
}
