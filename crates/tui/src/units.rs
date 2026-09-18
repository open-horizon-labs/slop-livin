//! Markability: which report rows are a "folded unit" that Backspace can
//! mark for deletion. Per DESIGN.md / PRODUCT.md: folded artifacts only
//! (dependency trees, build outputs, caches, Docker objects) — never a
//! checkout, worktree, `.git`, Source tree, or unowned path.

use slop_livin_core::report::ArtifactKind;

/// A stable identity for a unit that can be marked, used as the key in
/// the app's mark-set and later to build the plan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(pub String);

impl UnitId {
    pub fn for_artifact(path: &std::path::Path) -> Self {
        UnitId(path.display().to_string())
    }
}

/// Whether this artifact kind is a foldable unit Backspace may mark.
/// Returns `Err(reason)` naming why an unmarkable kind is refused, so the
/// caller can show it inline.
pub fn markable(kind: &ArtifactKind) -> Result<(), &'static str> {
    match kind {
        ArtifactKind::DependencyTree
        | ArtifactKind::BuildOutput
        | ArtifactKind::Cache
        | ArtifactKind::DockerImage
        | ArtifactKind::DockerBuildCache
        | ArtifactKind::DockerVolume => Ok(()),
        ArtifactKind::Git => Err("git metadata is never a delete target"),
        ArtifactKind::Source => Err("source trees are never a delete target"),
        ArtifactKind::Loose => Err("loose bytes have no single owning path to fold"),
        ArtifactKind::Unknown => Err("kind is unclassified; not a folded unit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_artifacts_are_markable() {
        for k in [
            ArtifactKind::DependencyTree,
            ArtifactKind::BuildOutput,
            ArtifactKind::Cache,
            ArtifactKind::DockerImage,
            ArtifactKind::DockerBuildCache,
            ArtifactKind::DockerVolume,
        ] {
            assert!(markable(&k).is_ok(), "{k:?} should be markable");
        }
    }

    #[test]
    fn non_artifacts_are_refused_with_a_reason() {
        for k in [
            ArtifactKind::Git,
            ArtifactKind::Source,
            ArtifactKind::Loose,
            ArtifactKind::Unknown,
        ] {
            let reason = markable(&k).unwrap_err();
            assert!(!reason.is_empty());
        }
    }
}
