//! Application state and key handling. Pure aside from the action layer
//! calls the confirm step makes; the rest is unit-testable without a
//! terminal.

use crate::actions::{self, MarkedUnit};
use crate::filter::{self, Filter};
use crate::model::{self, Row, Sort};
use crate::units::markable;
use slop_livin_core::report::Report;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
            ViewKind::Unowned => ViewKind::Projects,
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
    pub refusal: Option<(String, Instant)>,
    pub observing: Option<(u32, u32)>,
    pub last_result: Option<String>,
    pub observed_label: String,
    pub actor: String,
    pub sort: Sort,
    pub quit: bool,
    /// Terminal width at the last draw; the header fits its clauses to it.
    pub width: u16,
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
            refusal: None,
            observing: None,
            last_result: None,
            observed_label: "just now".to_string(),
            actor: "human".to_string(),
            sort: Sort::None,
            quit: false,
            width: 0,
        }
    }

    /// Swap in a fresh report (background observation finished). Rows are
    /// derived from `report` on demand, so nothing else needs rebuilding;
    /// the selection is clamped by `rows()` consumers.
    pub fn replace_report(&mut self, report: Report) {
        self.report = report;
        self.selected = 0;
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
                    Some(n) => model::tree_rows(&self.report, &n, &self.filter, &self.collapsed),
                    None => Vec::new(),
                };
            }
            ViewKind::Builds => model::builds_rows(&self.report),
            ViewKind::Deps => model::deps_rows(&self.report),
            ViewKind::Kinds => model::kinds_rows(&self.report, &self.filter),
            ViewKind::Docker => model::docker_rows(&self.report),
            ViewKind::Unowned => model::unowned_rows(&self.report),
        };
        model::apply_sort(&mut rows, self.sort);
        rows
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = if self.sort == sort { Sort::None } else { sort };
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

    pub fn cancel_filter_edit(&mut self) {
        self.filter_text = std::mem::take(&mut self.filter_before_edit);
        self.editing_filter = false;
    }

    pub fn filter_input(&mut self, c: char) {
        self.filter_text.push(c);
    }

    pub fn filter_backspace(&mut self) {
        self.filter_text.pop();
    }

    pub fn commit_filter(&mut self) {
        self.editing_filter = false;
        match filter::parse(&self.filter_text) {
            Ok(f) => {
                self.filter = f;
                self.filter_error = None;
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
    }

    fn selected_row(&self) -> Option<Row> {
        self.rows().into_iter().nth(self.selected)
    }

    /// Toggles a worktree's collapsed state (tree view only).
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
            let name = row.label.split(" (").next().unwrap_or("").to_string();
            self.selected_project = Some(name);
            self.set_view(ViewKind::Tree);
        }
    }

    /// Backspace: mark the selected row for deletion, or refuse inline
    /// with the reason.
    pub fn mark_selected(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let Some(unit_id) = row.unit.clone() else {
            self.set_refusal("not a unit: nothing here can be marked");
            return;
        };
        let Some(kind) = row.kind.clone() else {
            self.set_refusal("not a unit: nothing here can be marked");
            return;
        };
        // Docker rows are markable in principle (folded units, same as a
        // dependency tree or build output), but there is no daemon-side
        // sink recheck yet to safely re-verify a Docker object right
        // before deletion the way the filesystem action layer does for a
        // path. Refuse with a specific, honest reason instead of
        // pretending the mark will do anything (#33).
        if matches!(
            kind,
            slop_livin_core::report::ArtifactKind::DockerImage
                | slop_livin_core::report::ArtifactKind::DockerBuildCache
                | slop_livin_core::report::ArtifactKind::DockerVolume
        ) {
            self.set_refusal("docker removal not available yet");
            return;
        }
        match markable(&kind) {
            Ok(()) => {
                if self.marked.remove(&unit_id.0).is_none() {
                    self.marked.insert(
                        unit_id.0.clone(),
                        MarkedUnit {
                            path: PathBuf::from(&unit_id.0),
                            bytes: row.bytes,
                            observed_at: self.report.observed_at,
                        },
                    );
                }
            }
            Err(reason) => self.set_refusal(reason),
        }
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

    pub fn cancel_confirm(&mut self) {
        self.confirm_open = false;
    }

    pub fn confirm_summary(&self) -> String {
        let units: Vec<MarkedUnit> = self.marked.values().cloned().collect();
        actions::confirm_summary(&units)
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
        let ledger = match slop_livin_core::ledger::Ledger::open(&ledger_path) {
            Ok(l) => l,
            Err(e) => {
                self.set_refusal(&format!("could not open ledger: {e}"));
                self.confirm_open = false;
                return;
            }
        };
        let results = actions::execute_plan(&units, &plan, &grant, &ledger, &trash, &self.actor);
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
        // Re-observe only the affected worktrees: re-running the walk for
        // the whole root is out of scope here, so refresh growth/bytes
        // for the units' own paths by dropping them from the in-memory
        // report (they are gone) and marking bytes 0 where they used to
        // be, which is the minimum "affected worktree" refresh without a
        // second data path.
        self.reobserve_after_delete(&results);
    }

    fn reobserve_after_delete(&mut self, results: &[actions::UnitResult]) {
        let removed: HashSet<String> = results
            .iter()
            .filter(|r| r.outcome.is_ok())
            .map(|r| r.path.display().to_string())
            .collect();
        for p in &mut self.report.projects {
            for wt in &mut p.worktrees {
                wt.artifacts
                    .retain(|a| !removed.contains(&a.path.display().to_string()));
            }
        }
    }

    pub fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slop_livin_core::entities::Confidence;
    use slop_livin_core::report::{
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
                worktrees: vec![WorktreeRow {
                    worktree_id: "w1".into(),
                    path: "/root/mole".into(),
                    kind: WorktreeKind::Main,
                    artifacts: vec![
                        ArtifactRow {
                            kind: ArtifactKind::DependencyTree,
                            path: "/root/mole/node_modules".into(),
                            bytes: 200 * 1024 * 1024,
                            local_bytes: 0,
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
                            local_bytes: 0,
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
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
        }
    }

    #[test]
    fn mark_refuses_non_artifact_kinds() {
        let mut app = App::new(fixture_report(), "/root".into());
        app.clear_filter();
        app.set_view(ViewKind::Tree);
        // row 0 = worktree, row 1 = node_modules (markable), row 2 = src (not markable)
        app.selected = 2;
        app.mark_selected();
        assert!(app.marked.is_empty());
        assert!(app.refusal_active().unwrap().contains("source trees"));
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
        assert!(app.confirm_summary().contains("delete 1 unit"));
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
        // worktrees(Projects)/tree/builds/deps/docker/kinds/unowned.
        let mut v = ViewKind::Projects;
        for _ in 0..7 {
            v = v.next();
        }
        assert_eq!(v, ViewKind::Projects);
    }
}
