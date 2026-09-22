//! target: crates/core/src/build_adapters/maven.rs
//! why: discarded-evidence variant -- clearing a unit's stated limits through a mutating method, not an assignment
pub fn sweep_quiet(u: &mut crate::artifact::NestedArtifact) {
    u.coverage.limits.clear();
}
