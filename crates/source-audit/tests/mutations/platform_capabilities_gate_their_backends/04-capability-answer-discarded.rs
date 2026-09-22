//! target: crates/core/src/schedule.rs
//! mode: append
//! why: discarded-result variant -- the capability is asked first, in the right place, and the answer thrown away, so the installer proceeds on every platform

pub fn install(interval_raw: &str, roots: &[PathBuf]) -> Result<String> {
    // Asking is not refusing.
    let _ = scheduling();
    let seconds = parse_interval(interval_raw)?;
    let exe = current_exe()?;
    let plist = plist_path();
    let log = log_file();
    fs::create_dir_all(log_dir()).context("create log dir")?;
    let body = render_plist(&exe, roots, seconds, &log, None);
    fs::write(&plist, body)?;
    load_plist(&plist)?;
    Ok(format!("Scheduled observation every {}\n", format_interval(seconds)))
}
