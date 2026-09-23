//! target: crates/tui/src/actions.rs
//! mode: append
//! expect: accept
//! source: accept, re-review 5 findings 3 and 4
//! why: the TUI naming the resolved swamp dir -- the one store every TUI action reads protection from -- is the legitimate shape; building one from a path is the store modules'
/// Accept: the resolved store.
fn sweep_accept_store() -> StoreDir {
    swamp_core::fs_gate::StoreDir::resolved()
}
