//! target: crates/core/src/schedule.rs
//! mode: append
//! why: the check is present but runs after the plist is on disk -- a refusal after a write leaves the state behind and reports failure

pub fn install(interval_raw: &str, roots: &[PathBuf]) -> Result<String> {
    let seconds = parse_interval(interval_raw)?;
    let exe = current_exe()?;
    let plist = plist_path();
    let log = log_file();
    fs::create_dir_all(log_dir()).context("create log dir")?;
    let body = render_plist(&exe, roots, seconds, &log, None);
    fs::write(&plist, body)?;
    // Asked, honoured, and far too late.
    if let Some(refusal) = scheduling().refusal() {
        bail!("{refusal}");
    }
    load_plist(&plist)?;
    Ok(format!("Scheduled observation every {}\n", format_interval(seconds)))
}
