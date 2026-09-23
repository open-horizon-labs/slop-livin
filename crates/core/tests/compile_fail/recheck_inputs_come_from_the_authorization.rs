//! execution-sinks-recheck-live-state (re-review 5, finding 3): the live
//! recheck takes every input from the `Authorized` token -- anchor,
//! reviewed identity, sidecar members, store, Docker removal. A sink that
//! hands it its own store, path, "reviewed" identity or member list does
//! not compile.
use std::path::{Path, PathBuf};

fn main() {
    let members: Vec<PathBuf> = Vec::new();
    let _proof = swamp_core::recheck::run_all(
        Path::new("/tmp/other-store"),
        Path::new("/w/target"),
        None,
        &members,
    );
}
