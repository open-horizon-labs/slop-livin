//! target: crates/tui/src/model.rs
//! why: the exact shape this chunk removed -- the TUI special-casing one adapter by comparing its id
pub fn sweep_is_cargo(u: &swamp_core::artifact::NestedArtifact) -> bool {
    u.adapter.as_deref().filter(|a| *a != "cargo").is_none()
}
