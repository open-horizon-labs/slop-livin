//! target: crates/core/src/build_adapters/node.rs
//! why: re-review 3's agent-side slip here -- the builder used correctly, then the field overwritten
pub fn sweep_claim(c: &super::BuildContainer, p: std::path::PathBuf) -> crate::artifact::NestedArtifact {
    let mut u = super::NestedUnitBuilder::new(c, crate::artifact::ArtifactRole::Output, p).build();
    u.coverage.supported = true;
    u
}
