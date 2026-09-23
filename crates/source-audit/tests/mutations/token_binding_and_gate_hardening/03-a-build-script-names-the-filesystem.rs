//! target: crates/core/build.rs
//! mode: create
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 6
//! why: a build script is compiled and run on every build, outside every crate-root deny; the audit now loads it as a module of its crate, so the gate paths it names are rejected like anywhere else
fn main() {
    let _ = std::fs::metadata("Cargo.toml");
}
