mod schedule;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use swamp_core::{
    filter,
    render::{
        OverviewSort, render_kinds, render_overview_sorted, render_project_tree, render_types,
        render_view_builds, render_view_deps, render_view_docker, render_view_reconciliation,
        render_view_unowned, render_worktree_signals, render_worktrees,
    },
    report::{Report, report_full_mode, to_json},
    scan::{ScanOptions, observation},
    store::Store,
};

/// `--view` at root or with `--project`. `--kinds`/`--docker` remain as
/// aliases for `--view kinds`/`--view docker` (#33). `Worktrees` at root
/// is #35's git-enriched one-line-per-worktree listing
/// (`render_worktrees`); with `--project` it is the tree drill (#33),
/// also the default when `--project` is given with no `--view`.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum View {
    Worktrees,
    Builds,
    Deps,
    Docker,
    Kinds,
    Unowned,
    Reconciliation,
    /// Per-ecosystem rollup: projects wearing the tag, artifacts it
    /// generates, bytes and growth.
    Types,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum SortArg {
    Growth,
    Size,
    Name,
    Type,
    Age,
}

impl From<SortArg> for OverviewSort {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::Growth => OverviewSort::Growth,
            SortArg::Size => OverviewSort::Size,
            SortArg::Name => OverviewSort::Name,
            SortArg::Type => OverviewSort::Type,
            SortArg::Age => OverviewSort::Age,
        }
    }
}

#[derive(Parser)]
#[command(name = "swamp", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum ConfigAction {
    Show,
    Path,
    Init,
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
        /// Bytes and count per artifact kind across the root. Text output
        /// only. Deprecated alias for `--view kinds`.
        #[arg(long)]
        kinds: bool,
        /// Named view, at root or narrowed with `--project`: worktrees
        /// (git-enriched one-line-per-worktree listing at root, the tree
        /// drill with `--project`), builds, deps, docker, kinds, unowned,
        /// reconciliation. Every question the issue lists is exactly one
        /// command through this flag. Text output only.
        #[arg(long, value_enum)]
        view: Option<View>,
        /// Signals for one worktree (matched by exact or root-relative
        /// path), the one-command surface for a question the maintainer
        /// rule comment lists explicitly. Text output only.
        #[arg(long)]
        worktree: Option<PathBuf>,
        /// Filter worktrees/artifacts, e.g. "merge-complete idle > 48h".
        /// See `swamp_core::filter` for the grammar. Only consulted
        /// by `--view worktrees` at root.
        #[arg(long)]
        filter: Option<String>,
        /// Refresh GitHub enrichment live before reading it, instead of
        /// reading `enrich.parquet` as-is. By default `report` never
        /// shells out to `gh` -- run `swamp observe` (or wait for
        /// the schedule) to keep the cache warm, and reach for this flag
        /// only when you're fine waiting on live calls right now.
        #[arg(long)]
        enrich: bool,
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
        /// Skip the FSEvents-driven incremental attempt and force a full
        /// walk (also re-anchors the stored event id for next time).
        #[arg(long)]
        full: bool,
        /// Order of the overview's project rows: growth (default), size,
        /// name, type (grouped by ecosystem), age (oldest artifact first).
        #[arg(long, value_enum, default_value = "growth")]
        sort: SortArg,
        /// Reverse the sort order (smallest first, newest first, …).
        #[arg(long)]
        reverse: bool,
    },
    /// Observe-only: walk `root`s, write the growth store, and refresh
    /// GitHub enrichment live for every GitHub-remote worktree found
    /// (concurrent, coalesced per repo -- see `github::observe_all`). No
    /// rendering. This is what a scheduled LaunchAgent run executes, and
    /// the only `swamp` command that calls `gh` on your behalf by
    /// default; `report` reads whatever this last wrote.
    Observe {
        #[arg(required = true)]
        roots: Vec<PathBuf>,
        /// Skip the FSEvents-driven incremental attempt and force a full
        /// walk (also re-anchors the stored event id for next time).
        #[arg(long)]
        full: bool,
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
    /// Propose a plan from the report: folded artifact rows only, each
    /// with project, worktree, kind, bytes, growth, recovery contract and
    /// signals. Nothing is deleted. Prints the plan id and the approve
    /// command (R7, #26).
    Propose {
        root: PathBuf,
        /// Narrow to rows matching this filter, e.g. "kind:BuildOutput idle > 30d".
        #[arg(long)]
        filter: Option<String>,
        /// Restrict to these exact artifact paths.
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Human authorization for ONE plan: writes a one-shot grant scoped to
    /// that plan id. Only a human at this keyboard should run this.
    Approve { plan_id: String },
    /// Execute an approved plan: sink re-derivation, Trash, ledger,
    /// measured free space. Refuses per unit with the fact that refused it.
    Execute {
        plan_id: String,
        #[arg(long, default_value = "human:cli")]
        actor: String,
        #[arg(long)]
        json: bool,
        /// Before trashing a build directory, copy compiled outputs to
        /// `<worktree>/bin/`: Rust `target/{release,debug}` executables,
        /// Python `dist/*.whl` and `build/**/*.so`. Other kinds: no-op.
        #[arg(long, short = 'k')]
        keep_executables: bool,
    },
    /// The configuration file: `config show` prints effective values,
    /// `config path` where it lives, `config init` writes one with every
    /// key and its meaning (never overwrites an existing file).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// List plans (newest first).
    Plans {
        #[arg(long)]
        json: bool,
    },
    /// Standing grants: `grant add '<predicate>' --budget 5GB --expires 7d`,
    /// `grant list`, `grant revoke <id>`. Predicates: kind:, project:,
    /// idle > <dur>, merge-complete. Only a human at this keyboard should
    /// add grants; MCP has no tool that can.
    Grant {
        #[command(subcommand)]
        cmd: GrantCmd,
    },
}

#[derive(Subcommand)]
enum GrantCmd {
    Add {
        predicate: String,
        /// Total bytes this grant may ever authorize, e.g. 5GB.
        #[arg(long)]
        budget: String,
        /// Lifetime, e.g. 7d.
        #[arg(long)]
        expires: String,
        /// Maximum number of units this grant may authorize.
        #[arg(long)]
        max_units: Option<u32>,
    },
    List,
    Revoke {
        grant_id: String,
    },
}

fn parse_size_arg(s: &str) -> Result<u64> {
    let t = s.trim().to_uppercase();
    let (num, mult) = if let Some(n) = t.strip_suffix("TB") {
        (n, 1_000_000_000_000u64)
    } else if let Some(n) = t.strip_suffix("GB") {
        (n, 1_000_000_000)
    } else if let Some(n) = t.strip_suffix("MB") {
        (n, 1_000_000)
    } else if let Some(n) = t.strip_suffix("KB") {
        (n, 1_000)
    } else if let Some(n) = t.strip_suffix('B') {
        (n, 1)
    } else {
        (t.as_str(), 1)
    };
    let v: f64 = num
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad size {s:?}"))?;
    Ok((v * mult as f64) as u64)
}

fn print_plan(plan: &swamp_core::actions::Plan) {
    println!(
        "plan {}  {} units  {}  expires in {}s",
        plan.id,
        plan.units.len(),
        swamp_core::render::human_bytes_pub(plan.planned_bytes()),
        plan.expires_at.saturating_sub(swamp_core::entities::now())
    );
    for u in &plan.units {
        println!(
            "  {:<14} {:>10}  {:>+10}  {}  {}  [{}]  {}",
            format!("{:?}", u.kind).to_lowercase(),
            swamp_core::render::human_bytes_pub(u.bytes),
            u.growth_bytes
                .map(swamp_core::render::human_bytes_signed)
                .unwrap_or_else(|| "—".into()),
            u.project,
            u.path.display(),
            u.recovery,
            u.signals.join(" · ")
        );
        if let Some(t) = u.track {
            print!("    [{}]", t.label());
        }
        if !u.warnings.is_empty() {
            print!("  ⚠ {}", u.warnings.join(" · "));
        }
        if u.track.is_some() || !u.warnings.is_empty() {
            println!();
        }
    }
    for r in &plan.refused {
        println!("  refused  {}  — {}", r.path.display(), r.cause);
    }
    println!(
        "authorize (human only): {}",
        swamp_core::actions::approve_command(&plan.id)
    );
}

/// `${SWAMP_DIR}`, defaulting to `~/.local/share/swamp`.
fn swamp_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SWAMP_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/swamp")
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Ui {
        root: PathBuf::from("."),
        no_observe: false,
    }) {
        Command::Ui { root, no_observe } => {
            swamp_tui::run(&root, no_observe)?;
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
            view,
            worktree,
            filter: filter_expr,
            enrich,
            all,
            docker,
            dirs,
            depth,
            full,
            sort,
            reverse,
        } => {
            // `--kinds`/`--docker` are deprecated aliases folded under
            // `--view` (#33); an explicit `--view` wins if somehow both
            // are given.
            let view = view.or_else(|| {
                if kinds {
                    Some(View::Kinds)
                } else if docker && project.is_none() {
                    Some(View::Docker)
                } else {
                    None
                }
            });
            // The growth store is always consulted, even under
            // `--no-observe`: growth is read from whatever prior
            // observations already exist there (item 5), and only the
            // *write* of a new observation is skipped. GitHub enrichment
            // is a separate opt-in (`--enrich`): plain `report` never
            // shells out to `gh`, regardless of `--no-observe`.
            let store_dir = swamp_dir();
            let progress =
                spawn_progress_line(!json && std::io::IsTerminal::is_terminal(&std::io::stderr()));
            let r = report_full_mode(
                &root,
                docker_facts.as_deref(),
                verify_du,
                Some(&store_dir),
                since.as_deref(),
                !no_observe,
                dirs,
                enrich,
                full,
            );
            progress.stop();
            let r = r?;
            let parsed_filter = match filter_expr.as_deref().map(filter::parse) {
                Some(Ok(f)) => Some(f),
                Some(Err(e)) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                None => None,
            };
            if json {
                println!("{}", to_json(&r)?);
            } else if let Some(wt_path) = worktree {
                match render_worktree_signals(&r, &wt_path) {
                    Some(text) => print!("{text}"),
                    None => {
                        eprintln!(
                            "no worktree at {} found under {}",
                            wt_path.display(),
                            root.display()
                        );
                        std::process::exit(1);
                    }
                }
            } else if dirs {
                match render_dirs(&r, project.as_deref(), depth) {
                    Ok(text) => print!("{text}"),
                    Err(name) => {
                        eprintln!("no project named {name:?} found under {}", root.display());
                        std::process::exit(1);
                    }
                }
            } else if let Some(name) = project {
                match view {
                    None | Some(View::Worktrees) => match render_project_tree(&r, &name) {
                        Some(text) => print!("{text}"),
                        None => {
                            eprintln!("no project named {name:?} found under {}", root.display());
                            std::process::exit(1);
                        }
                    },
                    Some(View::Builds) => print!("{}", render_view_builds(&r, Some(&name))),
                    Some(View::Deps) => print!("{}", render_view_deps(&r, Some(&name))),
                    Some(View::Docker) => print!("{}", render_view_docker(&r, Some(&name))),
                    Some(View::Kinds) => print!("{}", render_kinds(&r)),
                    Some(View::Types) => print!("{}", render_types(&r)),
                    Some(View::Unowned) => print!("{}", render_view_unowned(&r)),
                    Some(View::Reconciliation) => print!("{}", render_view_reconciliation(&r)),
                }
            } else {
                match view {
                    Some(View::Worktrees) => print!(
                        "{}",
                        render_worktrees(&r, &parsed_filter.unwrap_or_default())
                    ),
                    Some(View::Kinds) => print!("{}", render_kinds(&r)),
                    Some(View::Builds) => print!("{}", render_view_builds(&r, None)),
                    Some(View::Deps) => print!("{}", render_view_deps(&r, None)),
                    Some(View::Docker) => print!("{}", render_view_docker(&r, None)),
                    Some(View::Types) => print!("{}", render_types(&r)),
                    Some(View::Unowned) => print!("{}", render_view_unowned(&r)),
                    Some(View::Reconciliation) => print!("{}", render_view_reconciliation(&r)),
                    None => print!(
                        "{}",
                        render_overview_sorted(&r, all, verify_du, docker, sort.into(), reverse)
                    ),
                }
            }
        }
        Command::Propose {
            root,
            filter,
            paths,
            since,
            json,
        } => {
            let store_dir = swamp_dir();
            let r = report_full_mode(
                &root,
                None,
                false,
                Some(&store_dir),
                since.as_deref(),
                true,
                false,
                false,
                false,
            )?;
            let parsed = match filter.as_deref().map(filter::parse) {
                Some(Ok(f)) => Some(f),
                Some(Err(e)) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                None => None,
            };
            let plan = swamp_core::actions::propose(&r, parsed.as_ref(), &paths, "human:cli")?;
            swamp_core::actions::save_plan(&store_dir, &plan)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print_plan(&plan);
            }
        }
        Command::Approve { plan_id } => {
            // The human's confirm line: every unit with the facts on it,
            // before the grant is written.
            let plan = swamp_core::actions::load_plan(&swamp_dir(), &plan_id)?;
            for u in &plan.units {
                println!(
                    "  {:<16} {:>10}  {}{}",
                    u.verb,
                    swamp_core::render::human_bytes_pub(u.bytes),
                    u.path.display(),
                    if u.warnings.is_empty() {
                        String::new()
                    } else {
                        format!("  ⚠ {}", u.warnings.join(" · "))
                    }
                );
            }
            let g = swamp_core::actions::approve(&swamp_dir(), &plan_id, "human:cli")?;
            println!(
                "approved plan {} with one-shot grant {} (budget {}, {} units, expires {})",
                plan_id,
                g.id,
                swamp_core::render::human_bytes_pub(g.budget_bytes.unwrap_or(0)),
                g.max_units.unwrap_or(0),
                g.expires_at
            );
            println!("execute with: swamp execute {plan_id}");
        }
        Command::Config { action } => {
            let dir = swamp_dir();
            let path = dir.join("config.toml");
            match action {
                ConfigAction::Path => println!("{}", path.display()),
                ConfigAction::Show => {
                    print!("{}", swamp_core::growth::load_config(&dir).to_toml());
                    if !path.exists() {
                        eprintln!(
                            "(defaults; no file at {} — `swamp config init` writes one)",
                            path.display()
                        );
                    }
                }
                ConfigAction::Init => {
                    if path.exists() {
                        eprintln!("{} already exists; not overwriting", path.display());
                        std::process::exit(1);
                    }
                    std::fs::create_dir_all(&dir)?;
                    std::fs::write(&path, swamp_core::growth::GrowthConfig::default().to_toml())?;
                    println!("wrote {}", path.display());
                }
            }
        }
        Command::Execute {
            plan_id,
            actor,
            json,
            keep_executables,
        } => {
            let res = if keep_executables {
                swamp_core::actions::execute_keeping_executables(&swamp_dir(), &plan_id, &actor)?
            } else {
                swamp_core::actions::execute(&swamp_dir(), &plan_id, &actor)?
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!("plan {}: {}", res.plan_id, res.state);
                for o in &res.outcomes {
                    println!(
                        "  {:<9} {:>10}  {}{}",
                        o.status,
                        swamp_core::render::human_bytes_pub(o.planned_bytes),
                        o.path.display(),
                        o.cause
                            .as_ref()
                            .map(|c| format!("  — {c}"))
                            .unwrap_or_default()
                    );
                    for kept in &o.preserved {
                        println!("            kept {}", kept.display());
                    }
                }
                let permanent = if res.removed_permanently_bytes > 0 {
                    format!(
                        " · removed permanently {}",
                        swamp_core::render::human_bytes_pub(res.removed_permanently_bytes)
                    )
                } else {
                    String::new()
                };
                println!(
                    "planned {} · trashed {}{permanent} · free space measured {}",
                    swamp_core::render::human_bytes_pub(res.planned_bytes),
                    swamp_core::render::human_bytes_pub(res.trashed_bytes),
                    res.freed_measured
                        .map(swamp_core::render::human_bytes_signed)
                        .unwrap_or_else(|| "n/a".into())
                );
                if let Some(n) = &res.next_step {
                    println!("next: {n}");
                }
            }
        }
        Command::Plans { json } => {
            let plans = swamp_core::actions::list_plans(&swamp_dir())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plans)?);
            } else if plans.is_empty() {
                println!("no plans");
            } else {
                for p in plans {
                    println!(
                        "{}  {:?}  {} units  {}  created {}  expires {}",
                        p.id,
                        p.status,
                        p.units.len(),
                        swamp_core::render::human_bytes_pub(p.planned_bytes()),
                        p.created_at,
                        p.expires_at
                    );
                }
            }
        }
        Command::Grant { cmd } => {
            let dir = swamp_dir();
            match cmd {
                GrantCmd::Add {
                    predicate,
                    budget,
                    expires,
                    max_units,
                } => {
                    let budget_bytes = parse_size_arg(&budget)?;
                    let expires_secs = swamp_core::growth::parse_duration_secs(&expires)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bad --expires {expires:?} (e.g. 7d, 12h)")
                        })?;
                    let g = swamp_core::actions::add_standing_grant(
                        &dir,
                        &predicate,
                        budget_bytes,
                        max_units,
                        expires_secs,
                        "human:cli",
                    )?;
                    println!(
                        "grant {} added: delete where {} · budget {} · expires {}",
                        g.id, g.predicate, budget, expires
                    );
                }
                GrantCmd::List => {
                    let gs = swamp_core::actions::list_grants(&dir)?;
                    if gs.is_empty() {
                        println!("no grants");
                    }
                    for g in gs {
                        println!(
                            "{}  {}  {}  budget {} spent {}  units {}/{}  expires {}  by {}",
                            g.id,
                            if g.revoked { "revoked" } else { "live" },
                            g.plan_id
                                .as_ref()
                                .map(|p| format!("plan {p}"))
                                .unwrap_or_else(|| format!("where {}", g.predicate)),
                            swamp_core::render::human_bytes_pub(g.budget_bytes.unwrap_or(0)),
                            swamp_core::render::human_bytes_pub(g.spent_bytes),
                            g.used_units,
                            g.max_units
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| "∞".into()),
                            g.expires_at,
                            g.actor
                        );
                    }
                }
                GrantCmd::Revoke { grant_id } => {
                    swamp_core::actions::revoke_grant(&dir, &grant_id)?;
                    println!("grant {grant_id} revoked");
                }
            }
        }
        Command::Observe { roots, full } => {
            schedule::cmd_observe(swamp_dir(), roots, full)?;
        }
        Command::Schedule { every, off, roots } => {
            schedule::cmd_schedule(swamp_dir(), every, off, roots)?;
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
/// in the report, mirroring `render_project_tree`'s `None` case.
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
            let mut rows: Vec<&swamp_core::report::DirRollup> = dirs_by_worktree
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
                let track = row
                    .track
                    .map(|t| format!("  [{}]", t.label()))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  {label:<40} {:>12} bytes{growth}  changed {}{track}",
                    row.allocated_total,
                    age_from_mod_time_min(row.mod_time_min)
                );
            }

            let mut file_rows: Vec<&swamp_core::report::FileRow> = files_by_worktree
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

/// A one-line stderr progress readout while a walk runs: bytes and
/// directories seen so far from `walk::progress`, redrawn in place ten
/// times a second, erased when done. Off when stderr is not a terminal or
/// the caller wants machine output.
struct ProgressLine {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ProgressLine {
    fn stop(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn spawn_progress_line(enabled: bool) -> ProgressLine {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !enabled {
        return ProgressLine { stop, handle: None };
    }
    let flag = stop.clone();
    let handle = std::thread::spawn(move || {
        use std::io::Write;
        let mut drew = false;
        while !flag.load(std::sync::atomic::Ordering::Relaxed) {
            let (bytes, dirs, active) = swamp_core::walk::progress::snapshot();
            if active {
                let _ = write!(
                    std::io::stderr(),
                    "\r\x1b[2Kobserving… {} · {dirs} dirs",
                    swamp_core::render::human_bytes_pub(bytes)
                );
                drew = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if drew {
            let _ = write!(std::io::stderr(), "\r\x1b[2K");
        }
        let _ = std::io::stderr().flush();
    });
    ProgressLine {
        stop,
        handle: Some(handle),
    }
}
