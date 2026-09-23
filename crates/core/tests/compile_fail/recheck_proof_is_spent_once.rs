//! execution-sinks-recheck-live-state: a proof is consumed by the
//! destructive call it licenses, so one recheck cannot license two moves.
use std::path::Path;
use swamp_core::authority::Authorized;
use swamp_core::fs_gate::destroy::trash_move;
use swamp_core::recheck::RecheckProof;

fn twice(proof: RecheckProof, auth: &Authorized) {
    let _ = trash_move(proof, auth, Path::new("/tmp/trash"), "a");
    let _ = trash_move(proof, auth, Path::new("/tmp/trash"), "b");
}

fn main() {}
