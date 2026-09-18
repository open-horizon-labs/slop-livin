mod schedule;

use anyhow::Result;
use clap::{Parser, Subcommand};
use slop_livin_core::{
    render::{render_kinds, render_overview, render_project},
    report::{Report, report_full, to_json},
    scan::{ScanOptions, observation},
    store::Store,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "slop-livin")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Diffstat-ledger terminal UI (ratatui). Default when no
    /// subcommand is given.
    Ui {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Skip persisting a new observation; render the last one.
        #[arg(long)]
        no_observe: bool,
    },
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
        /// R4c: print each worktree's subdirectory growth table (and any
        /// large files that grew) instead of the overview. Combine with
        /// `--project` to narrow to one project; text output only, kept
        /// separate from `render.rs`'s overview rendering.
        #[arg(long)]
        dirs: bool,
        /// With `--dirs`, only print directories up to this many path
        /// components deep (relative to the worktree root). Unset shows
        /// every directory.
        #[arg(long)]
        depth: Option<usize>,
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
    match cli.command.unwrap_or(Command::Ui {
        root: PathBuf::from("."),
        no_observe: false,
    }) {
        Command::Ui { root, no_observe } => {
            slop_livin_tui::run(&root, no_observe)?;
        }
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
            dirs,
            depth,
        } => {
            // The growth store is always consulted, even under
            // `--no-observe`: growth is read from whatever prior
            // observations already exist there (item 5), and only the
            // *write* of a new observation is skipped.
            let store_dir = slop_livin_dir();
            let r = report_full(
                &root,
                docker_facts.as_deref(),
                verify_du,
                Some(&store_dir),
                since.as_deref(),
                !no_observe,
                dirs,
            )?;
            if json {
                println!("{}", to_json(&r)?);
            } else if dirs {
                match render_dirs(&r, project.as_deref(), depth) {
                    Ok(text) => print!("{text}"),
                    Err(name) => {
                        eprintln!("no project named {name:?} found under {}", root.display());
                        std::process::exit(1);
                    }
                }
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

/// R4c `--dirs` drill: per worktree, subdirectories sorted by growth desc
/// then bytes desc, with `changed <age>` derived from `mod_time_min`;
/// large files that grew are listed beneath. Kept in the CLI (not
/// `render.rs`) per this slice's scope: it is a minimal text view over
/// `dirs_by_worktree`/`files_by_worktree`, not part of the overview
/// renderer's contract.
///
/// Returns `Err(name)` when `only_project` names a project not present
/// in the report, mirroring `render_project`'s `None` case.
fn render_dirs(
    report: &Report,
    only_project: Option<&str>,
    depth: Option<usize>,
) -> Result<String, String> {
    use std::fmt::Write as _;

    let empty_dirs = std::collections::HashMap::new();
    let empty_files = std::collections::HashMap::new();
    let dirs_by_worktree = report.dirs_by_worktree.as_ref().unwrap_or(&empty_dirs);
    let files_by_worktree = report.files_by_worktree.as_ref().unwrap_or(&empty_files);

    if let Some(name) = only_project
        && !report.projects.iter().any(|p| p.name == name)
    {
        return Err(name.to_string());
    }

    let mut out = String::new();
    for project in &report.projects {
        if let Some(name) = only_project
            && project.name != name
        {
            continue;
        }
        for worktree in &project.worktrees {
            let mut rows: Vec<&slop_livin_core::report::DirRollup> = dirs_by_worktree
                .get(&worktree.worktree_id)
                .map(|v| v.iter().collect())
                .unwrap_or_default();
            if let Some(max_depth) = depth {
                rows.retain(|d| {
                    d.rel_path.is_empty() || d.rel_path.matches('/').count() < max_depth
                });
            }
            rows.sort_by(|a, b| {
                b.growth_bytes
                    .unwrap_or(0)
                    .cmp(&a.growth_bytes.unwrap_or(0))
                    .then(b.allocated_total.cmp(&a.allocated_total))
            });

            let _ = writeln!(
                out,
                "{} [{}] {}",
                project.name,
                worktree.worktree_id,
                worktree.path.display()
            );
            for row in &rows {
                let label = if row.rel_path.is_empty() {
                    ".".to_string()
                } else {
                    row.rel_path.clone()
                };
                let growth = row
                    .growth_bytes
                    .map(|g| format!(" ({g:+} bytes)"))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  {label:<40} {:>12} bytes{growth}  changed {}",
                    row.allocated_total,
                    age_from_mod_time_min(row.mod_time_min)
                );
            }

            let mut file_rows: Vec<&slop_livin_core::report::FileRow> = files_by_worktree
                .get(&worktree.worktree_id)
                .map(|v| {
                    v.iter()
                        .filter(|f| f.growth_bytes.unwrap_or(0) > 0)
                        .collect()
                })
                .unwrap_or_default();
            file_rows.sort_by(|a, b| b.growth_bytes.cmp(&a.growth_bytes));
            for f in file_rows {
                let _ = writeln!(
                    out,
                    "    large file {:<38} {:>12} bytes (+{} bytes)  changed {}",
                    f.rel_path,
                    f.allocated,
                    f.growth_bytes.unwrap_or(0),
                    age_from_mod_time_min(f.mod_time_min)
                );
            }
        }
    }
    Ok(out)
}

/// A rough human age string ("3d", "5h", "12m", "just now") from minutes
/// since the Unix epoch, relative to now.
fn age_from_mod_time_min(mod_time_min: i32) -> String {
    let now_min = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        / 60) as i64;
    let age_min = (now_min - mod_time_min as i64).max(0);
    if age_min < 1 {
        "just now".to_string()
    } else if age_min < 60 {
        format!("{age_min}m")
    } else if age_min < 60 * 24 {
        format!("{}h", age_min / 60)
    } else {
        format!("{}d", age_min / (60 * 24))
    }
}
