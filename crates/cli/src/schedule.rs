//! `slop-livin observe` and `slop-livin schedule` subcommand handlers.
//!
//! `observe` is the program the LaunchAgent installed by `schedule` runs:
//! walk + growth-store write only, never a rendered report. `schedule`
//! installs/reports/removes the per-user LaunchAgent itself. All LaunchAgent
//! logic lives in `slop_livin_core::schedule`; this module is CLI glue only.

use anyhow::{Result, bail};
use slop_livin_core::growth::load_config;
use slop_livin_core::report::observe_only;
use slop_livin_core::schedule::{
    self, LockOutcome, RunOutcome, acquire_lock, append_log, log_file, write_last_run,
};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// `slop-livin observe <root>...`. Exits 0 on success, on a graceful
/// "another observation is running" skip, and even on a timeout/error --
/// only in-process misuse (e.g. no roots given) is a hard error, since a
/// launchd-triggered run should never wedge into a retry storm.
pub fn cmd_observe(store_dir: PathBuf, roots: Vec<PathBuf>) -> Result<()> {
    if roots.is_empty() {
        bail!("observe needs at least one root");
    }

    let config = load_config(&store_dir);
    let timeout = Duration::from_secs(config.observe_timeout_sec.max(1));

    let lock = match acquire_lock(&store_dir)? {
        LockOutcome::Acquired(guard) => guard,
        LockOutcome::HeldBy { pid, since } => {
            println!("another observation is running (pid {pid}) since {since}");
            return Ok(());
        }
    };

    let start = Instant::now();
    let (tx, rx) = mpsc::channel();
    let work_roots = roots.clone();
    let work_dir = store_dir.clone();
    thread::spawn(move || {
        let mut summaries = Vec::new();
        let mut failure: Option<String> = None;
        for root in &work_roots {
            match observe_only(root, &work_dir, None) {
                Ok(summary) => summaries.push((root.clone(), summary)),
                Err(e) => {
                    failure = Some(e.to_string());
                    break;
                }
            }
        }
        // The receiver may already be gone if we timed out; that's fine,
        // the thread just finishes its work and exits.
        let _ = tx.send((summaries, failure));
    });

    match rx.recv_timeout(timeout) {
        Ok((summaries, failure)) => {
            let wall_ms = start.elapsed().as_millis() as u64;
            let now = slop_livin_core::entities::now();

            if let Some(msg) = failure {
                let outcome = RunOutcome {
                    observed_at: now,
                    wall_ms,
                    walked_total: 0,
                    projects: 0,
                    mode: "full".to_string(),
                    outcome: format!("error({msg})"),
                };
                append_log(&log_file(), &outcome)?;
                let _ = write_last_run(&store_dir, &outcome);
                drop(lock);
                eprintln!("observe failed: {msg}");
                std::process::exit(1);
            }

            for (root, summary) in &summaries {
                println!(
                    "root={} observed_at={} wall_ms={} walked_total={} projects={} mode={}",
                    root.display(),
                    summary.observed_at,
                    wall_ms,
                    summary.walked_total,
                    summary.projects,
                    summary.mode
                );
                println!(
                    "  github: calls={} worktrees_enriched={} elapsed={:.1}s",
                    summary.github.calls_made,
                    summary.github.worktrees_enriched,
                    summary.github.elapsed_secs
                );
            }

            let walked_total: u64 = summaries.iter().map(|(_, s)| s.walked_total).sum();
            let projects: usize = summaries.iter().map(|(_, s)| s.projects).sum();
            let outcome = RunOutcome {
                observed_at: now,
                wall_ms,
                walked_total,
                projects,
                mode: "full".to_string(),
                outcome: "ok".to_string(),
            };
            append_log(&log_file(), &outcome)?;
            write_last_run(&store_dir, &outcome)?;
            drop(lock);
            Ok(())
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let wall_ms = start.elapsed().as_millis() as u64;
            let now = slop_livin_core::entities::now();
            let outcome = RunOutcome {
                observed_at: now,
                wall_ms,
                walked_total: 0,
                projects: 0,
                mode: "full".to_string(),
                outcome: "timeout".to_string(),
            };
            append_log(&log_file(), &outcome)?;
            let _ = write_last_run(&store_dir, &outcome);
            drop(lock);
            eprintln!("observe timed out after {}s", config.observe_timeout_sec);
            std::process::exit(1);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            drop(lock);
            bail!("observe worker thread died without reporting a result");
        }
    }
}

/// `slop-livin schedule [--every <interval>] [--off] <root>...`.
pub fn cmd_schedule(
    store_dir: PathBuf,
    every: Option<String>,
    off: bool,
    roots: Vec<PathBuf>,
) -> Result<()> {
    if off {
        print!("{}", schedule::uninstall()?);
        return Ok(());
    }
    if let Some(interval) = every {
        print!("{}", schedule::install(&interval, &roots)?);
        return Ok(());
    }
    print!("{}", schedule::status(&store_dir)?);
    Ok(())
}
