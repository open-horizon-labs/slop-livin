//! target: crates/core/src/build_adapters/node.rs
//! expect: accept
//! why: the sanctioned enrichment shape must stay legal -- re-open with `amend` and change through named methods
pub fn sweep_enrich(u: crate::artifact::NestedArtifact) -> crate::artifact::NestedArtifact {
    super::NestedUnitBuilder::amend(u)
        .limit("enriched later")
        .build()
}
