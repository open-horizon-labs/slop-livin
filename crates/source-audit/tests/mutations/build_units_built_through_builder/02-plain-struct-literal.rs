//! target: crates/core/src/build_adapters/gradle.rs
//! why: the original forbidden shape -- a literal claiming supported coverage for an untested layout
pub fn sweep_plain_literal(path: std::path::PathBuf) -> crate::artifact::NestedArtifact {
    crate::artifact::NestedArtifact {
        id: String::new(),
        relative_path: String::new(),
        path,
        parent_id: None,
        container_id: None,
        role: crate::artifact::ArtifactRole::Unknown,
        membership: crate::artifact::Membership::Exclusive,
        is_dir: true,
        device: 0,
        inode: 0,
        logical_bytes: 0,
        bytes: 0,
        physical_bytes: 0,
        physical_total: 0,
        mtime_max: 0,
        variant: Default::default(),
        producer_evidence: Vec::new(),
        consumer_evidence: Vec::new(),
        coverage: crate::artifact::ArtifactCoverage {
            supported: true,
            complete: true,
            limits: Vec::new(),
        },
        action_group: None,
        present: true,
        growth_bytes: None,
        regrowth_count: 0,
        decision_evidence: Vec::new(),
    }
}
