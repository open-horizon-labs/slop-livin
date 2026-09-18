//! `slop-livin ui`: the diffstat-ledger terminal UI. See DESIGN.md and
//! `.impeccable/surfaces/tui.md` (binding). Renders the same
//! `slop_livin_core::report_with` `Report` the CLI/MCP use; no second
//! data path.

pub mod actions;
pub mod app;
pub mod filter;
pub mod model;
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
    if app.editing_filter {
        match code {
            KeyCode::Enter => app.commit_filter(),
            KeyCode::Esc => app.editing_filter = false,
            KeyCode::Backspace => app.filter_backspace(),
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
        KeyCode::Char('/') => {
            app.filter_text.clear();
            app.start_filter_edit();
        }
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
pub fn run(root: &Path, no_observe: bool) -> Result<()> {
    let store_dir = if no_observe {
        None
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Some(PathBuf::from(home).join(".local/share/slop-livin"))
    };
    let report =
        slop_livin_core::report::report_with(root, None, false, store_dir.as_deref(), None)?;
    let mut app = App::new(report, root.to_path_buf());

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
        terminal.draw(|f| ui::draw(f, app))?;
        if app.quit {
            return Ok(());
        }
        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key.code);
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
        }
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
