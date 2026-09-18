//! `slop-livin ui`: the diffstat-ledger terminal UI. See DESIGN.md and
//! `.impeccable/surfaces/tui.md` (binding). Renders the same
//! `slop_livin_core::report_with` `Report` the CLI/MCP use; no second
//! data path.

pub mod actions;
pub mod app;
pub mod filter;
pub mod model;
pub mod picker;
pub mod ui;
pub mod units;

use anyhow::Result;
use app::{App, ViewKind};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use model::Sort;
use ratatui::Terminal;
use ratatui::backend::Backend;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn ledger_path() -> PathBuf {
    if let Ok(dir) = std::env::var("SLOP_LIVIN_LEDGER_PATH") {
        return PathBuf::from(dir);
    }
    let dir = if let Ok(d) = std::env::var("SLOP_LIVIN_DIR") {
        PathBuf::from(d)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".local/share/slop-livin")
    };
    dir.join("ledger.jsonl")
}

/// Dispatches one key event against the app state. Kept separate from
/// the terminal event loop so it is directly unit-testable.
pub fn handle_key(app: &mut App, code: KeyCode) {
    handle_key_mod(app, code, false)
}

/// `shift` distinguishes Shift-→/Shift-← inside the picker's growth field.
pub fn handle_key_mod(app: &mut App, code: KeyCode, shift: bool) {
    if let Some(p) = app.picker.as_mut() {
        match code {
            KeyCode::Up => p.up(),
            KeyCode::Down => p.down(),
            KeyCode::Right if shift => p.cycle_secondary(1),
            KeyCode::Left if shift => p.cycle_secondary(-1),
            KeyCode::Right => p.cycle(1),
            KeyCode::Left => p.cycle(-1),
            KeyCode::Backspace => p.backspace(),
            KeyCode::Enter => app.apply_picker(),
            KeyCode::Esc => app.picker = None,
            KeyCode::Char('e') if p.field != 2 => app.picker_to_raw_edit(),
            KeyCode::Char(c) if p.field == 2 => p.type_char(c),
            KeyCode::Char('0') => {
                app.picker = None;
                app.clear_filter();
            }
            _ => {}
        }
        return;
    }
    if app.editing_filter {
        match code {
            KeyCode::Enter => app.commit_filter(),
            KeyCode::Esc => app.cancel_filter_edit(),
            KeyCode::Backspace => app.filter_backspace(),
            KeyCode::Tab => app.filter_tab_complete(),
            KeyCode::Char(c) => app.filter_input(c),
            _ => {}
        }
        return;
    }
    if app.help_open {
        if matches!(code, KeyCode::Char('?') | KeyCode::Esc) {
            app.toggle_help();
        }
        return;
    }
    match code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Right => app.toggle_expand(),
        KeyCode::Left => app.toggle_expand(),
        KeyCode::Enter => app.drill_into_selected(),
        KeyCode::Esc => app.cancel_confirm(),
        KeyCode::Backspace => app.mark_selected(),
        KeyCode::Char('/') => app.open_picker(),
        KeyCode::Char(':') => app.start_filter_edit(),
        KeyCode::Char('0') => app.clear_filter(),
        KeyCode::Char('v') => app.set_view(app.view.next()),
        KeyCode::Char(d @ '1'..='5') => {
            if let Some(v) = ViewKind::from_digit(d) {
                app.set_view(v);
            }
        }
        KeyCode::Char('g') => app.set_sort(Sort::Growth),
        KeyCode::Char('s') => app.set_sort(Sort::Size),
        KeyCode::Char('?') => app.toggle_help(),
        _ => {}
    }
}

/// Runs the interactive UI against `root`. `no_observe` skips persisting
/// a new observation (read-only report, same as `slop-livin report
/// --no-observe`).
fn store_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SLOP_LIVIN_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/slop-livin")
}

pub fn run(root: &Path, no_observe: bool) -> Result<()> {
    let store = store_dir();
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // Paint the last cached report immediately (milliseconds); observe in
    // the background and swap the result in. With no cache yet, the first
    // observation has to happen before there is anything to show.
    let cached = slop_livin_core::report::load_last_report(&store, &root);
    let (tx, rx) = std::sync::mpsc::channel::<Result<slop_livin_core::Report>>();
    let mut app = match cached {
        Some(r) if no_observe => {
            let mut a = App::new(r, root.clone());
            a.observed_label = "from last observation".into();
            a
        }
        Some(r) => {
            let mut a = App::new(r, root.clone());
            a.observed_label = "from last observation".into();
            a.observing = Some((0, 0));
            let (root2, store2) = (root.clone(), store.clone());
            std::thread::spawn(move || {
                let res =
                    slop_livin_core::report::report_with(&root2, None, false, Some(&store2), None);
                let _ = tx.send(res);
            });
            a
        }
        None => {
            let report = slop_livin_core::report::report_full_mode(
                &root,
                None,
                false,
                Some(&store),
                None,
                !no_observe,
                false,
                false,
                false,
            )?;
            App::new(report, root.clone())
        }
    };
    app.pending = Some(rx);

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = event_loop(&mut terminal, &mut app);

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    result
}

fn event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        if let Some(rx) = &app.pending
            && let Ok(res) = rx.try_recv()
        {
            app.pending = None;
            app.observing = None;
            match res {
                Ok(r) => {
                    app.replace_report(r);
                    app.observed_label = "just now".into();
                }
                Err(e) => app.status = Some(format!("observation failed: {e}")),
            }
        }
        if let Ok(sz) = terminal.size() {
            app.width = sz.width;
        }
        terminal.draw(|f| ui::draw(f, app))?;
        if app.quit {
            return Ok(());
        }
        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key_mod(
                app,
                key.code,
                key.modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use slop_livin_core::report::{Reconciliation, Report};

    fn empty_report() -> Report {
        Report {
            observed_at: 0,
            root: "/root".into(),
            projects: vec![],
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
    fn slash_edits_existing_filter_text_and_esc_restores_it() {
        let mut app = App::new(empty_report(), "/root".into());
        let before = app.filter_text.clone();
        handle_key(&mut app, KeyCode::Char(':'));
        assert!(app.editing_filter);
        assert_eq!(app.filter_text, before, "existing text stays editable");
        for _ in 0..before.len() {
            handle_key(&mut app, KeyCode::Backspace);
        }
        for c in "kind:BuildOutput".chars() {
            handle_key(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.filter_text, "kind:BuildOutput");
        handle_key(&mut app, KeyCode::Esc);
        assert!(!app.editing_filter);
        assert_eq!(app.filter_text, before, "Esc restores the previous filter");
        handle_key(&mut app, KeyCode::Char(':'));
        for c in " idle > 48h".chars() {
            handle_key(&mut app, KeyCode::Char(c));
        }
        handle_key(&mut app, KeyCode::Enter);
        assert!(!app.editing_filter);
        assert!(app.filter_error.is_none(), "{:?}", app.filter_error);
        assert!(app.filter_text.ends_with("idle > 48h"));
    }

    #[test]
    fn slash_opens_picker_and_enter_applies_its_filter() {
        let mut app = App::new(empty_report(), "/root".into());
        handle_key(&mut app, KeyCode::Char('/'));
        assert!(app.picker.is_some());
        handle_key(&mut app, KeyCode::Down); // kind
        handle_key(&mut app, KeyCode::Right); // build
        handle_key(&mut app, KeyCode::Enter);
        assert!(app.picker.is_none());
        assert_eq!(app.filter_text, "growth > 100MB in 7d kind:BuildOutput");
        assert!(app.filter_error.is_none());
        handle_key(&mut app, KeyCode::Char('/'));
        handle_key(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert_eq!(
            app.filter_text, "growth > 100MB in 7d kind:BuildOutput",
            "Esc keeps the applied filter"
        );
    }

    #[test]
    fn header_drops_trailing_clauses_to_fit_width() {
        let clauses: Vec<String> = [
            "/root",
            "observed just now",
            "56 projects",
            "36GB attributed",
            "193MB unowned",
            "docker 14GB unowned",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let full = ui::fit_clauses(&clauses, 200);
        assert!(full.ends_with("docker 14GB unowned"));
        let narrow = ui::fit_clauses(&clauses, 50);
        assert!(narrow.chars().count() <= 50, "{narrow:?}");
        assert!(narrow.starts_with("/root · observed just now"));
        assert!(!narrow.contains("docker"));
        assert_eq!(
            ui::fit_clauses(&clauses, 3),
            "/root",
            "first clause always kept"
        );
    }

    #[test]
    fn q_quits() {
        let mut app = App::new(empty_report(), "/root".into());
        handle_key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn draws_without_panicking_at_both_first_class_sizes() {
        for (w, h) in [(80u16, 24u16), (200u16, 60u16)] {
            let backend = TestBackend::new(w, h);
            let mut terminal = Terminal::new(backend).unwrap();
            let app = App::new(empty_report(), "/root".into());
            terminal.draw(|f| ui::draw(f, &app)).unwrap();
        }
    }
}
