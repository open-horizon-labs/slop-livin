//! protection-fails-closed (re-review 5, finding 7): adding or removing
//! a keep-list entry spends the confirmation the CLI's `cmd_protect`
//! minted for exactly that change. Removing a protection without one
//! (the step that lets an approved plan move what it protected) does not
//! compile.
use std::path::Path;

fn main() {
    let _ = swamp_core::agents::protect_remove(Path::new("/tmp/store"), Path::new("/w/keep"));
    let _ = swamp_core::agents::protect_remove_confirmed(Path::new("/tmp/store"), Path::new("/w/keep"));
}
