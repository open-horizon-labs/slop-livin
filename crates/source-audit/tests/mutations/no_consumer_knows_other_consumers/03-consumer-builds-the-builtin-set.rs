//! target: crates/core/src/consumers/cache.rs
//! why: a consumer constructing the whole builtin set, which is registration knowledge it must not have
pub fn sweep_rebuild_bus() {
    let _ = with_builtins();
}

fn with_builtins() {}
