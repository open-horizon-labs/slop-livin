//! target: crates/cli/src/main.rs
//! why: re-review 3 slip -- a central match moved into a file no dispatch list contained
pub fn sweep_label(u: &swamp_core::artifact::NestedArtifact) -> &'static str {
    match u.adapter.as_deref() {
        Some("node") => "Node",
        Some("gradle") => "Gradle",
        _ => "other",
    }
}
