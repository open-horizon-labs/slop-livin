use anyhow::Result;
use clap::{Parser, Subcommand};
use slop_livin_core::{
    measurement::preregister,
    report::{render_text, report_with, to_json},
    scan::{ScanOptions, observation},
    store::Store,
    volume::truth,
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
    Pressure {
        #[arg(long, default_value_t = 10_000_000_000)]
        floor_bytes: u64,
    },
    Candidates {
        #[arg(long)]
        root: Option<PathBuf>,
    },
    ChangedSince {
        #[arg(long)]
        seconds: u64,
    },
    Measure {
        #[arg(long, default_value = "first-case-study")]
        name: String,
    },
    Schedule {
        #[arg(long)]
        every: Option<u64>,
        #[arg(long)]
        off: bool,
    },
    Truth {
        #[arg(long)]
        attributed: u64,
        #[arg(long)]
        reported: u64,
    },
    /// Project x worktree x artifact growth report (R2: discovery only).
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
    },
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
        Command::Pressure { floor_bytes } => println!(
            "{}",
            serde_json::json!({"tool":"pressure","floor_bytes":floor_bytes,"state":"question","source":"volume"})
        ),
        Command::Candidates { root } => println!(
            "{}",
            serde_json::json!({"tool":"candidates","root":root,"state":"insufficient-evidence","next_step":"scan"})
        ),
        Command::ChangedSince { seconds } => println!(
            "{}",
            serde_json::json!({"tool":"changed_since","seconds":seconds,"state":"question"})
        ),
        Command::Measure { name } => println!(
            "{}",
            serde_json::to_string_pretty(&preregister(name, 30, 10_000_000_000, 1))?
        ),
        Command::Schedule { every, off } => println!(
            "{}",
            serde_json::json!({"every":every,"off":off,"mode":"refresh-only","state":"explicit"})
        ),
        Command::Truth {
            attributed,
            reported,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&truth(attributed, reported, vec![]))?
        ),
        Command::Report {
            root,
            json,
            docker_facts,
            verify_du,
        } => {
            let r = report_with(&root, docker_facts.as_deref(), verify_du)?;
            if json {
                println!("{}", to_json(&r)?);
            } else {
                print!("{}", render_text(&r));
            }
        }
    }
    Ok(())
}
