#![cfg_attr(
    not(test),
    deny(clippy::disallowed_methods, clippy::disallowed_types, unsafe_code)
)]

mod schedule;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use swamp_core::{
    artifact::{ArtifactRole, NestedArtifact},
    filter,
    render::{
        OverviewSort, render_kinds, render_overview_sorted, render_project_tree_with_agents,
        render_types, render_view_builds, render_view_deps, render_view_docker,
        render_view_reconciliation, render_view_unowned, render_worktree_signals, render_worktrees,
    },
    report::{Report, report_full_mode},
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
    /// Nested Cargo target/build units with physical-accounting and
    /// evidence/unknown details. Inspection only.
    Rust,
    /// Ranked list of every discovered project: name, id, total bytes,
    /// growth, checkout+worktree count, remote. JSON only.
    Projects,
    /// Rows with growth > 0 since the window, sorted desc, plus an
    /// unowned-bytes summary and coverage (walked/du/unowned totals,
    /// permission-denied count, history span). JSON only.
    Grown,
    /// External/shared storage units (#43): Cargo registry, rustup
    /// toolchains, Homebrew, and other detector-resolved locations with
    /// no containing project. Identity, category, size/growth/regrowth
    /// history and declared consumers. Inspection only -- see `propose
    /// --external`.
    External,
    /// Agent-tool storage (#91-#99/#100): sessions, caches, logs,
    /// checkpoints and protected config for every named coding-agent
    /// tool (Claude Code, Codex, Oh My Pi, OpenCode, Gemini CLI, Pi,
    /// Aider, GitHub Copilot CLI, Cursor, Windsurf, Cline, Roo Code,
    /// Continue), grouped tool → category → unit with size/growth/age
    /// and project linkage. `--project` filters to units linked to that
    /// project. Redaction-aware by construction (this view never has
    /// session content to print). Inspection only from this command;
    /// supported cleanup actions go through `swamp propose-agents` (see
    /// below) -- see `swamp protect` for the human-keep-intent surface
    /// this view respects.
    Agents,
}

impl View {
    /// The name this view is addressed by in `--view` and echoed back in
    /// `report --json`'s `"view"` field -- derived from clap's own
    /// kebab-case rendering of the variant so the flag value and the
    /// JSON contract never drift apart.
    fn name(self) -> String {
        use clap::ValueEnum;
        self.to_possible_value()
            .map(|v| v.get_name().to_string())
            .unwrap_or_else(|| "worktrees".to_string())
    }
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
    /// Review Cargo cleanup groups, oldest modified first, then largest. Creates unapproved plans;
    /// never authorizes or deletes. Reports blocked groups without widening scope.
    CleanupCheck {
        root: PathBuf,
        /// Exact Cargo group paths from a Rust report. Categories are not expanded.
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        /// Restrict to one role, for example test-executable or incremental.
        #[arg(long, value_parser = ["test-executable", "example", "incremental", "build-script-output"])]
        role: Option<String>,
        /// Maximum groups to check (1–20). This is not an exhaustive cleanup search.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// Skip this many age-ranked candidates. Pages may shift after a rebuild.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Review individual groups within this directory, never the directory itself.
        #[arg(long)]
        within: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Diffstat-ledger terminal UI (ratatui). Default when no
    /// subcommand is given.
    Ui {
        /// Defaults to the configured effective scope's first present
        /// root when omitted (see `swamp scope`); an explicit root
        /// still replaces the configured scope for this invocation.
        root: Option<PathBuf>,
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
        /// Defaults to the configured effective scope's first present
        /// root when omitted (see `swamp scope`); an explicit root
        /// still replaces the configured scope for this invocation,
        /// though configured exclusions still apply.
        root: Option<PathBuf>,
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
        /// reconciliation, rust. Every question the issue lists is exactly one
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
        /// Bound a JSON array-shaped result (the `result` array with
        /// `--view`, or the `projects` array without one) to this many
        /// rows. Only consulted with `--json`; the envelope's `total`
        /// and `truncated` fields say whether this is a partial page.
        #[arg(long)]
        limit: Option<usize>,
        /// Skip this many rows of a JSON array-shaped result before
        /// applying `--limit`. Only consulted with `--json`.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// With `--view docker --json`, restrict to Docker objects with
        /// no join evidence to a project (never attributed by name
        /// similarity in this mode).
        #[arg(long)]
        unowned_only: bool,
    },
    /// Observe-only: walk `root`s, write the growth store, and refresh
    /// GitHub enrichment live for every GitHub-remote worktree found
    /// (concurrent, coalesced per repo -- see `github::observe_all`). No
    /// rendering. This is what a scheduled LaunchAgent run executes, and
    /// the only `swamp` command that calls `gh` on your behalf by
    /// default; `report` reads whatever this last wrote.
    Observe {
        /// Defaults to every present root in the configured effective
        /// scope when omitted (see `swamp scope`); explicit roots still
        /// replace the configured scope for this invocation, though
        /// configured exclusions still apply.
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
    /// Propose cleanup: the one entry point for artifact/Cargo-group/
    /// worktree paths (with `root`), agent-storage units, or external
    /// units (with `--path` and no `root`) -- see `--path`'s own help
    /// for exactly how a bare path is routed. Review paths, sizes,
    /// warnings and recovery before authorizing. Use report --view
    /// worktrees for worktree signals; for individual Cargo builds,
    /// start with cleanup-check. Nothing is deleted by this command.
    #[command(
        after_help = "Whole-worktree example:\n  swamp report ~/src --view worktrees\n  swamp propose ~/src --path /absolute/path/to/a-worktree\n\nAgent-storage or external-unit example (no root needed):\n  swamp report --view agents\n  swamp propose --path /absolute/path/to/a/session/or/unit\n\nExternal-unit inspection (never actionable; refused at execution):\n  swamp propose --external --path /absolute/path/to/an/external/unit\n\nReview the plan; proposing never authorizes removal."
    )]
    Propose {
        /// A walked report's root. Omit it entirely when every `--path`
        /// names an agent-storage unit (`report --view agents`) or an
        /// external unit (`report --view external`) instead of a
        /// filesystem artifact/worktree -- those are detector-resolved,
        /// not root-relative, and this command finds them the same way
        /// `report --view agents|external` does, without a root.
        root: Option<PathBuf>,
        /// Narrow to rows matching this filter, e.g. "kind:BuildOutput idle > 30d".
        /// Only meaningful with a `root` (filesystem artifacts).
        #[arg(long)]
        filter: Option<String>,
        /// Exact artifact/Cargo-group/worktree path (with `root`), or an
        /// exact agent-storage/external unit path (without `root`) --
        /// routed automatically: an agent-storage unit match takes
        /// priority, then an external unit, then (only with `root`) a
        /// filesystem artifact/Cargo-group/worktree. Use `--external` to
        /// force the external-unit route explicitly.
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
        /// Force the external-unit route (never agent-storage or
        /// filesystem): every resulting unit is inspection-only and
        /// `execute` refuses it unconditionally -- this exists to
        /// review external storage through the plan/ledger surface,
        /// never to make it actionable (#43/#101). With no `--path`,
        /// proposes every discovered external unit.
        #[arg(long, conflicts_with = "root")]
        external: bool,
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
    /// Print the effective scan scope (#41): every root swamp would use
    /// for this invocation, its status (present/missing/unreadable/
    /// skipped-as-nested/excluded) and every reason it is in scope --
    /// built-in default, detector (with id/category/provenance),
    /// configured include, or explicit command root -- plus the full
    /// detector catalog (including disabled/not-present/unresolved
    /// entries) and the detector catalog version. With explicit roots,
    /// shows what those roots resolve to (configured exclusions still
    /// apply) instead of the configured scope. This is the one shared
    /// resolution every scope-aware command (`report`, `observe`, `ui`,
    /// `schedule`) uses when no explicit root is given -- never a
    /// separate ad hoc computation.
    Scope {
        roots: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// List plans (newest first).
    Plans {
        #[arg(long)]
        json: bool,
    },
    /// Standing grants: `grant add '<predicate>' --budget 5GB --expires 7d`,
    /// `grant list [--json]`, `grant revoke <id>`. Predicates: kind:,
    /// project:, idle > <dur>, merge-complete. Reserved for a human at
    /// this keyboard by convention, not by a technical wall: see
    /// `.oh/guardrails/human-only-authorization.md`.
    Grant {
        #[command(subcommand)]
        cmd: GrantCmd,
    },
    /// Deprecated alias for `swamp propose --path <unit>` (no `root`)
    /// (#101 unification): kept only so existing scripts/muscle memory
    /// keep working. Propose cleanup for exact agent-storage unit paths:
    /// a cache/log category directory, or an individual session's own
    /// transcript path from `report --view agents`. Every match becomes
    /// a real action or a named refusal (protected, unsupported
    /// category, database-like file, active session) -- never a
    /// silent inspection-only row. Approve/execute the resulting plan
    /// with the same `swamp approve`/`swamp execute` used for every
    /// other plan. Nothing is deleted by this command.
    ProposeAgents {
        /// Exact agent-storage unit paths from `report --view agents`.
        #[arg(long = "path", required = true)]
        paths: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Human keep/protect intent for agent-storage paths (#100/#101):
    /// survives refresh, blocks `propose-agents`/`execute` for any unit
    /// under a protected path, and is never itself inferred from
    /// observation -- only this command changes it.
    Protect {
        #[command(subcommand)]
        cmd: ProtectCmd,
    },
}

#[derive(Subcommand)]
enum ProtectCmd {
    Add {
        path: PathBuf,
    },
    Remove {
        path: PathBuf,
    },
    List {
        #[arg(long)]
        json: bool,
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
    List {
        #[arg(long)]
        json: bool,
    },
    Revoke {
        grant_id: String,
    },
}

fn parse_size_arg(s: &str) -> Result<u64> {
    // The one size parser (`filter::parse_size`): decimal and binary
    // units, the same base the formatter prints in.
    swamp_core::filter::parse_size(s).ok_or_else(|| anyhow::anyhow!("bad size {s:?}"))
}

fn print_plan(plan: &swamp_core::actions::Plan) {
    println!(
        "plan {}  {} units  {}  expires in {}s",
        plan.id,
        plan.units().len(),
        swamp_core::render::human_bytes_pub(plan.planned_bytes()),
        plan.expires_at()
            .saturating_sub(swamp_core::entities::now())
    );
    for u in plan.units() {
        println!(
            "  {:<14} {:>10}  {:>+10}  {}  {}  [{}]  {}",
            format!("{:?}", u.kind()).to_lowercase(),
            swamp_core::render::human_bytes_pub(u.bytes()),
            u.growth_bytes()
                .map(swamp_core::render::human_bytes_signed)
                .unwrap_or_else(|| "—".into()),
            u.project(),
            u.path().display(),
            u.recovery(),
            u.signals().join(" · ")
        );
        if let Some(t) = u.track() {
            print!("    [{}]", t.label());
        }
        if !u.warnings().is_empty() {
            print!("  ⚠ {}", u.warnings().join(" · "));
        }
        if u.track().is_some() || !u.warnings().is_empty() {
            println!();
        }
    }
    for r in plan.refused() {
        println!("  refused  {}  — {}", r.path.display(), r.cause);
    }
    println!(
        "authorize (human only): {}",
        swamp_core::actions::approve_command(&plan.id)
    );
}

/// `${SWAMP_DIR}`, defaulting to `~/.local/share/swamp`.
/// The resolved swamp dir (`$SWAMP_DIR`, else `~/.local/share/swamp`):
/// the gate's one resolver, `fs_gate::store::StoreDir::resolved`.
fn swamp_dir() -> PathBuf {
    swamp_core::fs_gate::store::StoreDir::resolved()
        .path()
        .to_path_buf()
}

/// The one shared resolution every scope-aware command (#41) goes
/// through: built-in defaults, detector results, and configured
/// include/exclude/disabled-detectors, or -- when `explicit` is
/// non-empty -- exactly those roots (configured exclusions still
/// apply). A malformed `config.toml` is a hard error here (nonzero
/// exit via `main`'s `Result`, message on stderr): scope resolution
/// never silently falls back to a broader default on invalid config.
fn resolve_scope(explicit: &[PathBuf]) -> Result<swamp_core::scope::EffectiveScope> {
    let store_dir = swamp_dir();
    let cfg = swamp_core::growth::load_config_checked(&store_dir)?;
    let env = swamp_core::locations::Environment::from_process();
    let registry = swamp_core::locations::Registry::with_builtins();
    Ok(swamp_core::scope::resolve_effective_scope(
        &env,
        &cfg.scan,
        explicit,
        &registry,
        swamp_core::entities::now(),
    ))
}

/// For the single-root commands (`report`, `ui`) that have not yet
/// adopted full multi-root observation (#42/#50 -- see
/// `.oh/sessions/2026-09-21-scope-and-detector-registry.md`): resolves
/// the configured scope and picks its first present root, noting on
/// stderr when more than one root is actually in scope. Multi-root
/// commands (`observe`, `schedule`) use `resolve_scope(...).scan_paths()`
/// directly instead of this function.
fn resolve_single_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    let explicit_roots: Vec<PathBuf> = explicit.into_iter().collect();
    let scope = resolve_scope(&explicit_roots)?;
    if !explicit_roots.is_empty() {
        return match scope.roots.first() {
            Some(r)
                if matches!(
                    r.status,
                    swamp_core::scope::RootStatus::Present | swamp_core::scope::RootStatus::Missing
                ) =>
            {
                Ok(r.path.clone())
            }
            Some(r) => anyhow::bail!(
                "root {} is not in scope ({:?}); configured exclusions apply to explicit roots too -- see `swamp scope --json`",
                r.path.display(),
                r.status
            ),
            None => anyhow::bail!("no root given"),
        };
    }
    let present = scope.scan_paths();
    if present.is_empty() {
        if scope.is_empty_scope() {
            anyhow::bail!(
                "effective scan scope is empty: no built-in default, detector, or configured include is enabled. This is explicit, not a fallback to the current directory -- see `swamp scope --json`, or pass a root explicitly."
            );
        }
        anyhow::bail!(
            "configured scope has no present root (every candidate is missing/unreadable/excluded) -- see `swamp scope --json`, or pass a root explicitly."
        );
    }
    if present.len() > 1 {
        eprintln!(
            "note: configured scope has {} present roots; using {} (the first). Full multi-root reporting is #50's job -- see `swamp scope --json`, or `swamp observe`/`swamp schedule` for multi-root observation.",
            present.len(),
            present[0].display()
        );
    }
    Ok(present[0].clone())
}

/// Persists the just-resolved scope and, when a previous one exists,
/// prints a one-line coverage-change note to stderr (#41's "explain
/// effective coverage and baseline changes"). This never touches byte
/// history: it is coverage bookkeeping only, per
/// `.oh/guardrails/coverage-changes-are-not-storage-changes.md`.
fn note_and_persist_scope(store_dir: &Path, scope: &swamp_core::scope::EffectiveScope) {
    if let Some(previous) = swamp_core::scope::load_last_effective_scope(store_dir) {
        let changes = swamp_core::scope::coverage_changes(&previous, scope);
        if !changes.is_empty() {
            let summary = changes
                .iter()
                .map(|c| {
                    let sign = match c.kind {
                        swamp_core::scope::CoverageChangeKind::Added => '+',
                        swamp_core::scope::CoverageChangeKind::Removed => '-',
                    };
                    format!("{sign}root {} ({})", c.path.display(), c.reason)
                })
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("coverage changed since last observation: {summary}");
        }
    }
    if let Err(e) = swamp_core::scope::persist_effective_scope(store_dir, scope) {
        eprintln!("note: could not persist effective scope for next run: {e}");
    }
}

/// One line per non-`Complete` root from a `report_scope` call (#42):
/// missing/excluded/inaccessible/partial regions, so a coherent
/// multi-root observation never silently under-reports without saying
/// why. A `Complete` root prints nothing -- the ordinary case should not
/// be noisy.
fn print_scope_coverage_note(coverage: &[swamp_core::coverage::RootCoverage]) {
    use swamp_core::coverage::RegionStatus;
    let incomplete: Vec<&swamp_core::coverage::RootCoverage> = coverage
        .iter()
        .filter(|c| !matches!(c.status, RegionStatus::Complete))
        .collect();
    if incomplete.is_empty() {
        return;
    }
    let summary = incomplete
        .iter()
        .map(|c| format!("{} ({})", c.path.display(), c.status.label()))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("scope coverage: {summary}");
}

fn render_scope_text(scope: &swamp_core::scope::EffectiveScope) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "catalog {} · defaults={} · disabled=[{}] · {}",
        scope.catalog_version,
        scope.defaults_enabled,
        scope.disabled_detectors.join(", "),
        if scope.explicit {
            "explicit roots (configured exclusions still apply)"
        } else {
            "configured scope"
        }
    );
    for root in &scope.roots {
        let status = match &root.status {
            swamp_core::scope::RootStatus::Present => "present".to_string(),
            swamp_core::scope::RootStatus::Missing => "missing".to_string(),
            swamp_core::scope::RootStatus::Unreadable { reason } => {
                format!("unreadable ({reason})")
            }
            swamp_core::scope::RootStatus::SkippedAsNested { parent } => {
                format!("skipped-as-nested (folded into {})", parent.display())
            }
            swamp_core::scope::RootStatus::Excluded { pattern } => format!("excluded ({pattern})"),
        };
        let reasons: Vec<String> = root
            .reasons
            .iter()
            .map(|r| match r {
                swamp_core::scope::RootReason::BuiltinDefault => "built-in default".to_string(),
                swamp_core::scope::RootReason::Detector {
                    detector_id,
                    category,
                    provenance,
                } => format!("detector:{detector_id} ({category:?}, {provenance:?})"),
                swamp_core::scope::RootReason::Included => "include".to_string(),
                swamp_core::scope::RootReason::ExplicitCommand => "explicit".to_string(),
                swamp_core::scope::RootReason::NestedFrom { path } => {
                    format!("covers nested {}", path.display())
                }
            })
            .collect();
        let _ = writeln!(
            out,
            "  {:<10} {}  [{}]",
            status,
            root.path.display(),
            reasons.join("; ")
        );
    }
    if !scope.pruned_subtrees.is_empty() {
        let _ = writeln!(out, "pruned subtrees (excluded, inside an in-scope root):");
        for p in &scope.pruned_subtrees {
            let _ = writeln!(out, "  {} under {}", p.pattern, p.root.display());
        }
    }
    let _ = writeln!(out, "detectors:");
    for d in &scope.detectors {
        for loc in d.locations_for_display() {
            let status = match &loc.status {
                swamp_core::locations::LocationStatus::Resolved => "resolved".to_string(),
                swamp_core::locations::LocationStatus::NotPresent => "not-present".to_string(),
                swamp_core::locations::LocationStatus::Disabled => "disabled".to_string(),
                swamp_core::locations::LocationStatus::UnresolvedWithReason { reason } => {
                    format!("unresolved ({reason})")
                }
            };
            let path = loc
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "-".to_string());
            let _ = writeln!(out, "  {:<12} {:<10} {}", d.detector_id, status, path);
        }
    }
    out
}

/// Human authorization for one plan: prints every unit with its facts,
/// then writes a one-shot grant scoped to that plan id.
///
/// This is one of exactly two call sites in the whole workspace allowed
/// to reach `swamp_core::actions::approve`/`add_standing_grant`/
/// `revoke_grant` (the other is the TUI's confirmed-execution path,
/// `crates/tui/src/actions.rs`'s `execute_one`) -- enforced by the
/// `human_only_authorization` source audit, which is transport-
/// independent by construction: it does not matter that this function
/// happens to live in the CLI binary, only that authorization is minted
/// from exactly this reviewed call site and no other. Any shell-capable
/// process can invoke `swamp approve`/`swamp grant add`; the boundary
/// this enforces is "authorization is minted from one reviewed code
/// path", not "only a human process can reach this binary". See
/// `.oh/guardrails/human-only-authorization.md`.
fn cmd_approve(plan_id: &str) -> Result<()> {
    let plan = swamp_core::actions::load_plan(&swamp_dir(), plan_id)?;
    for u in plan.units() {
        println!(
            "  {:<16} {:>10}  {}{}",
            u.verb(),
            swamp_core::render::human_bytes_pub(u.bytes()),
            u.path().display(),
            if u.warnings().is_empty() {
                String::new()
            } else {
                format!("  ⚠ {}", u.warnings().join(" · "))
            }
        );
    }
    // The human typed `swamp approve <plan>` and was shown exactly these
    // units: the reviewed CLI confirmation site, bound to this plan's id
    // and content digest (`.oh/guardrails/human-only-authorization.md`).
    let confirmed = swamp_core::authority::HumanConfirmed::cli_approve("human:cli", &plan);
    let g = swamp_core::actions::approve_confirmed(&swamp_dir(), plan_id, confirmed)?;
    println!(
        "approved plan {} with one-shot grant {} (budget {}, {} units, expires {})",
        plan_id,
        g.id(),
        swamp_core::render::human_bytes_pub(g.budget_bytes().unwrap_or(0)),
        g.max_units().unwrap_or(0),
        g.expires_at()
    );
    println!("execute with: swamp execute {plan_id}");
    Ok(())
}

/// Human authorization for a standing grant. See `cmd_approve`'s doc for
/// why this call site's identity matters to the `human_only_authorization`
/// audit.
fn cmd_grant_add(
    dir: &Path,
    predicate: &str,
    budget: &str,
    expires: &str,
    max_units: Option<u32>,
) -> Result<()> {
    let budget_bytes = parse_size_arg(budget)?;
    let expires_secs = swamp_core::growth::parse_duration_secs(expires)
        .ok_or_else(|| anyhow::anyhow!("bad --expires {expires:?} (e.g. 7d, 12h)"))?;
    // The human typed `swamp grant add …`: the reviewed CLI confirmation
    // site, bound to exactly these terms
    // (`.oh/guardrails/human-only-authorization.md`).
    let confirmed = swamp_core::authority::HumanConfirmed::cli_grant(
        "human:cli",
        swamp_core::authority::StandingTerms {
            predicate: predicate.to_string(),
            budget_bytes,
            max_units,
            expires_in_secs: expires_secs,
        },
    );
    let g = swamp_core::actions::add_standing_grant_confirmed(
        dir,
        predicate,
        budget_bytes,
        max_units,
        expires_secs,
        confirmed,
    )?;
    println!(
        "grant {} added: delete where {} · budget {} · expires {}",
        g.id(),
        g.predicate(),
        budget,
        expires
    );
    Ok(())
}

/// `swamp protect add|remove|list`. A keep-list change is human-only
/// (re-review 5, finding 7): this is the one reviewed site that mints the
/// confirmation for it, bound to exactly the change the human typed, and
/// `protect_{add,remove}_confirmed` spend it.
fn cmd_protect(cmd: ProtectCmd) -> Result<()> {
    let store_dir = swamp_dir();
    match cmd {
        ProtectCmd::Add { path } => {
            // Resolve a relative argument against the cwd before
            // storing, rather than printing "protected: debug" for an
            // entry that protects nothing (the 2026-09-22 re-review's
            // CE5). `protect_add_confirmed` refuses a non-absolute path
            // outright; doing the join here means `swamp protect add
            // debug` from inside a tool home does the obvious thing and
            // *says* which path it protected.
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()?.join(&path)
            };
            let confirmed = swamp_core::authority::HumanConfirmed::cli_protect(
                "human:cli",
                swamp_core::authority::ProtectChange::Add(resolved.clone()),
            );
            swamp_core::agents::protect_add_confirmed(&store_dir, &resolved, confirmed)?;
            println!("protected: {}", resolved.display());
        }
        ProtectCmd::Remove { path } => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()?.join(&path)
            };
            let confirmed = swamp_core::authority::HumanConfirmed::cli_protect(
                "human:cli",
                swamp_core::authority::ProtectChange::Remove(resolved.clone()),
            );
            swamp_core::agents::protect_remove_confirmed(&store_dir, &resolved, confirmed)?;
            println!("no longer protected: {}", resolved.display());
        }
        ProtectCmd::List { json } => {
            let listing = swamp_core::agents::protect_listing(&store_dir)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&listing)?);
            } else if listing.is_empty() {
                println!("no protected agent-storage paths");
            } else {
                println!("{listing}");
            }
        }
    }
    Ok(())
}

/// Human revocation of a standing or one-shot grant. See `cmd_approve`'s
/// doc for why this call site's identity matters to the
/// `human_only_authorization` audit.
fn cmd_grant_revoke(dir: &Path, grant_id: &str) -> Result<()> {
    swamp_core::actions::revoke_grant(dir, grant_id)?;
    println!("grant {grant_id} revoked");
    Ok(())
}

const CLEANUP_COVERAGE_DETAIL_ROWS: usize = 20;
const CLEANUP_COVERAGE_DETAIL_LIMITS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
struct CleanupCoverageDetail {
    path: PathBuf,
    limits: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct CleanupScopeSummary {
    nested_row_count: usize,
    coverage_limited_count: usize,
    unknown_or_residual_count: usize,
    coverage_limited_details: Vec<CleanupCoverageDetail>,
}

fn cleanup_path_in_scope(path: &Path, within: Option<&Path>) -> bool {
    within.is_none_or(|scope| path != scope && path.starts_with(scope))
}

fn cleanup_path_at_or_below_scope(path: &Path, within: Option<&Path>) -> bool {
    within.is_none_or(|scope| path == scope || path.starts_with(scope))
}

fn cleanup_scope_summary(
    nested_artifacts: &[NestedArtifact],
    within: Option<&Path>,
) -> CleanupScopeSummary {
    let mut summary = CleanupScopeSummary {
        nested_row_count: 0,
        coverage_limited_count: 0,
        unknown_or_residual_count: 0,
        coverage_limited_details: Vec::new(),
    };
    for unit in nested_artifacts {
        let in_scope = cleanup_path_at_or_below_scope(&unit.path, within);
        // An incomplete Cargo container above `within` limits what can be
        // established inside the requested scope, so retain that evidence
        // without counting the ancestor as a row inside the scope.
        let relevant_ancestor = within.is_some_and(|scope| {
            unit.path != scope
                && scope.starts_with(&unit.path)
                && (!unit.coverage.complete || !unit.coverage.supported)
        });
        if in_scope {
            summary.nested_row_count += 1;
            if matches!(unit.role, ArtifactRole::Unknown | ArtifactRole::Residual) {
                summary.unknown_or_residual_count += 1;
            }
        }
        if (in_scope || relevant_ancestor) && (!unit.coverage.complete || !unit.coverage.supported)
        {
            summary.coverage_limited_count += 1;
            if summary.coverage_limited_details.len() < CLEANUP_COVERAGE_DETAIL_ROWS {
                summary
                    .coverage_limited_details
                    .push(CleanupCoverageDetail {
                        path: unit.path.clone(),
                        limits: unit
                            .coverage
                            .limits
                            .iter()
                            .take(CLEANUP_COVERAGE_DETAIL_LIMITS)
                            .cloned()
                            .collect(),
                    });
            }
        }
    }
    summary
}

/// Whether `u`'s project linkage names `project` (case-insensitive,
/// matching this codebase's other `--project` matching): only a
/// `Linked` unit can match; every other linkage state (unresolved,
/// missing, not-a-project, moved, remote, shared, not-applicable) is
/// filtered out by a project filter, never silently included. `None`
/// (no filter given) matches everything.
fn agent_unit_matches_project(u: &swamp_core::agents::AgentUnit, project: Option<&str>) -> bool {
    let Some(project) = project else { return true };
    matches!(
        &u.project_link,
        swamp_core::agents::ProjectLinkState::Linked { project_name, .. }
            if project_name.eq_ignore_ascii_case(project)
    )
}

// ---------------------------------------------------------------------
// Unified `propose` entry point (#101): a single CLI command that
// routes `--path` to the right proposer -- agent-storage unit,
// external unit, or (only with an explicit `root`) a filesystem
// artifact/Cargo-group/worktree -- so a human never has to know in
// advance which of three subsystems a path belongs to. `propose-agents`
// is kept only as a thin, deprecated alias into the same code (see
// `Command::ProposeAgents`'s handler below).
// ---------------------------------------------------------------------

/// Agent-storage units for the unified `propose --path` route (no
/// `root`): a real report walk, exactly like `report --view agents`
/// computes, so every known project worktree is available to supply
/// Aider's per-repo units (#96) -- never the narrower "walk upward from
/// each requested path" fast path `propose-agents` used before this
/// chunk, which could not discover an Aider unit whose worktree root
/// was not itself derivable from the requested path (chunk E's
/// follow-up). This costs a full scope walk instead of a handful of
/// `stat`s, which is the deliberate trade #101's correctness
/// requirement makes: `propose` without a `root` is not a hot path.
fn discover_agent_units_for_propose(
    store_dir: &Path,
) -> Result<Vec<swamp_core::agents::AgentUnit>> {
    // Through the one observation that owns discovery, never a second
    // pass of the CLI's own
    // (`.oh/guardrails/discovery-owned-by-report-pipeline.md`). It walks
    // for the worktree roots Aider's per-repo units need and runs the
    // agent pass with the same ownership window, so nothing here can
    // tombstone a row the walk did not cover.
    Ok(observe_for_cli(
        &resolve_scope(&[])?,
        swamp_core::report::ObservationParts::AGENTS,
        None,
        store_dir,
        true,
        3600,
    )?
    .agent_units)
}

/// The CLI's one route into `report::observe_scope`, with this binary's
/// fixed arguments (no Docker facts, no `du` verification, no directory
/// rows, no enrichment) filled in.
fn observe_for_cli(
    scope: &swamp_core::scope::EffectiveScope,
    want: swamp_core::report::ObservationParts,
    base: Option<swamp_core::report::Report>,
    store_dir: &Path,
    observe: bool,
    since_secs: u64,
) -> Result<swamp_core::report::ScopeObservation> {
    swamp_core::report::observe_scope(
        scope,
        want,
        base,
        None,
        false,
        Some(store_dir),
        None,
        observe,
        false,
        false,
        false,
        swamp_core::fs_events::platform_source().as_ref(),
        swamp_core::growth::load_config(store_dir).retention_days,
        since_secs,
    )
}

/// External units for the unified `propose` route: the same
/// detector-resolved discovery `report --view external` uses, with the
/// same live tool-version/dependency association wiring (#56/#57) --
/// a proposed external unit's `PlanUnit.evidence` (#61) should carry the
/// same consumer facts the `report --view external` text/JSON path
/// does, not a narrower answer just because this route takes a
/// different code path to get there.
fn discover_external_units_for_propose(
    store_dir: &Path,
) -> Result<Vec<swamp_core::external::ExternalUnit>> {
    // One observation, which also attaches the live tool-version and
    // dependency associations (#56/#57) -- so a proposed external unit's
    // evidence carries the same consumer facts `report --view external`
    // shows, rather than a narrower answer for taking a different route.
    Ok(observe_for_cli(
        &resolve_scope(&[])?,
        swamp_core::report::ObservationParts::EXTERNAL,
        None,
        store_dir,
        true,
        24 * 3600,
    )?
    .external_units)
}

/// Saves `plan` and prints it (JSON envelope or the plain-text form),
/// the identical tail every `propose`/`propose-agents` branch used to
/// duplicate.
fn save_and_print_plan(
    store_dir: &Path,
    plan: &swamp_core::actions::Plan,
    observed_at: u64,
    json: bool,
) -> Result<()> {
    swamp_core::actions::save_plan(store_dir, plan)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&swamp_core::agent_json::propose_envelope(
                plan,
                observed_at
            )?)?
        );
    } else {
        print_plan(plan);
    }
    Ok(())
}

/// The unified `propose` entry point's full routing logic, shared
/// verbatim between `Command::Propose` and the deprecated
/// `Command::ProposeAgents` alias.
#[allow(clippy::too_many_arguments)]
fn propose_unified(
    root: Option<PathBuf>,
    filter: Option<String>,
    paths: Vec<PathBuf>,
    since: Option<String>,
    json: bool,
    external: bool,
) -> Result<()> {
    let store_dir = swamp_dir();
    if external {
        anyhow::ensure!(
            root.is_none(),
            "--external is only valid without a root: external units are detector-resolved, independent of any walked root"
        );
        let units = discover_external_units_for_propose(&store_dir)?;
        let observed_at = swamp_core::entities::now();
        let plan = swamp_core::actions::propose_external(&units, &paths, "human:cli")?;
        return save_and_print_plan(&store_dir, &plan, observed_at, json);
    }
    if let Some(root) = root {
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
        // Fail closed: unreadable or malformed protection state is
        // *unknown*, and proposing against an empty keep list would
        // silently unprotect every artifact row
        // (`.oh/guardrails/protection-fails-closed.md`).
        let plan = swamp_core::actions::propose_checking_store_protection(
            &r,
            parsed.as_ref(),
            &paths,
            "human:cli",
            &store_dir,
        )?;
        return save_and_print_plan(&store_dir, &plan, r.observed_at, json);
    }
    anyhow::ensure!(
        !paths.is_empty(),
        "either a root (for a filesystem artifact/Cargo-group/worktree) or at least one --path (for an agent-storage or external unit) is required"
    );
    let agent_units = discover_agent_units_for_propose(&store_dir)?;
    let agent_hit = paths
        .iter()
        .any(|p| agent_units.iter().any(|u| &u.path == p));
    if agent_hit {
        let observed_at = swamp_core::entities::now();
        let plan = swamp_core::actions::propose_agents(&agent_units, &paths, "human:cli")?;
        return save_and_print_plan(&store_dir, &plan, observed_at, json);
    }
    let external_units = discover_external_units_for_propose(&store_dir)?;
    let external_hit = paths
        .iter()
        .any(|p| external_units.iter().any(|u| &u.path == p));
    if external_hit {
        let observed_at = swamp_core::entities::now();
        let plan = swamp_core::actions::propose_external(&external_units, &paths, "human:cli")?;
        return save_and_print_plan(&store_dir, &plan, observed_at, json);
    }
    anyhow::bail!(
        "no agent-storage or external unit matched any given --path (checked {} agent-storage unit(s), {} external unit(s)); pass a root to propose a filesystem artifact/Cargo group/worktree instead, or run `swamp report --view agents|external --json` to find the exact unit path",
        agent_units.len(),
        external_units.len()
    );
}

/// The bounded, documented JSON contract behind `report --json` (see
/// `skills/swamp/references/commands-and-json.md`): with `--view`, an
/// envelope `{view, project, result, observed_at, since,
/// index_refreshed, total, truncated}` (plus `coverage` for `--view
/// grown`); without one, the full (optionally project-scoped) report
/// with the same `since`/`index_refreshed`/`total`/`truncated` fields
/// added and its top-level `projects` array bounded by
/// `--limit`/`--offset`. `--filter`, when given, narrows the whole
/// report before any view is computed -- the same order the retired MCP
/// `report` tool applied it in, so a filtered view and a filtered full
/// report agree on what rows exist. This function is the only place
/// that builds `report --json` output; every branch below funnels
/// through it so `--view`/`--project`/`--filter` can never again be
/// silently ignored in JSON mode the way the whole-report dump used to
/// ignore them.
#[allow(clippy::too_many_arguments)]
fn report_json_envelope(
    r: &Report,
    root: &Path,
    view: Option<View>,
    project: Option<&str>,
    parsed_filter: Option<&filter::Filter>,
    since: Option<&str>,
    index_refreshed: bool,
    unowned_only: bool,
    limit: Option<usize>,
    offset: usize,
    scope_coverage: &[swamp_core::coverage::RootCoverage],
    external_units: &[swamp_core::external::ExternalUnit],
    agent_units: &[swamp_core::agents::AgentUnit],
    store_interiors: &[swamp_core::artifact::NestedArtifact],
) -> Result<serde_json::Value> {
    let store_dir = swamp_dir();
    let since_str = swamp_core::agent_json::effective_since(&store_dir, since);
    let mut rr = r.clone();
    if let Some(f) = parsed_filter {
        swamp_core::agent_json::apply_filter_to_report(&mut rr, f);
    }
    let observed_at = rr.observed_at;

    if let Some(v) = view {
        let name = v.name();
        let mut result = match v {
            View::Grown => swamp_core::agent_json::what_grew_payload(&rr, project),
            View::Projects => swamp_core::agent_json::list_projects_payload(&rr, project),
            View::Worktrees => swamp_core::agent_json::list_worktrees_payload(
                &rr,
                &filter::Filter::default(),
                project,
            ),
            View::Docker => {
                swamp_core::agent_json::docker_objects_payload(&rr, unowned_only, project)
            }
            View::Rust => serde_json::json!(rr.nested_artifacts),
            View::External => serde_json::json!({
                "units": external_units,
                "total_bytes": swamp_core::external::total_bytes(external_units),
                // Each machine-wide build store's identified interior,
                // keyed by the external unit's path, in the shape
                // `--view builds --json` uses for a project container.
                "interiors": external_units
                    .iter()
                    .filter_map(|u| {
                        swamp_core::agent_json::interior_json(&u.path, store_interiors)
                            .map(|i| (u.path.display().to_string(), i))
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>(),
            }),
            View::Agents => {
                let filtered: Vec<&swamp_core::agents::AgentUnit> = agent_units
                    .iter()
                    .filter(|u| agent_unit_matches_project(u, project))
                    .collect();
                serde_json::json!({
                    "units": filtered,
                    "total_bytes": filtered.iter().map(|u| u.bytes).sum::<u64>(),
                })
            }
            _ => swamp_core::agent_json::view_payload(&rr, &name, project),
        };
        let page = swamp_core::agent_json::paginate(&mut result, limit, offset);
        let mut envelope = serde_json::json!({
            "view": name,
            "project": project,
            "result": result,
            "observed_at": observed_at,
            "since": since_str,
            "index_refreshed": index_refreshed,
        });
        if let Some(p) = page {
            envelope["total"] = serde_json::json!(p.total);
            envelope["truncated"] = serde_json::json!(p.truncated);
        }
        if !scope_coverage.is_empty() {
            envelope["scope_coverage"] = serde_json::json!(scope_coverage);
        }
        if v == View::Docker && project.is_none() {
            envelope["buildkit"] = swamp_core::agent_json::buildkit_payload(&rr);
        }
        if v == View::Grown {
            envelope["coverage"] = serde_json::json!({
                "walked_total": rr.reconciliation.walked_total,
                "du_total": rr.reconciliation.du_total,
                "unowned_total": rr.reconciliation.unowned,
                "attributed_total": rr.reconciliation.attributed,
                "observed_at": observed_at,
                "since": since_str,
                "index_refreshed": index_refreshed,
                "history": swamp_core::agent_json::history_block(&store_dir, root, since),
            });
        }
        return Ok(envelope);
    }

    if let Some(name) = project {
        swamp_core::agent_json::scope_to_project(&mut rr, name);
    }
    let mut value = serde_json::to_value(&rr)?;
    value["since"] = serde_json::json!(since_str);
    value["index_refreshed"] = serde_json::json!(index_refreshed);
    if !scope_coverage.is_empty() {
        value["scope_coverage"] = serde_json::json!(scope_coverage);
    }
    // `--project NAME --json` (no `--view`): include this project's own
    // linked agent-storage units inline, same linkage-state contract as
    // `--view agents --project NAME` (#100's "the CLI `report --project
    // X --json` includes linked agent units with linkage states" --
    // this used to be silently absent whenever `--view agents` was not
    // also passed).
    if project.is_some() && !agent_units.is_empty() {
        let filtered: Vec<&swamp_core::agents::AgentUnit> = agent_units
            .iter()
            .filter(|u| agent_unit_matches_project(u, project))
            .collect();
        value["agent_storage"] = serde_json::json!({
            "units": filtered,
            "total_bytes": filtered.iter().map(|u| u.bytes).sum::<u64>(),
        });
    }
    if let Some(mut projects) = value.get("projects").cloned() {
        let page = swamp_core::agent_json::paginate(&mut projects, limit, offset);
        value["projects"] = projects;
        if let Some(p) = page {
            value["total"] = serde_json::json!(p.total);
            value["truncated"] = serde_json::json!(p.truncated);
        }
    }
    Ok(value)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Ui {
        root: None,
        no_observe: false,
    }) {
        Command::Ui { root, no_observe } => {
            if let Some(explicit) = root {
                swamp_tui::run(&explicit, no_observe)?;
            } else {
                // No explicit root: the TUI opens the *whole* configured
                // multi-root scope (#51) -- project/shared/external/
                // agent-tool storage from every present root at once,
                // including a root with no Git checkout in it at all,
                // not just the first present root the CLI used to pick
                // before the TUI even started.
                let scope = resolve_scope(&[])?;
                if scope.scan_paths().is_empty() {
                    if scope.is_empty_scope() {
                        anyhow::bail!(
                            "effective scan scope is empty: no built-in default, detector, or configured include is enabled. This is explicit, not a fallback to the current directory -- see `swamp scope --json`, or pass a root explicitly."
                        );
                    }
                    anyhow::bail!(
                        "configured scope has no present root (every candidate is missing/unreadable/excluded) -- see `swamp scope --json`, or pass a root explicitly."
                    );
                }
                if !no_observe {
                    note_and_persist_scope(&swamp_dir(), &scope);
                }
                swamp_tui::run_scope(&scope, no_observe)?;
            }
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
            limit,
            offset,
            unowned_only,
        } => {
            let explicit_root = root.clone();
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
            // An explicit root replaces the configured scope entirely and
            // stays on the single-root path (#42's "a single explicit
            // root is just a scope of one"); with no explicit root, the
            // whole configured scope is observed coherently in one call
            // (#42/#50) instead of only its first present root.
            let (r, coverage) = if let Some(explicit) = &explicit_root {
                let root = resolve_single_root(Some(explicit.clone()))?;
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
                (r, Vec::new())
            } else {
                let scope = resolve_scope(&[])?;
                if scope.scan_paths().is_empty() {
                    if scope.is_empty_scope() {
                        anyhow::bail!(
                            "effective scan scope is empty: no built-in default, detector, or configured include is enabled. This is explicit, not a fallback to the current directory -- see `swamp scope --json`, or pass a root explicitly."
                        );
                    }
                    anyhow::bail!(
                        "configured scope has no present root (every candidate is missing/unreadable/excluded) -- see `swamp scope --json`, or pass a root explicitly."
                    );
                }
                // Coverage notes reflect the whole configured scope
                // (#41's "explain effective coverage and baseline
                // changes").
                if !no_observe {
                    note_and_persist_scope(&store_dir, &scope);
                }
                swamp_core::report::report_scope(
                    &scope,
                    docker_facts.as_deref(),
                    verify_du,
                    Some(&store_dir),
                    since.as_deref(),
                    !no_observe,
                    dirs,
                    enrich,
                    full,
                )
                .map(|(r, c)| (Ok(r), c))
                .unwrap_or_else(|e| (Err(e), Vec::new()))
            };
            progress.stop();
            let r = r?;
            let root = r.root.clone();
            if !coverage.is_empty() {
                print_scope_coverage_note(&coverage);
            }
            // External units (#43) and agent-tool storage (#91/#92/#100)
            // are detector-resolved, not derived from the walked
            // root(s): resolved against the configured scope so both
            // views work the same whether `report` was scoped to the
            // catalog or to an explicit root.
            //
            // They come from `report::observe_scope`, the one
            // observation that owns discovery, with `r` handed in as its
            // base so no second walk happens
            // (`.oh/guardrails/discovery-owned-by-report-pipeline.md`).
            // Before this the CLI ran each pass itself, against a scope
            // it re-resolved locally, in an order nobody declared --
            // which is exactly what let the two tombstone each other's
            // rows in the shared history table.
            //
            // Which parts are asked for is unchanged: external units for
            // `--view external`; agent units for `--view agents` *and*
            // for any project-scoped query, so the project tree's
            // collapsed "Agent storage (linked)" row and
            // `--project NAME --json`'s linked units are never silently
            // missing just because `--view agents` was not also passed
            // (#100's project-linkage acceptance). Aider's per-repo
            // units (#96) need every known worktree root, which `r`
            // already carries.
            let want = swamp_core::report::ObservationParts {
                external: view == Some(View::External),
                agents: view == Some(View::Agents) || project.is_some(),
            };
            let (r, external_units, agent_units, store_interiors) =
                if want == swamp_core::report::ObservationParts::WALK_ONLY {
                    (r, Vec::new(), Vec::new(), Vec::new())
                } else {
                    let observation = observe_for_cli(
                        &resolve_scope(&[])?,
                        want,
                        Some(r),
                        &store_dir,
                        !no_observe,
                        since
                            .as_deref()
                            .and_then(swamp_core::growth::parse_duration_secs)
                            .unwrap_or(24 * 3600),
                    )?;
                    (
                        observation.merged,
                        observation.external_units,
                        observation.agent_units,
                        observation.store_interiors,
                    )
                };
            if !json
                && r.projects
                    .iter()
                    .flat_map(|p| &p.worktrees)
                    .flat_map(|w| &w.artifacts)
                    .any(|a| a.dedup_stale)
            {
                eprintln!(
                    "Unique-byte totals were not recomputed this pass; use --full to reconcile. Allocated sizes are current and may count hardlinks multiple times."
                );
            }
            let parsed_filter = match filter_expr.as_deref().map(filter::parse) {
                Some(Ok(f)) => Some(f),
                Some(Err(e)) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                None => None,
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report_json_envelope(
                        &r,
                        &root,
                        view,
                        project.as_deref(),
                        parsed_filter.as_ref(),
                        since.as_deref(),
                        !no_observe,
                        unowned_only,
                        limit,
                        offset,
                        &coverage,
                        &external_units,
                        &agent_units,
                        &store_interiors,
                    )?)?
                );
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
                    None | Some(View::Worktrees) => {
                        match render_project_tree_with_agents(&r, &name, &agent_units) {
                            Some(text) => print!("{text}"),
                            None => {
                                eprintln!(
                                    "no project named {name:?} found under {}",
                                    root.display()
                                );
                                std::process::exit(1);
                            }
                        }
                    }
                    Some(View::Builds) => print!("{}", render_view_builds(&r, Some(&name))),
                    Some(View::Deps) => print!("{}", render_view_deps(&r, Some(&name))),
                    Some(View::Docker) => print!("{}", render_view_docker(&r, Some(&name))),
                    Some(View::Kinds) => print!("{}", render_kinds(&r)),
                    Some(View::Types) => print!("{}", render_types(&r)),
                    Some(View::Rust) => {
                        print!(
                            "{}",
                            swamp_core::render::render_view_rust_with_limit(
                                &r,
                                Some(&name),
                                if all { None } else { Some(30) }
                            )
                        )
                    }
                    Some(View::Unowned) => print!("{}", render_view_unowned(&r)),
                    Some(View::Reconciliation) => print!("{}", render_view_reconciliation(&r)),
                    Some(View::External) => {
                        print!(
                            "{}",
                            swamp_core::render::render_view_external_with(
                                &external_units,
                                &store_interiors,
                                r.observed_at,
                            )
                        )
                    }
                    Some(View::Agents) => {
                        print!(
                            "{}",
                            swamp_core::render::render_view_agents(
                                &agent_units,
                                Some(&name),
                                all,
                                r.observed_at
                            )
                        )
                    }
                    Some(v @ (View::Projects | View::Grown)) => {
                        eprintln!("--view {} is JSON only; add --json", v.name());
                        std::process::exit(1);
                    }
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
                    Some(View::Rust) => {
                        print!(
                            "{}",
                            swamp_core::render::render_view_rust_with_limit(
                                &r,
                                None,
                                if all { None } else { Some(30) }
                            )
                        )
                    }
                    Some(View::Unowned) => print!("{}", render_view_unowned(&r)),
                    Some(View::Reconciliation) => print!("{}", render_view_reconciliation(&r)),
                    Some(View::External) => {
                        print!(
                            "{}",
                            swamp_core::render::render_view_external_with(
                                &external_units,
                                &store_interiors,
                                r.observed_at,
                            )
                        )
                    }
                    Some(View::Agents) => {
                        print!(
                            "{}",
                            swamp_core::render::render_view_agents(
                                &agent_units,
                                None,
                                all,
                                r.observed_at
                            )
                        )
                    }
                    Some(v @ (View::Projects | View::Grown)) => {
                        eprintln!("--view {} is JSON only; add --json", v.name());
                        std::process::exit(1);
                    }
                    None => print!(
                        "{}",
                        render_overview_sorted(&r, all, verify_du, docker, sort.into(), reverse)
                    ),
                }
            }
        }
        Command::CleanupCheck {
            root,
            paths,
            role,
            limit,
            offset,
            within,
            json,
        } => {
            anyhow::ensure!((1..=20).contains(&limit), "limit must be between 1 and 20");
            let root = swamp_core::fs_gate::canonicalize(root)?;
            anyhow::ensure!(
                paths.is_empty() || (offset == 0 && within.is_none()),
                "--path is an exact selection; do not combine it with --offset or --within"
            );
            let within = within.map(swamp_core::fs_gate::canonicalize).transpose()?;
            if let Some(within) = &within {
                anyhow::ensure!(
                    swamp_core::fs_gate::is_dir(within) && within.starts_with(&root),
                    "--within must be a directory inside the scan root"
                );
            }
            let store = swamp_dir();
            let start = std::time::Instant::now();
            let report = report_full_mode(
                &root,
                None,
                false,
                Some(&store),
                None,
                true,
                false,
                false,
                false,
            )?;
            let report_ms = start.elapsed().as_millis();
            let scope_summary = cleanup_scope_summary(&report.nested_artifacts, within.as_deref());
            let mut candidates: Vec<_> = report
                .nested_artifacts
                .iter()
                .filter(|u| {
                    swamp_core::cargo_cleanup::candidate(u)
                        && role.as_deref().is_none_or(|r| u.role.label() == r)
                        && cleanup_path_in_scope(&u.path, within.as_deref())
                })
                .collect();
            candidates
                .sort_by(|a, b| swamp_core::cargo_cleanup::cleanup_order(a, b, report.observed_at));
            let candidate_count = candidates.len();
            let candidate_allocated_bytes: u64 = candidates.iter().map(|u| u.bytes).sum();
            let selected: Vec<_> = if paths.is_empty() {
                candidates
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|u| u.path.clone())
                    .collect()
            } else {
                paths.clone()
            };
            let next_offset = (paths.is_empty()
                && offset.saturating_add(selected.len()) < candidate_count)
                .then_some(offset.saturating_add(selected.len()));
            let next_page = next_offset.map(|next| {
                let mut args = vec![
                    "swamp".to_string(),
                    "cleanup-check".into(),
                    root.display().to_string(),
                    "--offset".into(),
                    next.to_string(),
                    "--limit".into(),
                    limit.to_string(),
                ];
                if let Some(role) = &role {
                    args.extend(["--role".into(), role.clone()]);
                }
                if let Some(within) = &within {
                    args.extend(["--within".into(), within.display().to_string()]);
                }
                if json {
                    args.push("--json".into());
                }
                args
            });
            if !json {
                eprintln!(
                    "{} candidate groups in scope ({} allocated, not reclaimable space). Checking {} on this page; checks read group contents.",
                    candidate_count,
                    swamp_core::render::human_bytes_pub(candidate_allocated_bytes),
                    selected.len()
                );
            }
            // Empty pages must not silently restart the default selection.
            let results = if selected.is_empty() {
                Vec::new()
            } else {
                swamp_core::cargo_cleanup::check(
                    &report,
                    &store,
                    &selected,
                    role.as_deref(),
                    limit,
                )?
            };
            let considered = candidates
                .iter()
                .filter(|u| results.iter().any(|r| r.path == u.path))
                .count();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "root": root, "store": store, "report_ms": report_ms,
                        "limit": limit, "checked_count": results.len(), "exhaustive": false,
                        "offset": offset, "within": within,
                        "observed_candidate_count": candidate_count,
                        "candidate_allocated_bytes": candidate_allocated_bytes,
                        "scoped_nested_row_count": scope_summary.nested_row_count,
                        "coverage_limited_count": scope_summary.coverage_limited_count,
                        "unknown_or_residual_count": scope_summary.unknown_or_residual_count,
                        "coverage_limited_details": scope_summary.coverage_limited_details.iter().map(|d| serde_json::json!({
                            "path": d.path,
                            "limits": d.limits,
                        })).collect::<Vec<_>>(),
                        "not_checked_in_this_run": candidate_count.saturating_sub(considered),
                        "next_offset": next_offset, "next_page": next_page,
                        "note": "This is a bounded review of candidate groups, not a measure of total cleanup opportunity. A zero candidate count does not mean no cleanup opportunity; coverage-limited rows affecting the scope, including relevant ancestors, and noncandidate rows are reported separately. Allocated bytes are not guaranteed reclaimable. Checks are not evidence of disuse. No cleanup authorized or executed.",
                        "results": results
                    }))?
                );
            } else {
                println!(
                    "Cargo cleanup review · {} groups · report {report_ms} ms",
                    results.len()
                );
                for r in results {
                    println!("{} · {}", r.recommendation, r.consequence);
                    println!(
                        "{} · {} allocated · {} ms\n  {}\n  {}",
                        r.check_status,
                        swamp_core::render::human_bytes_pub(r.allocated_bytes),
                        r.elapsed_ms,
                        r.path.display(),
                        r.message
                    );
                    for warning in r.warnings {
                        println!("  {warning}");
                    }
                    if let Some(id) = r.plan_id {
                        println!("  Unapproved plan: {id} (store {})", store.display());
                    }
                    println!("  Next (arguments): {:?}", r.next_command);
                }
                println!(
                    "Bounded review, not an exhaustive search. Use --role test-executable or --path <exact-group> to narrow it. Allocated bytes are not guaranteed free space. Nothing approved or deleted."
                );
                println!(
                    "{} of {candidate_count} candidate groups were not checked in this run.",
                    candidate_count.saturating_sub(considered)
                );
                println!(
                    "Scope contains {} nested Cargo rows: {} coverage-limited rows affecting scope (including ancestors) and {} unknown/residual non-candidates.",
                    scope_summary.nested_row_count,
                    scope_summary.coverage_limited_count,
                    scope_summary.unknown_or_residual_count
                );
                if scope_summary.coverage_limited_count > 0 {
                    println!(
                        "Coverage-limited rows are not cleanup evidence; refresh before treating this scope as complete."
                    );
                    println!("Coverage-limited details (bounded):");
                    for detail in &scope_summary.coverage_limited_details {
                        let limits = if detail.limits.is_empty() {
                            "limits not recorded".to_string()
                        } else {
                            detail.limits.join(" · ")
                        };
                        println!("  {} — {limits}", detail.path.display());
                    }
                }
                if let Some(args) = next_page {
                    println!("Next page (same SWAMP_DIR, arguments): {args:?}");
                }
            }
        }
        Command::Propose {
            root,
            filter,
            paths,
            since,
            json,
            external,
        } => {
            propose_unified(root, filter, paths, since, json, external)?;
        }
        Command::ProposeAgents { paths, json } => {
            anyhow::ensure!(!paths.is_empty(), "--path is required (at least one)");
            eprintln!(
                "note: `propose-agents` is a deprecated alias; use `swamp propose --path <unit>` (no root needed) instead."
            );
            propose_unified(None, None, paths, None, json, false)?;
        }
        Command::Protect { cmd } => cmd_protect(cmd)?,
        Command::Approve { plan_id } => cmd_approve(&plan_id)?,
        Command::Config { action } => {
            let dir = swamp_dir();
            let path = dir.join("config.toml");
            match action {
                ConfigAction::Path => println!("{}", path.display()),
                ConfigAction::Show => {
                    print!(
                        "{}",
                        swamp_core::growth::load_config_checked(&dir)?.to_toml()
                    );
                    if !swamp_core::fs_gate::exists(&path) {
                        eprintln!(
                            "(defaults; no file at {} — `swamp config init` writes one)",
                            path.display()
                        );
                    }
                }
                ConfigAction::Init => {
                    if swamp_core::fs_gate::exists(&path) {
                        eprintln!("{} already exists; not overwriting", path.display());
                        std::process::exit(1);
                    }
                    swamp_core::fs_gate::store::write_text(
                        swamp_core::fs_gate::store::TextFile::Config {
                            store: &swamp_core::fs_gate::store::StoreDir::resolved(),
                        },
                        &swamp_core::growth::GrowthConfig::default().to_toml(),
                    )?;
                    println!("wrote {}", path.display());
                }
            }
        }
        Command::Scope { roots, json } => {
            let scope = resolve_scope(&roots)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&scope)?);
            } else {
                print!("{}", render_scope_text(&scope));
                if scope.is_empty_scope() {
                    eprintln!(
                        "effective scan scope is empty: no built-in default, detector, or configured include is enabled -- this is explicit, never a silent fallback to cwd or home."
                    );
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
                let total = plans.len();
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"plans": plans, "total": total})
                    )?
                );
            } else if plans.is_empty() {
                println!("no plans");
            } else {
                for p in plans {
                    println!(
                        "{}  {:?}  {} units  {}  created {}  expires {}",
                        p.id,
                        p.status(),
                        p.units().len(),
                        swamp_core::render::human_bytes_pub(p.planned_bytes()),
                        p.created_at(),
                        p.expires_at()
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
                } => cmd_grant_add(&dir, &predicate, &budget, &expires, max_units)?,
                GrantCmd::List { json } => {
                    let gs = swamp_core::actions::list_grants(&dir)?;
                    if json {
                        let total = gs.len();
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "grants": gs,
                                "total": total,
                                "note": "grants are minted only by a human running `swamp approve <plan_id>` or `swamp grant add ...`; no command reads standing authorization into existence on its own",
                            }))?
                        );
                    } else if gs.is_empty() {
                        println!("no grants");
                    } else {
                        for g in gs {
                            println!(
                                "{}  {}  {}  budget {} spent {}  units {}/{}  expires {}  by {}",
                                g.id(),
                                if g.revoked() { "revoked" } else { "live" },
                                g.plan_id()
                                    .as_ref()
                                    .map(|p| format!("plan {p}"))
                                    .unwrap_or_else(|| format!("where {}", g.predicate())),
                                swamp_core::render::human_bytes_pub(g.budget_bytes().unwrap_or(0)),
                                swamp_core::render::human_bytes_pub(g.spent_bytes()),
                                g.used_units(),
                                g.max_units()
                                    .map(|m| m.to_string())
                                    .unwrap_or_else(|| "∞".into()),
                                g.expires_at(),
                                g.actor()
                            );
                        }
                    }
                }
                GrantCmd::Revoke { grant_id } => cmd_grant_revoke(&dir, &grant_id)?,
            }
        }
        Command::Observe { roots, full } => {
            let store_dir = swamp_dir();
            let scope = resolve_scope(&roots)?;
            let resolved = scope.scan_paths();
            if resolved.is_empty() {
                if scope.is_empty_scope() {
                    anyhow::bail!(
                        "effective scan scope is empty: no built-in default, detector, or configured include is enabled. This is explicit, not a fallback to the current directory -- see `swamp scope --json`, or pass a root explicitly."
                    );
                }
                anyhow::bail!(
                    "no present root to observe (every candidate is missing/unreadable/excluded) -- see `swamp scope --json`, or pass a root explicitly."
                );
            }
            note_and_persist_scope(&store_dir, &scope);
            schedule::cmd_observe(store_dir, resolved, full)?;
        }
        Command::Schedule { every, off, roots } => {
            let store_dir = swamp_dir();
            // No explicit roots: install `observe` with none baked into
            // the plist's argv at all (#42/#50), so every scheduled fire
            // re-resolves the configured scope itself (same code path
            // `swamp observe` with no roots already takes) instead of
            // replaying whatever was present at `schedule --every` time.
            // A config edit therefore takes effect on the next scheduled
            // run, not only after `schedule --every` is run again. This
            // is a validate-then-install check only: it fails fast on an
            // empty scope now rather than installing a schedule that can
            // never do anything, but it does not freeze the resolved
            // list into the plist -- explicit roots on the command line
            // still do, exactly as an explicit root has always replaced
            // the configured scope for one invocation.
            if roots.is_empty() && !off && every.is_some() {
                let scope = resolve_scope(&[])?;
                if scope.scan_paths().is_empty() {
                    anyhow::bail!(
                        "effective scan scope is empty; nothing to schedule -- see `swamp scope --json`, or pass roots explicitly."
                    );
                }
            }
            schedule::cmd_schedule(store_dir, every, off, roots)?;
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

#[cfg(test)]
mod cleanup_check_tests {
    use super::{
        ArtifactRole, CLEANUP_COVERAGE_DETAIL_LIMITS, CLEANUP_COVERAGE_DETAIL_ROWS, Path, PathBuf,
        cleanup_scope_summary,
    };
    use swamp_core::artifact::{ArtifactCoverage, Membership, NestedArtifact};

    fn synthetic(
        path: impl Into<PathBuf>,
        role: ArtifactRole,
        supported: bool,
        complete: bool,
        limits: Vec<String>,
    ) -> NestedArtifact {
        let path = path.into();
        NestedArtifact {
            id: path.display().to_string(),
            relative_path: path.display().to_string(),
            path,
            parent_id: None,
            container_id: None,
            role,
            membership: Membership::Unknown,
            is_dir: false,
            device: 0,
            inode: 0,
            logical_bytes: 1,
            bytes: 1,
            physical_bytes: 1,
            physical_total: 1,
            mtime_max: 0,
            variant: Default::default(),
            producer_evidence: Vec::new(),
            consumer_evidence: Vec::new(),
            coverage: ArtifactCoverage {
                supported,
                complete,
                limits,
            },
            action_group: None,
            present: true,
            growth_bytes: None,
            regrowth_count: 0,
            decision_evidence: Vec::new(),
            adapter: None,
            basis: Default::default(),
            time_source: Default::default(),
            action: Default::default(),
            consequence: None,
            reported_by: None,
            writer_lock: None,
        }
    }

    #[test]
    fn cleanup_scope_summary_keeps_unknown_coverage_visible_and_bounded() {
        let mut rows = vec![
            synthetic(
                "/root/target",
                ArtifactRole::Container,
                false,
                false,
                vec!["root coverage".into()],
            ),
            synthetic(
                "/root/target/debug/deps/unknown",
                ArtifactRole::Unknown,
                false,
                false,
                vec!["unknown coverage".into()],
            ),
            synthetic(
                "/root/target/debug/deps/residual",
                ArtifactRole::Residual,
                true,
                true,
                Vec::new(),
            ),
            synthetic(
                "/root/target/debug/incremental/group",
                ArtifactRole::Incremental,
                true,
                true,
                Vec::new(),
            ),
            synthetic(
                "/root/target/debug/incremental/group/child",
                ArtifactRole::Residual,
                true,
                true,
                Vec::new(),
            ),
        ];
        for index in 0..25 {
            rows.push(synthetic(
                format!("/root/target/debug/deps/unknown-{index}"),
                ArtifactRole::Unknown,
                false,
                false,
                (0..10).map(|n| format!("limit-{n}")).collect(),
            ));
        }

        let summary = cleanup_scope_summary(&rows, Some(Path::new("/root/target/debug")));
        assert_eq!(summary.nested_row_count, rows.len() - 1);
        assert_eq!(summary.coverage_limited_count, 27);
        assert_eq!(summary.unknown_or_residual_count, 28);
        assert_eq!(
            summary.coverage_limited_details.len(),
            CLEANUP_COVERAGE_DETAIL_ROWS
        );
        assert!(
            summary
                .coverage_limited_details
                .iter()
                .all(|detail| detail.limits.len() <= CLEANUP_COVERAGE_DETAIL_LIMITS)
        );

        let group_scope = cleanup_scope_summary(
            &rows,
            Some(Path::new("/root/target/debug/incremental/group")),
        );
        assert_eq!(group_scope.nested_row_count, 2);
        assert_eq!(group_scope.coverage_limited_count, 1);
        assert_eq!(
            group_scope.coverage_limited_details[0].path,
            PathBuf::from("/root/target")
        );
        assert_eq!(group_scope.unknown_or_residual_count, 1);
    }
}
