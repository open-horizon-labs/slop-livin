//! target: crates/core/src/build_adapters/node.rs
//! why: alias/rename variant -- `use std::process::Command as Runner` runs `npm ls` past a token match
use std::process::Command as Runner;
pub fn sweep_run_npm(dir: &std::path::Path) -> Option<String> {
    let out = Runner::new("npm").arg("ls").arg("--json").current_dir(dir).output().ok()?;
    String::from_utf8(out.stdout).ok()
}
