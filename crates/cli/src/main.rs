mod schedule;

use anyhow::Result;
use clap::{Parser, Subcommand};
use slop_livin_core::{
    render::{render_kinds, render_overview, render_project},
    report::{report_with_observe, to_json},
    scan::{ScanOptions, observation},
    store::Store,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "slop-livin")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Scan {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Project x worktree x artifact growth report.
    Report {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        docker_facts: Option<PathBuf>,
        /// Also run `du -skPx` on the root as an independent total (slow).
        #[arg(long)]
        verify_du: bool,
        /// How far back to look for the growth baseline (e.g. "24h",
        /// "30m", "7d"). Overrides the `since` setting in config.toml.
        #[arg(long)]
        since: Option<String>,
        /// Skip persisting this observation into the growth store; the
        /// report is read-only and growth/regrowth stay unset.
        #[arg(long)]
        no_observe: bool,
        /// Drill into one project: worktree -> kind -> path -> bytes ->
        /// growth -> regrowth -> signals. Text output only.
        #[arg(long)]
        project: Option<String>,
        /// Bytes and count per artifact kind across the root. Text output only.
        #[arg(long)]
        kinds: bool,
        /// Show every project row instead of the default top-N. Text
        /// output only.
        #[arg(long)]
        all: bool,
        /// List every unjoined Docker object individually instead of the
        /// default one-line-per-kind summary. Text output only.
        #[arg(long)]
        docker: bool,
    },
    /// Observe-only: walk `root`s and write the growth store, no
    /// rendering. This is what a scheduled LaunchAgent run executes.
    Observe {
        #[arg(required = true)]
        roots: Vec<PathBuf>,
    },
    /// Install, report on, or remove the opt-in per-user LaunchAgent that
    /// runs `observe` on a fixed interval (#31).
    Schedule {
        /// Install (or replace) the schedule with this interval, e.g.
        /// "30m", "1h", "12h", "1d".
        #[arg(long)]
        every: Option<String>,
        /// Remove the schedule.
        #[arg(long)]
        off: bool,
        roots: Vec<PathBuf>,
    },
}

/// `${SLOP_LIVIN_DIR}`, defaulting to `~/.local/share/slop-livin`.
fn slop_livin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SLOP_LIVIN_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/slop-livin")
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { root, store } => {
            let obs = observation(&ScanOptions {
                roots: vec![root],
                cross_device: false,
                max_depth: None,
            })?;
            if let Some(s) = store {
                Store::open(s)?.write(&obs)?;
            }
            println!("{}", serde_json::to_string_pretty(&obs)?);
        }
        Command::Report {
            root,
            json,
            docker_facts,
            verify_du,
            since,
            no_observe,
            project,
            kinds,
            all,
            docker,
        } => {
            // The growth store is always consulted, even under
            // `--no-observe`: growth is read from whatever prior
            // observations already exist there (item 5), and only the
            // *write* of a new observation is skipped.
            let store_dir = slop_livin_dir();
            let r = report_with_observe(
                &root,
                docker_facts.as_deref(),
                verify_du,
                Some(&store_dir),
                since.as_deref(),
                !no_observe,
            )?;
            if json {
                println!("{}", to_json(&r)?);
            } else if let Some(name) = project {
                match render_project(&r, &name) {
                    Some(text) => print!("{text}"),
                    None => {
                        eprintln!("no project named {name:?} found under {}", root.display());
                        std::process::exit(1);
                    }
                }
            } else if kinds {
                print!("{}", render_kinds(&r));
            } else {
                print!("{}", render_overview(&r, all, verify_du, docker));
            }
        }
        Command::Observe { roots } => {
            schedule::cmd_observe(slop_livin_dir(), roots)?;
        }
        Command::Schedule { every, off, roots } => {
            schedule::cmd_schedule(slop_livin_dir(), every, off, roots)?;
        }
    }
    Ok(())
}
