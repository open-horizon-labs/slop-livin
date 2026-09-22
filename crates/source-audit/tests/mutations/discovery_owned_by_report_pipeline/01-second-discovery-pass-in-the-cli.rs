//! target: crates/cli/src/main.rs
//! why: a second discovery pass over the shared history table, which is how ordering started mattering
fn sweep_cli_units(
    scope: &swamp_core::scope::EffectiveScope,
    store: &std::path::Path,
) -> Vec<swamp_core::external::ExternalUnit> {
    swamp_core::external::discover_and_measure(scope, Some(store), true, 1_000, 30, 3600)
        .unwrap_or_default()
}
