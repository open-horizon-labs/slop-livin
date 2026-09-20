//! Application state and key handling. Pure aside from the action layer
//! calls the confirm step makes; the rest is unit-testable without a
//! terminal.

use crate::actions::{self, MarkedUnit};
use crate::filter::{self, Filter};
use crate::model::{self, Row, Sort};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use swamp_core::report::Report;

pub const REFUSAL_DISPLAY: Duration = Duration::from_secs(4);

/// Same view set the CLI's `--view` exposes at root (`worktrees` there
/// is `Projects` here: one row per project, same aggregation), plus
/// `Tree`, the per-project drill-down `--project` renders (#33). `v`
/// cycles this exact order on both surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Projects,
    Tree,
    Builds,
    Deps,
    Docker,
    Kinds,
    Unowned,
    /// Per-ecosystem rollup (`Report.summary`).
    Types,
}

impl ViewKind {
    pub fn from_digit(d: char) -> Option<Self> {
        Some(match d {
            '1' => ViewKind::Projects,
            '2' => ViewKind::Tree,
            '3' => ViewKind::Builds,
            '4' => ViewKind::Deps,
            '5' => ViewKind::Docker,
            '6' => ViewKind::Kinds,
            '7' => ViewKind::Unowned,
            '8' => ViewKind::Types,
            _ => return None,
        })
    }
    pub fn next(self) -> Self {
        match self {
            ViewKind::Projects => ViewKind::Tree,
            ViewKind::Tree => ViewKind::Builds,
            ViewKind::Builds => ViewKind::Deps,
            ViewKind::Deps => ViewKind::Docker,
            ViewKind::Docker => ViewKind::Kinds,
            ViewKind::Kinds => ViewKind::Unowned,
            ViewKind::Unowned => ViewKind::Types,
            ViewKind::Types => ViewKind::Projects,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Projects => "projects",
            ViewKind::Tree => "tree",
            ViewKind::Builds => "builds",
            ViewKind::Deps => "deps",
            ViewKind::Kinds => "kinds",
            ViewKind::Docker => "docker",
            ViewKind::Unowned => "unowned",
            ViewKind::Types => "types",
        }
    }
}

pub struct App {
    pub report: Report,
    pub root: PathBuf,
    pub view: ViewKind,
    pub filter_text: String,
    pub filter: Filter,
    pub filter_error: Option<String>,
    pub editing_filter: bool,
    /// Filter text as it was when editing began; restored on Esc.
    pub filter_before_edit: String,
    /// Background observation result, when one is in flight.
    pub pending: Option<std::sync::mpsc::Receiver<anyhow::Result<Report>>>,
    /// One-line status shown in the footer slot (errors, notices).
    pub status: Option<String>,
    pub selected: usize,
    pub selected_project: Option<String>,
    pub collapsed: HashSet<String>,
    /// Marked units, keyed by path string for stable identity.
    pub marked: BTreeMap<String, MarkedUnit>,
    pub confirm_open: bool,
    pub help_open: bool,
    /// The filter picker form, when open.
    pub picker: Option<crate::picker::Picker>,
    /// Tab-completion candidates shown under the raw filter line.
    pub completions: Vec<String>,
    pub refusal: Option<(String, Instant)>,
    pub observing: Option<(u32, u32)>,
    pub last_result: Option<String>,
    pub observed_label: String,
    pub actor: String,
    pub sort: Sort,
    /// Flip the active sort's order (`r`). Persisted with the sort.
    pub reverse: bool,
    /// Copy compiled outputs to `<worktree>/bin/` before trashing a build
    /// directory (`k` on the confirm line). Persisted.
    pub keep_executables: bool,
    pub quit: bool,
    /// Terminal width at the last draw; the header fits its clauses to it.
    pub width: u16,
    /// Seconds of observation history the store holds; bounds the growth
    /// windows a human may pick (the store cannot answer beyond it).
    pub history_secs: Option<u64>,
    /// git tracking status per absolute path, filled when a project is
    /// opened (one exclude stack per worktree, reused for its rows).
    pub track: std::collections::HashMap<PathBuf, swamp_core::ignore::TrackState>,
    /// Store dir, when known: the applied filter is persisted there so it
    /// survives relaunch (`ui_filter.txt`).
    pub store_dir: Option<PathBuf>,
    /// The live FSEvents stream on the root, running for the TUI's
    /// lifetime. Every change under the root, including our own deletes,
    /// arrives here; nothing "asks" for a refresh.
    pub watch: Option<swamp_core::fs_events::Watcher>,
    pub watch_rx: Option<std::sync::mpsc::Receiver<swamp_core::fs_events::WatchBatch>>,
    /// Changed directories received and not yet observed.
    pub live_changes: std::collections::HashSet<PathBuf>,
    pub live_last_event_id: u64,
    /// When the last batch arrived; observation starts once the stream has
    /// been quiet for `LIVE_QUIET`.
    pub live_last_batch: Option<Instant>,
}

fn ui_state_path(store: &std::path::Path) -> PathBuf {
    store.join("ui_state.json")
}

/// What the TUI remembers between sessions: the applied filter and the
/// sort. Both are choices a human made about how to look at their own
/// machine; asking again every launch is the tool forgetting on purpose.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub keep_executables: bool,
}

pub fn load_ui_state(store: &std::path::Path) -> UiState {
    std::fs::read_to_string(ui_state_path(store))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn sort_from_str(s: &str) -> Sort {
    match s {
        "growth" => Sort::Growth,
        "size" => Sort::Size,
        "name" => Sort::Name,
        "type" => Sort::Type,
        "age" => Sort::Age,
        _ => Sort::None,
    }
}

pub fn sort_to_str(s: Sort) -> &'static str {
    match s {
        Sort::Growth => "growth",
        Sort::Size => "size",
        Sort::Name => "name",
        Sort::Type => "type",
        Sort::Age => "age",
        Sort::None => "none",
    }
}

impl App {
    pub fn new(report: Report, root: PathBuf) -> Self {
        let filter = filter::default_filter();
        App {
            report,
            root,
            view: ViewKind::Projects,
            filter_text: filter::default_filter_text().to_string(),
            filter,
            filter_error: None,
            editing_filter: false,
            filter_before_edit: String::new(),
            pending: None,
            status: None,
            selected: 0,
            selected_project: None,
            collapsed: HashSet::new(),
            marked: BTreeMap::new(),
            confirm_open: false,
            help_open: false,
            picker: None,
            completions: Vec::new(),
            refusal: None,
            observing: None,
            last_result: None,
            observed_label: "just now".to_string(),
            actor: "human".to_string(),
            sort: Sort::None,
            reverse: false,
            keep_executables: false,
            quit: false,
            width: 0,
            store_dir: None,
            watch: None,
            watch_rx: None,
            live_changes: std::collections::HashSet::new(),
            live_last_event_id: 0,
            live_last_batch: None,
            track: std::collections::HashMap::new(),
            history_secs: None,
        }
    }

    /// Annotates every row of one project with its git tracking status:
    /// one exclude stack per worktree, one lookup per displayed path.
    pub fn annotate_project(&mut self, project_name: &str) {
        let Some(p) = self.report.projects.iter().find(|p| p.name == project_name) else {
            return;
        };
        let mut found = std::collections::HashMap::new();
        for wt in &p.worktrees {
            let Some(lens) = swamp_core::ignore::IgnoreLens::open(&wt.path) else {
                continue;
            };
            for a in &wt.artifacts {
                let rel = a
                    .path
                    .strip_prefix(&wt.path)
                    .map(|r| r.display().to_string())
                    .unwrap_or_default();
                found.insert(a.path.clone(), lens.status(&rel, true));
            }
            if let Some(dirs) = self
                .report
                .dirs_by_worktree
                .as_ref()
                .and_then(|m| m.get(&wt.worktree_id))
            {
                for d in dirs.iter().filter(|d| !d.rel_path.contains('/')) {
                    found.insert(wt.path.join(&d.rel_path), lens.status(&d.rel_path, true));
                }
            }
        }
        self.track.extend(found);
    }

    /// Swap in a fresh report (background observation finished). Rows are
    /// derived from `report` on demand, so nothing else needs rebuilding;
    /// the selection is clamped by `rows()` consumers.
    pub fn replace_report(&mut self, report: Report) {
        // A background observation must not move the cursor: remember
        // which row it is on and put it back on the same row, wherever
        // the new report sorts it.
        let anchor = self.selected_row_key();
        self.report = report;
        self.restore_selection(anchor);
    }

    /// Identity of the selected row: its unit path when it has one
    /// (stable across re-sorts), otherwise its label.
    fn selected_row_key(&self) -> Option<String> {
        self.rows()
            .into_iter()
            .nth(self.selected)
            .map(|r| r.unit.map(|u| u.0).unwrap_or(r.label))
    }

    fn restore_selection(&mut self, key: Option<String>) {
        let rows = self.rows();
        let found = key.and_then(|k| {
            rows.iter().position(|r| {
                r.unit
                    .as_ref()
                    .map(|u| u.0.clone())
                    .unwrap_or_else(|| r.label.clone())
                    == k
            })
        });
        self.selected = found.unwrap_or_else(|| self.selected.min(rows.len().saturating_sub(1)));
    }

    /// Rows for the current view, honoring the active filter and, for
    /// the tree view, the currently selected project (defaulting to the
    /// first one).
    pub fn rows(&self) -> Vec<Row> {
        // The tree view is a hierarchy (worktree -> artifact), not a flat
        // ranked list, so sort never reorders it -- reordering would break
        // the rail's parent/child adjacency.
        let mut rows = match self.view {
            ViewKind::Projects => model::projects_rows(&self.report, &self.filter),
            ViewKind::Tree => {
                let name = self
                    .selected_project
                    .clone()
                    .or_else(|| self.report.projects.first().map(|p| p.name.clone()));
                return match name {
                    Some(n) => model::tree_rows(
                        &self.report,
                        &n,
                        &self.filter,
                        &self.collapsed,
                        &self.track,
                    ),
                    None => Vec::new(),
                };
            }
            ViewKind::Builds => model::builds_rows(&self.report, &self.filter),
            ViewKind::Deps => model::deps_rows(&self.report, &self.filter),
            ViewKind::Kinds => model::kinds_rows(&self.report, &self.filter),
            ViewKind::Docker => model::docker_rows(&self.report),
            ViewKind::Unowned => model::unowned_rows(&self.report),
            ViewKind::Types => model::types_rows(&self.report, &self.filter),
        };
        model::apply_sort(&mut rows, self.sort, self.reverse);
        rows
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = if self.sort == sort { Sort::None } else { sort };
        self.persist_ui_state();
    }

    /// `r`: flip the order of whatever sort is active.
    pub fn toggle_reverse(&mut self) {
        self.reverse = !self.reverse;
        self.persist_ui_state();
    }

    /// `k`: whether a delete first copies compiled outputs to `bin/`.
    pub fn toggle_keep_executables(&mut self) {
        self.keep_executables = !self.keep_executables;
        self.persist_ui_state();
    }

    /// Whether the active filter is one that has no data source yet.
    /// Kept for the UI: every predicate now has a data source, so a
    /// filter never lands in a "no data yet" state.
    pub fn filter_has_no_data(&self) -> bool {
        false
    }

    pub fn set_view(&mut self, v: ViewKind) {
        self.view = v;
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: i32) {
        let len = self.rows().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected as i32 + delta;
        self.selected = cur.clamp(0, len as i32 - 1) as usize;
    }

    /// Enter filter editing with the current text kept, so the human edits
    /// what is there (Backspace to trim, type to extend) rather than
    /// starting from an empty line behind a `/`.
    pub fn start_filter_edit(&mut self) {
        self.filter_before_edit = self.filter_text.clone();
        if self.filter_text == "0" {
            self.filter_text.clear();
        }
        self.editing_filter = true;
        self.filter_error = None;
    }

    pub fn open_picker(&mut self) {
        self.picker = Some(crate::picker::Picker::from_report(
            &self.report,
            &self.filter_text,
            self.history_secs,
        ));
    }

    pub fn apply_picker(&mut self) {
        if let Some(p) = self.picker.take() {
            self.filter_text = p.compose();
            self.editing_filter = false;
            self.commit_filter();
        }
    }

    /// `e` in the picker: carry its composed text into the raw line.
    pub fn picker_to_raw_edit(&mut self) {
        if let Some(p) = self.picker.take() {
            self.filter_before_edit = self.filter_text.clone();
            self.filter_text = p.compose();
            if self.filter_text == "0" {
                self.filter_text.clear();
            }
            self.editing_filter = true;
            self.filter_error = None;
        }
    }

    pub fn filter_tab_complete(&mut self) {
        let projects: Vec<String> = self
            .report
            .projects
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let (text, cands) = crate::picker::apply_completion(&self.filter_text, &projects);
        self.filter_text = text;
        self.completions = if cands.len() > 1 { cands } else { Vec::new() };
    }

    pub fn cancel_filter_edit(&mut self) {
        self.completions.clear();
        self.filter_text = std::mem::take(&mut self.filter_before_edit);
        self.editing_filter = false;
    }

    pub fn filter_input(&mut self, c: char) {
        self.filter_text.push(c);
        self.completions.clear();
    }

    /// Rows the current filter would show — for the picker's live count.
    pub fn match_count_for(&self, text: &str) -> Option<usize> {
        let f = crate::filter::parse(text).ok()?;
        let mut probe = App::new(self.report.clone(), self.root.clone());
        probe.view = self.view;
        probe.selected_project = self.selected_project.clone();
        probe.filter = f;
        Some(probe.rows().len())
    }

    pub fn filter_backspace(&mut self) {
        self.filter_text.pop();
    }

    pub fn commit_filter(&mut self) {
        self.editing_filter = false;
        self.completions.clear();
        match filter::parse(&self.filter_text) {
            Ok(f) => {
                self.filter = f;
                self.filter_error = None;
                self.persist_filter();
            }
            Err(e) => {
                self.filter_error = Some(e);
                // previous filter stays applied
            }
        }
        self.selected = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filter_text = "0".to_string();
        self.filter = Filter::default();
        self.filter_error = None;
        self.selected = 0;
        self.persist_filter();
    }

    fn persist_filter(&self) {
        self.persist_ui_state();
    }

    fn persist_ui_state(&self) {
        if let Some(store) = &self.store_dir {
            let state = UiState {
                filter: self.filter_text.clone(),
                sort: sort_to_str(self.sort).to_string(),
                reverse: self.reverse,
                keep_executables: self.keep_executables,
            };
            let _ = std::fs::create_dir_all(store);
            if let Ok(text) = serde_json::to_string_pretty(&state) {
                let _ = std::fs::write(ui_state_path(store), text);
            }
        }
    }

    fn selected_row(&self) -> Option<Row> {
        self.rows().into_iter().nth(self.selected)
    }

    /// Toggles a worktree's collapsed state (tree view only).
    /// `→`: go one level in. On an expandable row that means expanding
    /// it; in the projects view it opens the project; otherwise nothing,
    /// because there is nowhere further in.
    pub fn enter_row(&mut self) {
        if self.view == ViewKind::Projects {
            self.drill_into_selected();
            return;
        }
        if self.selected_row().is_some_and(|r| r.expandable) {
            self.toggle_expand();
        }
    }

    /// `←`: go one level out. Collapse an expanded row where that is what
    /// "out" means; otherwise leave the view, the same as `Esc`. At the
    /// projects view there is no level above, so it does nothing rather
    /// than quitting.
    pub fn leave_row(&mut self) {
        if self.view == ViewKind::Projects {
            return;
        }
        let expanded = self
            .selected_row()
            .is_some_and(|r| r.expandable && r.collapsed_children.is_none());
        if expanded {
            self.toggle_expand();
        } else {
            self.set_view(ViewKind::Projects);
        }
    }

    pub fn toggle_expand(&mut self) {
        if self.view != ViewKind::Tree {
            return;
        }
        if let Some(row) = self.selected_row()
            && row.expandable
        {
            // The label carries the path for worktree rows; reconstruct
            // the key the same way tree_rows does, from the report.
            let name = self
                .selected_project
                .clone()
                .or_else(|| self.report.projects.first().map(|p| p.name.clone()));
            if let Some(name) = name
                && let Some(p) = self.report.projects.iter().find(|p| p.name == name)
            {
                // Selected index among tree rows maps to worktree rows at depth 1.
                let rows = self.rows();
                if let Some(sel) = rows.get(self.selected) {
                    let idx = rows
                        .iter()
                        .take(self.selected + 1)
                        .filter(|r| r.depth == 1)
                        .count()
                        .saturating_sub(1);
                    if sel.depth == 1
                        && let Some(wt) = p.worktrees.get(idx)
                    {
                        let key = wt.path.display().to_string();
                        if self.collapsed.contains(&key) {
                            self.collapsed.remove(&key);
                        } else {
                            self.collapsed.insert(key);
                        }
                    } else if sel.depth == 2
                        && let Some(wt) = p.worktrees.get(idx)
                    {
                        // A Source row: expand it into its own directories.
                        let key = format!("source:{}", wt.path.display());
                        if self.collapsed.contains(&key) {
                            self.collapsed.remove(&key);
                        } else {
                            self.collapsed.insert(key);
                        }
                    }
                }
            }
        }
    }

    /// Enters a project from the projects view into its tree.
    pub fn drill_into_selected(&mut self) {
        if self.confirm_open {
            self.confirm_delete();
            return;
        }
        if self.view == ViewKind::Projects
            && let Some(row) = self.selected_row()
        {
            // Strip the worktree count and the `[rs][js]` ecosystem tags.
            let shown = row.label.split("  · ").next().unwrap_or("").trim();
            let shown = shown
                .rsplit("] ")
                .next()
                .unwrap_or(shown)
                .trim()
                .to_string();
            // The row shows `owner/repo`; the report keys on the project's
            // own name, so map back rather than searching for the label.
            let name = self
                .report
                .projects
                .iter()
                .find(|p| model::project_display_name(p) == shown)
                .map(|p| p.name.clone())
                .unwrap_or(shown);
            // Source rows start collapsed; the human opens the one they
            // care about with →.
            if let Some(p) = self.report.projects.iter().find(|p| p.name == name) {
                for wt in &p.worktrees {
                    self.collapsed
                        .insert(format!("source:{}", wt.path.display()));
                }
            }
            self.annotate_project(&name);
            self.selected_project = Some(name);
            self.set_view(ViewKind::Tree);
        }
    }

    /// Backspace: mark the selected row for deletion, or refuse inline
    /// with the reason.
    /// `a`: mark every row in this view the tool knows how to act on,
    /// so "everything we know of here" is one gesture and still one
    /// confirm. Rows it cannot act on are left alone, and the refusal
    /// names how many and why.
    pub fn mark_all_in_view(&mut self) {
        let rows = self.rows();
        let mut refused: Option<&'static str> = None;
        let mut marked = 0usize;
        for row in rows {
            let Some(kind) = row.kind.clone() else {
                // Projects view: each row stands for a whole project.
                if let Some(project) = row.project.clone() {
                    // Bulk marking never reaches for a checkout: `A` over
                    // a screen of projects would otherwise queue every
                    // checkout under the root behind one Enter.
                    let n = self.mark_project(&project, false);
                    if n == 0 {
                        refused = refused.or(Some("nothing reclaimable in this project"));
                    }
                    marked += n;
                }
                continue;
            };
            match crate::units::markable(&kind) {
                Ok(()) => {
                    if row.unit.is_some() {
                        self.mark_row(&row);
                        marked += 1;
                    }
                }
                Err(why) => refused = refused.or(Some(why)),
            }
        }
        if marked == 0 {
            self.set_refusal(refused.unwrap_or("nothing in this view can be acted on"));
            return;
        }
        self.confirm_open = true;
    }

    pub fn mark_selected(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        self.mark_row(&row);
    }

    /// Mark every reclaimable artifact in one project: dependency
    /// trees, build output, caches, and its Docker objects. The
    /// checkout, its `.git`, and its source tree are never included —
    /// removing those is a deliberate act one level in, on the row that
    /// names the worktree and carries its dirty/unpushed warnings.
    ///
    /// The project's own filter state is deliberately not applied: the
    /// projects row shows the project's whole size, so marking it acts
    /// on the whole project rather than on whatever the current filter
    /// happens to show. Returns how many units it newly marked; a second
    /// press on a fully marked project clears it and returns 0.
    fn mark_project(&mut self, project: &str, include_checkouts: bool) -> usize {
        let rows = model::tree_rows(
            &self.report,
            project,
            &Filter::default(),
            &std::collections::HashSet::new(),
            &self.track,
        );
        let mut units: Vec<Row> = rows
            .iter()
            .filter(|r| {
                r.unit.is_some()
                    && r.kind
                        .as_ref()
                        .is_some_and(|k| crate::units::markable(k).is_ok())
            })
            .cloned()
            .collect();
        if units.is_empty() && include_checkouts {
            // A project with nothing rebuildable in it -- a checkout and
            // its source, and that is all. Refusing here was the whole
            // complaint: the row is not actionable and the human has to
            // open it to reach the only thing there is. So the row
            // offers the checkouts themselves, each carrying its own
            // dirty / unpushed / untracked-content warnings onto the
            // confirm line.
            units = rows
                .iter()
                .filter(|r| r.worktree.is_some() && r.unit.is_some())
                .cloned()
                .collect();
        }
        if units.is_empty() {
            return 0;
        }
        let marked_already = |app: &Self, r: &Row| {
            r.unit
                .as_ref()
                .is_some_and(|u| app.marked.contains_key(&u.0))
        };
        if units.iter().all(|r| marked_already(self, r)) {
            for r in &units {
                if let Some(u) = &r.unit {
                    self.marked.remove(&u.0);
                }
            }
            return 0;
        }
        let mut newly = 0usize;
        for r in units {
            if marked_already(self, &r) {
                continue;
            }
            self.mark_row(&r);
            newly += 1;
        }
        newly
    }

    /// The mark decision for one row (testable without a selection).
    ///
    /// Anything with a path can be marked: an artifact, a Source
    /// directory, a linked worktree, a whole checkout. There is no bar —
    /// the human decides. What the tool owes them is the facts, so each
    /// unit carries its warnings and the confirm line states them before
    /// Enter. Docker objects are the one exception: there is no
    /// implementation to remove them yet, so marking one would be a lie.
    pub fn mark_row(&mut self, row: &Row) {
        let Some(unit_id) = row.unit.clone() else {
            if row.signals.iter().any(|s| s == "category") {
                self.set_refusal(
                    "Category total: select an unchecked child group. Nothing changed.",
                );
                return;
            }
            if row.signals.iter().any(|s| s == "blocked") {
                self.set_refusal(
                    "Inspection-only: this output cannot be selected for cleanup. Nothing changed.",
                );
                return;
            }
            // A projects-view row is a whole project rather than one
            // path. Marking it means marking what that project can give
            // back, so the human does not have to open it first.
            if let Some(project) = row.project.clone() {
                if self.mark_project(&project, true) == 0 {
                    self.set_refusal("nothing reclaimable in this project");
                }
                return;
            }
            self.set_refusal("nothing to delete on this row");
            return;
        };
        // The ignored/untracked rows report bytes scattered across a
        // checkout under the worktree's own path. Marking one would
        // queue the whole checkout, which is not what the row says.
        if let Some(kind) = &row.kind
            && matches!(
                kind,
                swamp_core::report::ArtifactKind::Ignored
                    | swamp_core::report::ArtifactKind::Untracked
            )
            && let Err(why) = crate::units::markable(kind)
        {
            self.set_refusal(why);
            return;
        }
        // A Docker object is removed through the daemon, not moved to
        // Trash. Which command that is depends on the kind, and build
        // cache has none.
        let docker = match row.kind {
            Some(swamp_core::report::ArtifactKind::DockerImage) => {
                Some(swamp_core::docker::Removal::Image {
                    id: unit_id.0.clone(),
                })
            }
            Some(swamp_core::report::ArtifactKind::DockerVolume) => {
                Some(swamp_core::docker::Removal::Volume {
                    name: unit_id.0.clone(),
                })
            }
            _ => None,
        };
        if self.marked.remove(&unit_id.0).is_some() {
            return; // toggle off
        }
        let mut warnings: Vec<String> = Vec::new();
        let worktree = row.worktree.clone().map(|wt| {
            let whole_checkout = !wt.linked;
            if whole_checkout && wt.remote.is_none() {
                warnings.push("no remote to restore from".into());
            }
            if wt.dirty == Some(true) {
                warnings.push("dirty".into());
            }
            match wt.unpushed {
                Some(n) if n > 0 => warnings.push(format!("{n} unpushed")),
                None => warnings.push("unpushed unknown".into()),
                _ => {}
            }
            if wt.locked == Some(true) {
                warnings.push("locked".into());
            }
            if whole_checkout {
                for (p, b) in swamp_core::ignore::untracked_content(&wt.path, 3, 100_000) {
                    let rel = p.strip_prefix(&wt.path).unwrap_or(&p).display().to_string();
                    warnings.push(format!("{rel} untracked {}", model::human_bytes(b)));
                }
            }
            crate::actions::WorktreeTerms {
                merge_complete: wt.merge_complete,
                pr: wt.pr.clone(),
                whole_checkout,
                remote: wt.remote.clone(),
            }
        });
        match row.track {
            Some(swamp_core::ignore::TrackState::Untracked) => {
                warnings.push("untracked: in no version control".into())
            }
            Some(swamp_core::ignore::TrackState::Tracked) if row.worktree.is_none() => {
                warnings.push("tracked source".into())
            }
            _ => {}
        }
        if row.kind == Some(swamp_core::report::ArtifactKind::Git) {
            warnings.push("git object store: history goes with it".into());
        }
        match row.kind {
            Some(swamp_core::report::ArtifactKind::DockerVolume) => warnings.push(
                "docker volume: its contents exist nowhere else, and this does not go to Trash"
                    .into(),
            ),
            Some(swamp_core::report::ArtifactKind::DockerImage) => warnings
                .push("docker image: permanent, comes back only by pulling or rebuilding".into()),
            Some(swamp_core::report::ArtifactKind::Loose) => {
                warnings.push("no project claims these bytes".into())
            }
            _ => {}
        }
        let label = row.label.trim().to_string();
        let unit_path = PathBuf::from(&unit_id.0);
        let cargo_plan = if self
            .report
            .nested_artifacts
            .iter()
            .any(|u| u.path == unit_path)
        {
            match swamp_core::actions::propose(
                &self.report,
                None,
                std::slice::from_ref(&unit_path),
                "human:tui",
            ) {
                Ok(plan) => {
                    warnings.extend(plan.units.iter().flat_map(|u| u.warnings.iter().cloned()));
                    Some(plan)
                }
                Err(e) => {
                    self.set_refusal(&e.to_string());
                    return;
                }
            }
        } else {
            None
        };
        // The worktree this unit lives in: where `bin/` goes when keeping
        // executables. A worktree row is its own worktree.
        let worktree_path = self
            .report
            .projects
            .iter()
            .flat_map(|p| p.worktrees.iter())
            .filter(|wt| unit_path.starts_with(&wt.path))
            .max_by_key(|wt| wt.path.as_os_str().len())
            .map(|wt| wt.path.clone())
            .unwrap_or_else(|| {
                unit_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default()
            });
        let selected_bytes = cargo_plan
            .as_ref()
            .map(|p| p.planned_bytes())
            .unwrap_or(row.bytes);
        self.marked.insert(
            unit_id.0.clone(),
            MarkedUnit {
                cargo_plan,
                path: unit_path,
                docker,
                worktree_path,
                bytes: selected_bytes,
                observed_at: self.report.observed_at,
                worktree,
                label,
                warnings,
            },
        );
    }

    fn set_refusal(&mut self, msg: &str) {
        self.refusal = Some((format!("refused: {msg}"), Instant::now()));
    }

    pub fn refusal_active(&self) -> Option<&str> {
        self.refusal.as_ref().and_then(|(msg, at)| {
            if at.elapsed() < REFUSAL_DISPLAY {
                Some(msg.as_str())
            } else {
                None
            }
        })
    }

    pub fn open_confirm(&mut self) {
        if !self.marked.is_empty() {
            self.confirm_open = true;
        }
    }

    /// Backspace: delete what is under the cursor. If nothing is marked,
    /// mark the current row; then ask once.
    pub fn delete_here(&mut self) {
        if self.marked.is_empty()
            && let Some(row) = self.selected_row()
        {
            self.mark_row(&row);
        }
        self.open_confirm();
    }

    pub fn cancel_confirm(&mut self) {
        self.confirm_open = false;
    }

    pub fn confirm_summary(&self) -> String {
        let units: Vec<MarkedUnit> = self.marked.values().cloned().collect();
        actions::confirm_summary(&units, self.keep_executables)
    }

    /// Enter on the confirm banner: this keypress at the keyboard is the
    /// human authorization for this one plan. Drives plan -> grant ->
    /// execute -> ledger, then re-observes the affected worktrees only.
    pub fn confirm_delete(&mut self) {
        if !self.confirm_open {
            return;
        }
        let units: Vec<MarkedUnit> = self.marked.values().cloned().collect();
        if units.is_empty() {
            self.confirm_open = false;
            return;
        }
        let planned: u64 = units.iter().map(|u| u.bytes).sum();
        let (plan, grant) = actions::authorize(&units, &self.actor);
        let trash = actions::trash_root();
        let free_before = actions::free_space_bytes(&trash);
        let ledger_path = crate::ledger_path();
        let ledger = match swamp_core::ledger::Ledger::open(&ledger_path) {
            Ok(l) => l,
            Err(e) => {
                self.set_refusal(&format!("could not open ledger: {e}"));
                self.confirm_open = false;
                return;
            }
        };
        let results = actions::execute_plan(
            &units,
            &plan,
            &grant,
            &ledger,
            &trash,
            &self.actor,
            self.keep_executables,
        );
        let free_after = actions::free_space_bytes(&trash);
        let measured = match (free_before, free_after) {
            (Some(b), Some(a)) => Some(a as i64 - b as i64),
            _ => None,
        };
        let ok = results.iter().filter(|r| r.outcome.is_ok()).count();
        let failed: Vec<String> = results
            .iter()
            .filter_map(|r| {
                r.outcome
                    .as_ref()
                    .err()
                    .map(|e| format!("{}: {e}", r.path.display()))
            })
            .collect();
        self.marked.clear();
        self.confirm_open = false;
        let measured_txt = measured
            .map(model::human_signed_bytes)
            .unwrap_or_else(|| "unmeasured".into());
        self.last_result = Some(if failed.is_empty() {
            format!(
                "{ok} deleted · planned {} · measured {measured_txt}",
                model::human_bytes(planned)
            )
        } else {
            format!(
                "{ok} deleted, {} refused · planned {} · measured {measured_txt}",
                failed.len(),
                model::human_bytes(planned)
            )
        });
        // What just left the disk leaves the screen now; the store and the
        // header follow from a background incremental observe (FSEvents
        // narrows it to the touched trees), the same path startup uses.
        // The screen is right now; the store follows through the live
        // FSEvents stream, which sees the move to Trash like any change.
        self.prune_removed(&results);
        if self.watch.is_none() {
            self.observe_in_background();
        }
    }

    /// Drops every row under a successfully removed path from the
    /// in-memory report -- artifacts, whole worktrees, Source directory
    /// rollups -- and takes their bytes off the header totals, so the
    /// screen is right before the re-observe lands.
    pub fn prune_removed(&mut self, results: &[actions::UnitResult]) {
        let removed: Vec<PathBuf> = results
            .iter()
            .filter(|r| r.outcome.is_ok())
            .map(|r| r.path.clone())
            .collect();
        if removed.is_empty() {
            return;
        }
        let under = |p: &std::path::Path| removed.iter().any(|r| p == r || p.starts_with(r));
        // Companion paths are part of the exact group, not just the selected
        // executable. Suppress stale nested facts until the observer refreshes.
        self.report.nested_artifacts.retain(|u| {
            !under(&u.path)
                && !removed.iter().any(|r| {
                    u.path == r.with_extension("d") || u.path.starts_with(r.with_extension("dSYM"))
                })
        });
        let mut freed = 0u64;
        let wt_paths: std::collections::HashMap<String, PathBuf> = self
            .report
            .projects
            .iter()
            .flat_map(|p| p.worktrees.iter())
            .map(|w| (w.worktree_id.clone(), w.path.clone()))
            .collect();
        for p in &mut self.report.projects {
            p.worktrees.retain(|wt| {
                if under(&wt.path) {
                    freed += wt.artifacts.iter().map(|a| a.bytes).sum::<u64>();
                    false
                } else {
                    true
                }
            });
            for wt in &mut p.worktrees {
                wt.artifacts.retain(|a| {
                    if under(&a.path) {
                        freed += a.bytes;
                        false
                    } else {
                        true
                    }
                });
            }
        }
        self.report.projects.retain(|p| !p.worktrees.is_empty());
        if let Some(map) = self.report.dirs_by_worktree.as_mut() {
            for (wt_id, rows) in map.iter_mut() {
                let Some(base) = wt_paths.get(wt_id) else {
                    continue;
                };
                rows.retain(|d| !under(&base.join(&d.rel_path)));
            }
        }
        // Bytes of a removed Source directory were counted inside the
        // worktree's Source row; the re-observe corrects that row. The
        // totals shrink by what we know left.
        let rec = &mut self.report.reconciliation;
        rec.attributed = rec.attributed.saturating_sub(freed);
        rec.walked_total = rec.walked_total.saturating_sub(freed);
        for path in &removed {
            self.track.remove(path);
            self.collapsed.remove(&format!("source:{}", path.display()));
        }
        if self.selected >= self.rows().len() {
            self.selected = self.rows().len().saturating_sub(1);
        }
    }

    /// How long the stream must be quiet before its changes are observed.
    /// FSEvents already coalesces at 0.5 s; this only lets a burst (a
    /// build writing thousands of files) land as one observation.
    pub const LIVE_QUIET: Duration = Duration::from_millis(400);

    /// Starts the live FSEvents stream. `None` (no store, or no FSEvents
    /// on this platform) leaves the TUI on the scheduled observer alone.
    pub fn start_watch(&mut self) {
        if self.store_dir.is_none() || self.watch.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        if let Some(w) = swamp_core::fs_events::watch(&self.root, tx) {
            self.watch = Some(w);
            self.watch_rx = Some(rx);
        }
    }

    /// Consumes every batch the stream has delivered so far.
    pub fn drain_watch(&mut self) {
        let Some(rx) = &self.watch_rx else {
            return;
        };
        while let Ok(batch) = rx.try_recv() {
            self.live_changes.extend(batch.changed_dirs);
            self.live_last_event_id = self.live_last_event_id.max(batch.last_event_id);
            self.live_last_batch = Some(Instant::now());
        }
    }

    /// Changes are waiting, no observation is running, and the stream has
    /// been quiet long enough.
    pub fn live_observe_due(&self) -> bool {
        self.pending.is_none()
            && !self.live_changes.is_empty()
            && self
                .live_last_batch
                .is_some_and(|t| t.elapsed() >= Self::LIVE_QUIET)
    }

    /// Observes exactly the directories the stream reported, on a worker
    /// thread, through the same pipeline as everything else: the plan is
    /// the live batch, so the store re-walks those subtrees and carries
    /// every other row forward.
    pub fn observe_live(&mut self) {
        let Some(store) = self.store_dir.clone() else {
            return;
        };
        if self.pending.is_some() || self.live_changes.is_empty() {
            return;
        }
        let changed: Vec<PathBuf> = self.live_changes.drain().collect();
        let device = std::fs::metadata(&self.root)
            .ok()
            .map(|m| std::os::unix::fs::MetadataExt::dev(&m));
        let plan = swamp_core::fs_events::FsEventsPlan::from_live(
            changed,
            self.live_last_event_id,
            device,
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let source = swamp_core::fs_events::testing::CannedSource(plan);
            let res = swamp_core::report::report_full_mode_with_source(
                &root,
                None,
                false,
                Some(&store),
                None,
                true,
                true,
                false,
                false,
                &source,
            );
            let _ = tx.send(res);
        });
        self.pending = Some(rx);
        self.observing = Some((0, 0));
    }

    /// Starts an incremental observation of the root on a worker thread;
    /// `event_loop` swaps the result in when it arrives. No-op without a
    /// store (fixture apps in tests) or while one is already running.
    pub fn observe_in_background(&mut self) {
        let Some(store) = self.store_dir.clone() else {
            return;
        };
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let res =
                swamp_core::report::report_with_dirs(&root, None, false, Some(&store), None, true);
            let _ = tx.send(res);
        });
        self.pending = Some(rx);
        self.observing = Some((0, 0));
    }

    pub fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swamp_core::entities::Confidence;
    use swamp_core::report::{
        ArtifactKind, ArtifactRow, ProjectRow, Reconciliation, Source, WorktreeKind, WorktreeRow,
    };

    fn fixture_report() -> Report {
        Report {
            observed_at: 1000,
            root: "/root".into(),
            projects: vec![ProjectRow {
                project_id: "p1".into(),
                name: "mole".into(),
                remote: None,
                ecosystems: Vec::new(),
                worktrees: vec![WorktreeRow {
                    worktree_id: "w1".into(),
                    path: "/root/mole".into(),
                    kind: WorktreeKind::Main,
                    artifacts: vec![
                        ArtifactRow {
                            kind: ArtifactKind::DependencyTree,
                            path: "/root/mole/node_modules".into(),
                            bytes: 200 * 1024 * 1024,
                            mtime_max: 0,
                            ecosystem: None,
                            hardlinked: false,
                            dedup_stale: false,
                            allocated_bytes: None,
                            allocated_growth_bytes: None,
                            local_bytes: 0,
                            track: None,
                            growth_bytes: Some(150 * 1024 * 1024),
                            regrowth_count: 0,
                            observed_at: 1000,
                            confidence: Confidence::High,
                            source: Source::new("test"),
                            note: None,
                            created_at: None,
                            containers: Vec::new(),
                            shared_with: Vec::new(),
                            dangling: false,
                        },
                        ArtifactRow {
                            kind: ArtifactKind::Source,
                            path: "/root/mole/src".into(),
                            bytes: 10 * 1024 * 1024,
                            mtime_max: 0,
                            ecosystem: None,
                            hardlinked: false,
                            dedup_stale: false,
                            allocated_bytes: None,
                            allocated_growth_bytes: None,
                            local_bytes: 0,
                            track: None,
                            growth_bytes: Some(1024),
                            regrowth_count: 0,
                            observed_at: 1000,
                            confidence: Confidence::High,
                            source: Source::new("test"),
                            note: None,
                            created_at: None,
                            containers: Vec::new(),
                            shared_with: Vec::new(),
                            dangling: false,
                        },
                    ],
                    signals: vec![],
                    branch: None,
                    github: None,
                    merge_complete: None,
                    idle_secs: None,
                }],
            }],
            unowned: vec![],
            reconciliation: Reconciliation {
                attributed: 0,
                unowned: 0,
                walked_total: 0,
                du_total: None,
                docker_attributed: 0,
                docker_unowned: 0,
            },
            notes: vec![],
            series_by_key: Default::default(),
            total_series: Vec::new(),
            series_window_secs: 0,
            summary: Default::default(),
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
            nested_artifacts: Vec::new(),
        }
    }

    #[test]
    fn a_project_row_marks_the_projects_artifacts_without_entering_it() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.clear_filter();
        assert_eq!(app.view, ViewKind::Projects);
        app.selected = 0;
        app.mark_selected();
        assert!(
            app.refusal_active().is_none(),
            "a project row is actionable from the projects view"
        );
        assert!(!app.marked.is_empty(), "the project's artifacts are marked");
        assert!(
            app.marked.keys().any(|k| k.ends_with("node_modules")),
            "the dependency tree is included: {:?}",
            app.marked.keys().collect::<Vec<_>>()
        );
        assert!(
            !app.marked.keys().any(|k| k.ends_with("/src")),
            "the source tree is not: {:?}",
            app.marked.keys().collect::<Vec<_>>()
        );
        // A second press clears the project rather than re-marking it.
        app.mark_selected();
        assert!(app.marked.is_empty());
    }

    /// The same report with every rebuildable artifact removed: a
    /// checkout, its source and nothing else.
    fn report_with_only_source() -> Report {
        let mut report = fixture_report();
        for p in report.projects.iter_mut() {
            for wt in p.worktrees.iter_mut() {
                wt.artifacts
                    .retain(|a| a.kind == swamp_core::report::ArtifactKind::Source);
            }
        }
        report
    }

    #[test]
    fn a_project_with_nothing_rebuildable_offers_its_checkout() {
        let mut app = App::new(report_with_only_source(), "/root".into());
        app.clear_filter();
        app.selected = 0;
        app.mark_selected();
        assert!(
            app.refusal_active().is_none(),
            "the row must do something rather than refuse"
        );
        assert!(
            app.marked.contains_key("/root/mole"),
            "the checkout is the only thing this project has: {:?}",
            app.marked.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn mark_all_never_reaches_for_a_checkout() {
        let mut app = App::new(report_with_only_source(), "/root".into());
        app.clear_filter();
        app.mark_all_in_view();
        assert!(
            app.marked.is_empty(),
            "A over a screen of projects must not queue checkouts: {:?}",
            app.marked.keys().collect::<Vec<_>>()
        );
        assert!(app.refusal_active().is_some());
    }

    #[test]
    fn backspace_on_a_project_row_opens_one_confirm() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.clear_filter();
        app.selected = 0;
        app.delete_here();
        assert!(
            app.confirm_open,
            "Backspace asks once for the whole project"
        );
        assert!(!app.marked.is_empty());
    }

    #[test]
    fn source_rows_mark_with_a_warning_instead_of_a_refusal() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.clear_filter();
        app.set_view(ViewKind::Tree);
        // row 0 = worktree, row 1 = node_modules, row 2 = src (a Source row)
        app.selected = 2;
        app.mark_selected();
        assert_eq!(app.marked.len(), 1, "anything with a path can be marked");
        assert!(app.refusal_active().is_none());
        // Space toggles it off again.
        app.mark_selected();
        assert!(app.marked.is_empty());
    }

    #[test]
    fn mark_all_marks_what_it_can_and_opens_one_confirm() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.set_view(ViewKind::Deps);
        app.mark_all_in_view();
        assert!(!app.marked.is_empty(), "dependency trees are actionable");
        assert!(app.confirm_open, "one confirm for the whole set");
    }

    #[test]
    fn right_goes_in_and_left_comes_back_out() {
        let mut app = App::new(fixture_report(), "/root".into());
        assert_eq!(app.view, ViewKind::Projects);
        // Left at the top level is not an exit: there is no level above.
        app.leave_row();
        assert_eq!(
            app.view,
            ViewKind::Projects,
            "left must not leave the projects view"
        );
        app.enter_row();
        assert_eq!(app.view, ViewKind::Tree, "right opens the project");
        // In the tree, left first collapses the expanded row under the
        // cursor, the way a file tree does; only then does it go out.
        app.leave_row();
        assert_eq!(app.view, ViewKind::Tree, "the first left collapsed the row");
        assert!(
            app.selected_row()
                .is_some_and(|r| r.collapsed_children.is_some()),
            "the row under the cursor is now collapsed"
        );
        app.leave_row();
        assert_eq!(
            app.view,
            ViewKind::Projects,
            "the second left comes back out"
        );
    }

    #[test]
    fn live_changes_are_observed_after_the_stream_goes_quiet() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.store_dir = Some(std::env::temp_dir());
        let (tx, rx) = std::sync::mpsc::channel();
        app.watch_rx = Some(rx);
        tx.send(swamp_core::fs_events::WatchBatch {
            changed_dirs: vec![PathBuf::from("/root/mole/node_modules")],
            last_event_id: 42,
        })
        .unwrap();
        app.drain_watch();
        assert_eq!(app.live_changes.len(), 1);
        assert_eq!(app.live_last_event_id, 42);
        assert!(!app.live_observe_due(), "not before the quiet window");
        app.live_last_batch = Some(Instant::now() - Duration::from_secs(1));
        assert!(app.live_observe_due());
        // While an observation runs, more changes just accumulate.
        let (_ptx, prx) = std::sync::mpsc::channel();
        app.pending = Some(prx);
        assert!(!app.live_observe_due());
    }

    #[test]
    fn prune_removed_drops_worktrees_dirs_and_bytes_under_the_removed_paths() {
        let mut app = App::new(fixture_report(), "/root".into());
        let before = app.report.reconciliation.attributed;
        let wt_path = app.report.projects[0].worktrees[0].path.clone();
        let art = app.report.projects[0].worktrees[0].artifacts[0].clone();
        let mut dirs = std::collections::HashMap::new();
        dirs.insert(
            app.report.projects[0].worktrees[0].worktree_id.clone(),
            vec![swamp_core::report::DirRollup {
                worktree_id: app.report.projects[0].worktrees[0].worktree_id.clone(),
                track: None,
                rel_path: "node_modules/x".into(),
                parent_rel_path: None,
                allocated_total: 1,
                own_allocated: 1,
                file_count: 1,
                entry_count: 1,
                symlink_count: 0,
                mod_time_min: 0,
                complete: true,
                growth_bytes: None,
            }],
        );
        app.report.dirs_by_worktree = Some(dirs);
        app.prune_removed(&[actions::UnitResult {
            path: art.path.clone(),
            outcome: Ok(swamp_core::execution::Outcome {
                unit_id: String::new(),
                status: "ok".into(),
                reason: None,
                intended_bytes: 0,
                observed_free_space_delta: None,
            }),
        }]);
        assert!(
            app.report.projects[0].worktrees[0]
                .artifacts
                .iter()
                .all(|a| a.path != art.path)
        );
        assert_eq!(
            app.report.reconciliation.attributed,
            before.saturating_sub(art.bytes)
        );
        assert!(
            app.report
                .dirs_by_worktree
                .as_ref()
                .unwrap()
                .values()
                .all(|rows| rows.is_empty()),
            "dir rollups under the removed artifact go too"
        );
        // A whole worktree removal empties its project.
        app.prune_removed(&[actions::UnitResult {
            path: wt_path,
            outcome: Ok(swamp_core::execution::Outcome {
                unit_id: String::new(),
                status: "ok".into(),
                reason: None,
                intended_bytes: 0,
                observed_free_space_delta: None,
            }),
        }]);
        assert!(app.report.projects.is_empty());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn backspace_marks_the_current_row_and_asks_once() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.set_view(ViewKind::Tree);
        app.selected = 1; // node_modules
        app.delete_here();
        assert_eq!(app.marked.len(), 1);
        assert!(app.confirm_open, "one 'are you sure', with the facts on it");
        assert!(app.confirm_summary().contains("node_modules"));
        assert!(app.confirm_summary().contains("Enter yes"));
    }

    #[test]
    fn mark_and_confirm_full_flow() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.set_view(ViewKind::Tree);
        app.selected = 1; // node_modules
        app.mark_selected();
        assert_eq!(app.marked.len(), 1);
        app.open_confirm();
        assert!(app.confirm_open);
        assert!(app.confirm_summary().contains("delete node_modules"));
    }

    #[test]
    fn refusal_expires_after_display_window() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.set_refusal("test reason");
        assert!(app.refusal_active().is_some());
        app.refusal = Some((
            "refused: test".into(),
            Instant::now() - Duration::from_secs(5),
        ));
        assert!(app.refusal_active().is_none());
    }

    #[test]
    fn clear_filter_shows_zero_and_none() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.clear_filter();
        assert_eq!(app.filter, Filter::default());
        assert_eq!(app.filter_text, "0");
    }

    #[test]
    fn bad_filter_keeps_previous_and_sets_error() {
        let mut app = App::new(fixture_report(), "/root".into());
        let before = app.filter.clone();
        app.filter_text = "bananas".to_string();
        app.commit_filter();
        assert_eq!(app.filter, before);
        assert!(app.filter_error.is_some());
    }

    #[test]
    fn view_cycles_and_digit_keys() {
        assert_eq!(ViewKind::Projects.next(), ViewKind::Tree);
        assert_eq!(ViewKind::from_digit('3'), Some(ViewKind::Builds));
        assert_eq!(ViewKind::from_digit('6'), Some(ViewKind::Kinds));
        assert_eq!(ViewKind::from_digit('9'), None);
        // Full cycle returns to Projects, matching the CLI's view order:
        // worktrees(Projects)/tree/builds/deps/docker/kinds/unowned/types.
        assert_eq!(ViewKind::from_digit('8'), Some(ViewKind::Types));
        let mut v = ViewKind::Projects;
        for _ in 0..8 {
            v = v.next();
        }
        assert_eq!(v, ViewKind::Projects);
    }
}
