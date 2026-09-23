//! protection-fails-closed: an unreadable protect file is an error, and
//! there is no empty default to swallow it into -- `.unwrap_or_default()`
//! on the loaded list does not compile.
use std::path::Path;

fn main() {
    let _list = swamp_core::protection::load_protect(Path::new("/tmp/store")).unwrap_or_default();
}
