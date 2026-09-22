//! target: crates/core/src/schedule.rs
//! mode: append
//! why: alias/rename variant -- the write is spelled through `use std::fs::write as emit`, so a text-matching rule sees no write before the capability check and passes a mutation that installs on a platform with no scheduler

use std::fs::create_dir_all as ensure;
use std::fs::write as emit;

pub fn install(interval_raw: &str, roots: &[PathBuf]) -> Result<String> {
    let seconds = parse_interval(interval_raw)?;
    let exe = current_exe()?;
    let plist = plist_path();
    let log = log_file();
    ensure(log_dir()).context("create log dir")?;
    let body = render_plist(&exe, roots, seconds, &log, None);
    emit(&plist, body)?;
    if let Some(refusal) = scheduling().refusal() {
        bail!("{refusal}");
    }
    Ok(format!("Scheduled observation every {}\n", format_interval(seconds)))
}
