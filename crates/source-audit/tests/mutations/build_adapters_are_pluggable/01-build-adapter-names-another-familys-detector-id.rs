//! target: crates/core/src/build_adapters/gradle.rs
//! by: audit:ids_only_in_their_module
//! why: a build adapter naming another ecosystem's detector id directly --
//!      not its own family's twin, not the registry, not the matrix -- is
//!      exactly the central-chain shape the rule forbids: adding a
//!      detector must never mean editing another adapter's source.
fn sweep_reject_cross_family_id(id: &str) -> bool {
    id == "android"
}
