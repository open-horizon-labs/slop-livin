//! target: crates/core/src/schedule.rs
//! mode: append
//! why: an installer that never asks whether this platform has a scheduler writes a LaunchAgent plist into a directory no daemon reads and reports success

/// Installs the scheduled observation. Looks complete; asks nothing.
pub fn install(interval_raw: &str, roots: &[PathBuf]) -> Result<String> {
    let seconds = parse_interval(interval_raw)?;
    let exe = current_exe()?;
    let plist = plist_path();
    let log = log_file();
    fs::create_dir_all(log_dir()).context("create log dir")?;
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent).context("create LaunchAgents dir")?;
    }
    let body = render_plist(&exe, roots, seconds, &log, None);
    fs::write(&plist, body)?;
    load_plist(&plist)?;
    Ok(format!("Scheduled observation every {}\n", format_interval(seconds)))
}
