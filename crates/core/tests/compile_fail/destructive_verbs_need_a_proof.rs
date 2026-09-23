//! execution-sinks-recheck-live-state (re-review 5, finding 4): `docker
//! rm`, the executable copy and `git worktree prune` take a recheck proof
//! (or the receipt of the move one licensed) and build their arguments
//! from it. A caller naming the Docker id, the copy destination or the
//! repository itself does not compile.
use std::path::Path;
use swamp_core::authority::Authorized;
use swamp_core::fs_gate::destroy;

fn shortcuts(auth: &Authorized) {
    let _ = destroy::docker_remove(auth, Path::new("sha256:x"), "image", "sha256:y");
    let _ = destroy::copy_preserved(auth, Path::new("/w/target"), Path::new("/w/target/x"), Path::new("/"));
    let _ = destroy::git_worktree_prune(auth, Path::new("/w/wt"), Path::new("/Users/me/repo"));
}

fn main() {}
