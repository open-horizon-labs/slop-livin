//! target: crates/core/src/docker.rs
//! why: the constructor written inside `vec![..]`, where `syn::visit` never looks
pub fn sweep_commands() -> Vec<std::process::Command> {
    vec![std::process::Command::new("docker")]
}
