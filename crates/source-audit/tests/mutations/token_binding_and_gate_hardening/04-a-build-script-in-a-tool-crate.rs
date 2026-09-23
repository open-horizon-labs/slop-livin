//! target: crates/harvest/build.rs
//! mode: create
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 6
//! why: a build script in a workspace crate the audits do not model runs on `cargo build --workspace` with no rule reading it
fn main() {}
