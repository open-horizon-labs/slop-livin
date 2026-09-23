//! target: crates/tui/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, findings 3 and 4
//! why: the TUI sink building its own store from a path (the ledger's directory, say) -- where protection would then be read from; stores come from the resolved swamp dir, and `StoreDir::at` is the store modules' capability
/// Sweep 5: the store is wherever the ledger is.
pub fn sweep5_store_of(ledger: &Ledger) -> Option<StoreDir> {
    let path = ledger.path();
    swamp_core::fs_gate::StoreDir::at(path.parent()?).ok()
}
