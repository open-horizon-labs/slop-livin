//! The report pipeline as consumers on an in-memory event bus.
//!
//! Every stage of building a [`Report`] is a [`Consumer`] that declares
//! which [`EventKind`]s wake it and returns follow-on events. The bus holds
//! the registry and routes; **it is the only coupling between stages**.
//! No consumer knows another consumer exists. See
//! `docs/ADRs/001-event-bus-report-pipeline.md`.
//!
//! **Static registration, dynamic routing.** [`EventBus::with_builtins`]
//! registers every consumer before the first event fires; `run` seals the
//! registry. There is no runtime registration and no conditional wiring:
//! a consumer that has nothing to do for a run returns no events.
//!
//! **Runtime.** Consumers are `async fn on_event`, dispatched on a tokio
//! current-thread runtime ([`EventBus::run_blocking`]). All subscribers of
//! one event run concurrently; the follow-on events they return are
//! dispatched depth-first, in the order the consumers were registered.
//!
//! **Facts flow as events.** A consumer never sees `&mut Report`. It gets
//! the facts it subscribed to and emits new ones; gates wait for a set of
//! facts and emit the assembled rows; the assembler folds the final facts
//! into the report. Payloads that several consumers read are `Arc`s.
//!
//! Adding a stage: implement [`Consumer`] in a new file under
//! `consumers/`, register it in [`EventBus::with_builtins`]. The source
//! audits (`cargo run -p swamp-source-audit`) fail the build if a
//! consumer names another, registers at runtime, or if `report.rs` calls a
//! stage directly.

use crate::attribution::AttributionResult;
use crate::git::DiscoveredWorktree;
use crate::github::GithubFacts;
use crate::report::Signal;
use crate::report::{
    ArtifactRow, DirRollup, FileRow, GithubEnrichmentSummary, ProjectRow, Reconciliation, Report,
    UnownedRow,
};
use crate::signals::RawSignals;
use anyhow::{Result, bail};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Everything a run was asked for. Consumers read it; nobody writes it.
pub struct Ctx<'a> {
    pub root: PathBuf,
    pub docker_facts: Option<PathBuf>,
    pub verify_du: bool,
    pub store_dir: Option<PathBuf>,
    pub since_override: Option<String>,
    /// Persist a new observation into the growth store.
    pub observe: bool,
    /// Group per-directory rollups under each worktree into the report.
    pub include_dirs: bool,
    /// Refresh GitHub enrichment live (shells out to `gh`).
    pub enrich: bool,
    /// Skip the FSEvents attempt and walk everything.
    pub force_full: bool,
    pub fs_events: &'a dyn crate::fs_events::FsEventsSource,
    pub observed_at: u64,
    pub large_file_min_bytes: u64,
}

/// Per-worktree git activity, as one consumer computes it and others read it.
#[derive(Debug, Clone)]
pub struct WorktreeSignals {
    pub path: PathBuf,
    pub rows: Vec<Signal>,
    pub raw: RawSignals,
    pub branch: Option<String>,
}

/// The report under construction: what the assembly gate hands to the
/// growth store, which hands it on to tracking, which hands it to the
/// assembler. Each stage returns a new `Draft`; nothing is mutated in place
/// across consumers.
#[derive(Debug, Clone)]
pub struct Draft {
    pub projects: Vec<ProjectRow>,
    pub unowned: Vec<UnownedRow>,
    pub dirs: Vec<DirRollup>,
    pub files: Vec<FileRow>,
    pub dirs_by_worktree: Option<HashMap<String, Vec<DirRollup>>>,
    pub files_by_worktree: Option<HashMap<String, Vec<FileRow>>>,
    pub notes: Vec<String>,
    pub reconciliation: Reconciliation,
    pub github_enrichment: Option<GithubEnrichmentSummary>,
    pub schedule_line: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    RootRequested,
    RootObserved,
    ProjectsGrouped,
    SignalsComputed,
    EcosystemsDetected,
    GithubEnriched,
    DockerJoined,
    RowsAssembled,
    GrowthAnnotated,
    TrackingAnnotated,
    HistoryLoaded,
    ReportAssembled,
    /// Test-only traffic for the bus's own tests; no builtin subscribes.
    Probe,
}

/// Every fact the pipeline emits. Each variant carries what its readers
/// need; no consumer polls context for another's output.
#[derive(Debug, Clone)]
pub enum Event {
    RootRequested,
    RootObserved {
        discovered: Arc<Vec<DiscoveredWorktree>>,
        attribution: Arc<AttributionResult>,
        notes: Vec<String>,
        /// Worktree ids the walk actually visited; `None` = all of them.
        rewalked: Option<Arc<Vec<String>>>,
    },
    ProjectsGrouped {
        projects: Arc<Vec<ProjectRow>>,
        rewalked: Option<Arc<Vec<String>>>,
        worktree_paths: Arc<Vec<(PathBuf, String)>>,
        /// project_id -> normalized remote URL.
        project_remotes: Arc<HashMap<String, String>>,
        /// worktree_id -> that worktree's raw remote URL.
        worktree_remotes: Arc<HashMap<String, String>>,
        unowned: Arc<Vec<UnownedRow>>,
        dirs: Arc<Vec<DirRollup>>,
        files: Arc<Vec<FileRow>>,
        reconciliation: Reconciliation,
        notes: Vec<String>,
    },
    SignalsComputed {
        by_worktree: Arc<HashMap<String, WorktreeSignals>>,
    },
    EcosystemsDetected {
        /// project_id -> ecosystem tags.
        by_project: Arc<HashMap<String, Vec<String>>>,
    },
    GithubEnriched {
        facts_by_worktree: Arc<HashMap<String, GithubFacts>>,
        summary: Option<GithubEnrichmentSummary>,
        notes: Vec<String>,
    },
    DockerJoined {
        rows_by_worktree: Arc<HashMap<String, Vec<ArtifactRow>>>,
        unowned: Arc<Vec<UnownedRow>>,
        attributed_bytes: u64,
        unowned_bytes: u64,
        notes: Vec<String>,
    },
    RowsAssembled(Arc<Draft>),
    GrowthAnnotated(Arc<Draft>),
    TrackingAnnotated(Arc<Draft>),
    HistoryLoaded {
        series_by_key: Arc<HashMap<String, Vec<Option<u64>>>>,
        total_series: Vec<Option<u64>>,
        window_secs: u64,
    },
    ReportAssembled(Arc<Report>),
    Probe {
        tag: String,
        depth: u8,
    },
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Event::RootRequested => EventKind::RootRequested,
            Event::RootObserved { .. } => EventKind::RootObserved,
            Event::ProjectsGrouped { .. } => EventKind::ProjectsGrouped,
            Event::SignalsComputed { .. } => EventKind::SignalsComputed,
            Event::EcosystemsDetected { .. } => EventKind::EcosystemsDetected,
            Event::GithubEnriched { .. } => EventKind::GithubEnriched,
            Event::DockerJoined { .. } => EventKind::DockerJoined,
            Event::RowsAssembled(_) => EventKind::RowsAssembled,
            Event::GrowthAnnotated(_) => EventKind::GrowthAnnotated,
            Event::TrackingAnnotated(_) => EventKind::TrackingAnnotated,
            Event::HistoryLoaded { .. } => EventKind::HistoryLoaded,
            Event::ReportAssembled(_) => EventKind::ReportAssembled,
            Event::Probe { .. } => EventKind::Probe,
        }
    }
}

/// One stage. **Must not** register consumers, name other consumers, or
/// reach for another consumer's output except through the events it
/// subscribes to.
#[async_trait::async_trait(?Send)]
pub trait Consumer {
    /// Identifier for diagnostics.
    fn name(&self) -> &str;
    /// Which event kinds wake this consumer. `on_event` is called only for
    /// events whose kind appears here.
    fn subscribes_to(&self) -> &[EventKind];
    /// React to an event; return follow-on events to emit.
    async fn on_event(&self, event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>>;
}

pub struct EventBus {
    consumers: Vec<Box<dyn Consumer>>,
    sealed: bool,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        EventBus {
            consumers: Vec::new(),
            sealed: false,
        }
    }

    /// Every builtin stage, in registration order. Static: this is the
    /// one place the pipeline's membership is written down.
    pub fn with_builtins() -> Self {
        use crate::consumers::*;
        let mut bus = EventBus::new();
        for c in [
            Box::new(WalkConsumer) as Box<dyn Consumer>,
            Box::new(ProjectsConsumer),
            Box::new(SignalsConsumer),
            Box::new(EcosystemConsumer),
            Box::new(GithubConsumer::default()),
            Box::new(DockerConsumer),
            Box::new(AssemblyGate::default()),
            Box::new(GrowthConsumer),
            Box::new(TrackingConsumer),
            Box::new(HistoryConsumer),
            Box::new(ReportAssembler::default()),
            Box::new(CacheWriter),
        ] {
            bus.register(c).expect("builtins register before any run");
        }
        bus
    }

    /// Registers a consumer. Refused once `run` has started: the registry
    /// is fixed before the first event fires.
    pub fn register(&mut self, consumer: Box<dyn Consumer>) -> Result<()> {
        if self.sealed {
            bail!(
                "event bus is sealed: `{}` cannot register after run started",
                consumer.name()
            );
        }
        self.consumers.push(consumer);
        Ok(())
    }

    pub fn consumer_names(&self) -> Vec<&str> {
        self.consumers.iter().map(|c| c.name()).collect()
    }

    /// Dispatches `seed` and every follow-on until the queue is empty.
    /// Subscribers of one event run concurrently; their follow-ons are
    /// queued depth-first (a follow-on is dispatched before anything that
    /// was already waiting). Returns every event that was dispatched, in
    /// dispatch order.
    pub async fn run(&mut self, seed: Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        self.sealed = true;
        let mut queue: VecDeque<Event> = VecDeque::from([seed]);
        let mut dispatched: Vec<Event> = Vec::new();
        while let Some(event) = queue.pop_front() {
            let kind = event.kind();
            let subscribers: Vec<&dyn Consumer> = self
                .consumers
                .iter()
                .map(|c| c.as_ref())
                .filter(|c| c.subscribes_to().contains(&kind))
                .collect();
            let trace = std::env::var("SWAMP_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
            let results = futures_util::future::join_all(subscribers.iter().map(|c| async {
                let t = std::time::Instant::now();
                let r = c.on_event(&event, ctx).await;
                if trace {
                    eprintln!("[trace] {:?} → {}: {:?}", kind, c.name(), t.elapsed());
                }
                r
            }))
            .await;
            let mut follow_on: Vec<Event> = Vec::new();
            for (c, r) in subscribers.iter().zip(results) {
                let events =
                    r.map_err(|e| anyhow::anyhow!("consumer `{}` on {:?}: {e}", c.name(), kind))?;
                follow_on.extend(events);
            }
            for e in follow_on.into_iter().rev() {
                queue.push_front(e);
            }
            dispatched.push(event);
        }
        Ok(dispatched)
    }

    /// `run` on a fresh tokio current-thread runtime, for the synchronous
    /// callers (CLI, MCP, TUI worker thread). Must not be called from
    /// inside another tokio runtime.
    pub fn run_blocking(&mut self, seed: Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(self.run(seed, ctx))
    }
}

/// Builds a [`Report`] for `ctx` by running the builtin consumers from
/// `RootRequested`. The one entry point the report module calls.
pub fn run_report(ctx: &Ctx<'_>) -> Result<Report> {
    let mut bus = EventBus::with_builtins();
    let events = bus.run_blocking(Event::RootRequested, ctx)?;
    for e in events.into_iter().rev() {
        if let Event::ReportAssembled(report) = e {
            return Ok(Arc::try_unwrap(report).unwrap_or_else(|arc| (*arc).clone()));
        }
    }
    bail!("the bus finished without assembling a report")
}

/// The context a caller builds from the report-function arguments.
#[allow(clippy::too_many_arguments)]
pub fn ctx_for<'a>(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
    observe: bool,
    include_dirs: bool,
    enrich: bool,
    force_full: bool,
    fs_events: &'a dyn crate::fs_events::FsEventsSource,
) -> Ctx<'a> {
    let large_file_min_bytes = store_dir
        .map(|dir| crate::growth::load_config(dir).large_file_min_bytes)
        .unwrap_or(crate::growth::DEFAULT_LARGE_FILE_MIN_BYTES);
    Ctx {
        root: root.to_path_buf(),
        docker_facts: docker_facts.map(Path::to_path_buf),
        verify_du,
        store_dir: store_dir.map(Path::to_path_buf),
        since_override: since_override.map(str::to_string),
        observe,
        include_dirs,
        enrich,
        force_full,
        fs_events,
        observed_at: crate::entities::now(),
        large_file_min_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn ctx<'a>(src: &'a dyn crate::fs_events::FsEventsSource) -> Ctx<'a> {
        ctx_for(
            Path::new("/nonexistent"),
            None,
            false,
            None,
            None,
            false,
            false,
            false,
            true,
            src,
        )
    }

    /// Records every probe it sees; re-emits one deeper probe per event
    /// until `max_depth`.
    struct Prober {
        name: String,
        seen: Mutex<Vec<String>>,
        max_depth: u8,
    }
    #[async_trait::async_trait(?Send)]
    impl Consumer for Prober {
        fn name(&self) -> &str {
            &self.name
        }
        fn subscribes_to(&self) -> &[EventKind] {
            &[EventKind::Probe]
        }
        async fn on_event(&self, event: &Event, _ctx: &Ctx<'_>) -> Result<Vec<Event>> {
            let Event::Probe { tag, depth } = event else {
                return Ok(vec![]);
            };
            self.seen.lock().unwrap().push(format!("{tag}@{depth}"));
            if *depth < self.max_depth {
                return Ok(vec![Event::Probe {
                    tag: format!("{}>{}", tag, self.name),
                    depth: depth + 1,
                }]);
            }
            Ok(vec![])
        }
    }

    fn prober(name: &str, max_depth: u8) -> Box<Prober> {
        Box::new(Prober {
            name: name.into(),
            seen: Mutex::new(vec![]),
            max_depth,
        })
    }

    #[test]
    fn follow_on_events_are_routed_to_subscribers() {
        let src = crate::fs_events::UnsupportedPlatformSource;
        let c = ctx(&src);
        let mut bus = EventBus::new();
        bus.register(prober("a", 1)).unwrap();
        let events = bus
            .run_blocking(
                Event::Probe {
                    tag: "seed".into(),
                    depth: 0,
                },
                &c,
            )
            .unwrap();
        let tags: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::Probe { tag, .. } => Some(tag.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tags,
            vec!["seed", "seed>a"],
            "the follow-on was dispatched too"
        );
    }

    #[test]
    fn follow_on_events_dispatch_depth_first() {
        // Two seeds queued; a's follow-on from seed1 must run before seed2.
        let src = crate::fs_events::UnsupportedPlatformSource;
        let c = ctx(&src);
        let mut bus = EventBus::new();
        bus.register(prober("a", 1)).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let events = rt
            .block_on(async {
                // Seed with a probe whose follow-on itself has a follow-on:
                // order must be seed, seed>a, (seed>a)>a — never breadth-first.
                bus.register(prober("b", 2)).unwrap();
                bus.run(
                    Event::Probe {
                        tag: "s".into(),
                        depth: 0,
                    },
                    &c,
                )
                .await
            })
            .unwrap();
        let tags: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::Probe { tag, .. } => Some(tag.clone()),
                _ => None,
            })
            .collect();
        // a and b both answer "s" (depth 0): follow-ons s>a and s>b are
        // queued in registration order. Depth-first means s>a's own
        // descendants (only b answers at depth 1: s>a>b) come before s>b.
        assert_eq!(tags, vec!["s", "s>a", "s>a>b", "s>b", "s>b>b"]);
    }

    #[test]
    fn consumer_receives_events_regardless_of_registration_order() {
        let src = crate::fs_events::UnsupportedPlatformSource;
        let c = ctx(&src);
        for order in [["a", "b"], ["b", "a"]] {
            let mut bus = EventBus::new();
            for n in order {
                bus.register(prober(n, 0)).unwrap();
            }
            let events = bus
                .run_blocking(
                    Event::Probe {
                        tag: "x".into(),
                        depth: 0,
                    },
                    &c,
                )
                .unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(bus.consumer_names().len(), 2);
        }
    }

    #[test]
    fn registration_is_closed_once_run_starts() {
        let src = crate::fs_events::UnsupportedPlatformSource;
        let c = ctx(&src);
        let mut bus = EventBus::new();
        bus.register(prober("a", 0)).unwrap();
        bus.run_blocking(
            Event::Probe {
                tag: "x".into(),
                depth: 0,
            },
            &c,
        )
        .unwrap();
        let err = bus.register(prober("late", 0)).unwrap_err();
        assert!(err.to_string().contains("sealed"), "{err}");
    }

    #[test]
    fn builtins_cover_the_whole_pipeline_once_each() {
        let bus = EventBus::with_builtins();
        let names = bus.consumer_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "a consumer registered twice");
        for needed in [
            "walk", "projects", "signals", "github", "docker", "gate", "growth", "tracking",
            "history", "assemble", "cache",
        ] {
            assert!(
                names.iter().any(|n| n.contains(needed)),
                "missing {needed}: {names:?}"
            );
        }
    }
}
