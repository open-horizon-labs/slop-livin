//! Renders the real TUI against the real store, for eyeballing frames.
//! `cargo run -p slop-livin-tui --example render_real -- <project> [keys]`
use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use slop_livin_tui::{app::App, handle_key, ui};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project = args.first().cloned().unwrap_or_default();
    let keys = args.get(1).cloned().unwrap_or_default();
    let root = std::path::PathBuf::from("/Users/muness1/src");
    let store =
        std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/slop-livin");
    let report = slop_livin_core::report::report_full_mode(
        &root,
        None,
        false,
        Some(&store),
        None,
        false,
        true,
        false,
        false,
    )
    .expect("report");
    let mut app = App::new(report, root);
    app.width = 200;
    handle_key(&mut app, KeyCode::Char('0'));
    if !project.is_empty() {
        app.selected_project = Some(project.clone());
        app.annotate_project(&project);
        app.set_view(slop_livin_tui::app::ViewKind::Tree);
        for wt in app
            .report
            .projects
            .iter()
            .find(|p| p.name == project)
            .map(|p| p.worktrees.clone())
            .unwrap_or_default()
        {
            app.collapsed
                .insert(format!("source:{}", wt.path.display()));
        }
    }
    for k in keys.chars() {
        match k {
            'd' => handle_key(&mut app, KeyCode::Down),
            'r' => handle_key(&mut app, KeyCode::Right),
            'u' => handle_key(&mut app, KeyCode::Up),
            'b' => handle_key(&mut app, KeyCode::Backspace),
            c => handle_key(&mut app, KeyCode::Char(c)),
        }
    }
    let mut t = Terminal::new(TestBackend::new(200, 40)).unwrap();
    t.draw(|f| ui::draw(f, &app)).unwrap();
    print!("{}", t.backend());
}
