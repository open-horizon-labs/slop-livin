//! target: crates/tui/src/app.rs
//! why: alias variant -- the adapter id held in a const, so the comparison has no literal in it
const SWEEP_GRADLE: &str = "gradle";
pub fn sweep_is_gradle(u: &swamp_core::artifact::NestedArtifact) -> bool {
    u.adapter.as_deref() == Some(SWEEP_GRADLE)
}
