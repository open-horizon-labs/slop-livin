//! execution-sinks-recheck-live-state: moving a unit to the Trash takes a
//! `RecheckProof` (live snapshot + protection + occupancy) and an
//! `Authorized` token. The tempting shortcut -- move it with just the
//! authorization -- does not compile.
use std::path::Path;
use swamp_core::authority::Authorized;
use swamp_core::fs_gate::destroy::trash_move;

fn shortcut(auth: &Authorized) {
    let _ = trash_move(auth, Path::new("/tmp/trash"), "debug");
}

fn main() {}
