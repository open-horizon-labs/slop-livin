use std::{path::Path, process::Command};
pub fn occupied(path: &Path) -> bool {
    Command::new("lsof")
        .arg("--")
        .arg(path)
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(true)
}
