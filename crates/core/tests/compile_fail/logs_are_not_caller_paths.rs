//! json-persistence-is-allowlisted (re-review 5, finding 4): the ledger
//! is a store's `ledger.jsonl` (or the override the gate reads itself),
//! and the LaunchAgent plist is where the gate resolves it. Appending a
//! line to a file the caller names, or removing a plist the caller
//! names, does not compile.
use std::path::Path;
use swamp_core::fs_gate::store;

fn main() {
    let _ = store::append_line(store::LogFile::Ledger(Path::new("/Users/me/.zshrc")), "x");
    let _ = store::remove_text(store::TextFile::LaunchAgent {
        plist: Path::new("/Users/me/Library/LaunchAgents/other.plist"),
    });
}
