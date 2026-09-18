//! Captures ratatui `TestBackend` frames for every view and state at both
//! first-class terminal sizes (80x24, 200x60), and commits them under
//! `tests/frames/*.txt` for hand-checked alignment review.
//!
//! Regenerate with `UPDATE_FRAMES=1 cargo test -p slop-livin-tui
//! --test frames`; otherwise the test asserts the committed frame is
//! still exactly reproduced.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use slop_livin_core::entities::Confidence;
use slop_livin_core::report::{
    ArtifactKind, ArtifactRow, ProjectRow, Reconciliation, Report, Signal, Source, UnownedReason,
    UnownedRow, WorktreeKind, WorktreeRow,
};
use slop_livin_tui::app::{App, ViewKind};
use slop_livin_tui::ui;
use std::path::PathBuf;

fn art(kind: ArtifactKind, path: &str, bytes: u64, growth: Option<i64>) -> ArtifactRow {
    ArtifactRow {
        kind,
        path: PathBuf::from(path),
        bytes,
        local_bytes: 0,
        growth_bytes: growth,
        regrowth_count: 0,
        observed_at: 1_726_000_000,
        confidence: Confidence::High,
        source: Source::new("fixture"),
        note: None,
        created_at: None,
        containers: Vec::new(),
        shared_with: Vec::new(),
        dangling: false,
    }
}

fn fixture_report() -> Report {
    Report {
        observed_at: 1_726_000_000,
        root: PathBuf::from("/Users/dev/src"),
        projects: vec![
            ProjectRow {
                project_id: "p-mole".into(),
                name: "mole".into(),
                remote: None,
                worktrees: vec![WorktreeRow {
                    worktree_id: "w-mole".into(),
                    path: PathBuf::from("/Users/dev/src/mole"),
                    kind: WorktreeKind::Main,
                    artifacts: vec![
                        art(
                            ArtifactKind::DependencyTree,
                            "/Users/dev/src/mole/node_modules",
                            420 * 1024 * 1024,
                            Some(180 * 1024 * 1024),
                        ),
                        art(
                            ArtifactKind::BuildOutput,
                            "/Users/dev/src/mole/target",
                            2_147_483_648,
                            Some(1_073_741_824),
                        ),
                        art(
                            ArtifactKind::Source,
                            "/Users/dev/src/mole/src",
                            12 * 1024 * 1024,
                            Some(2048),
                        ),
                    ],
                    signals: vec![Signal {
                        name: "dirty".into(),
                        value: "clean".into(),
                    }],
                    branch: None,
                    github: None,
                    merge_complete: None,
                    idle_secs: None,
                }],
            },
            ProjectRow {
                project_id: "p-slop".into(),
                name: "slop-livin".into(),
                remote: None,
                worktrees: vec![WorktreeRow {
                    worktree_id: "w-slop".into(),
                    path: PathBuf::from("/Users/dev/src/slop-livin"),
                    kind: WorktreeKind::Main,
                    artifacts: vec![
                        art(
                            ArtifactKind::BuildOutput,
                            "/Users/dev/src/slop-livin/target",
                            6_442_450_944,
                            Some(2_684_354_560),
                        ),
                        art(
                            ArtifactKind::DockerImage,
                            "/Users/dev/src/slop-livin/.docker/img",
                            1_610_612_736,
                            Some(-104_857_600),
                        ),
                    ],
                    signals: vec![],
                    branch: None,
                    github: None,
                    merge_complete: None,
                    idle_secs: None,
                }],
            },
        ],
        unowned: vec![UnownedRow {
            path_or_object: "/Users/dev/.cache/leftover".into(),
            bytes: 209_715_200,
            reason: UnownedReason::SharedCache,
            docker_kind: None,
            shared_bytes: None,
            note: None,
            created_at: None,
            containers: Vec::new(),
            shared_with: Vec::new(),
            dangling: false,
        }],
        dirs_by_worktree: None,
        files_by_worktree: None,
        schedule_line: None,
        github_enrichment: None,
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

fn capture(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal.backend().to_string()
}

fn check(name: &str, got: &str) {
    let path = format!("{}/tests/frames/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("UPDATE_FRAMES").is_ok() {
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing frame fixture {path}; run with UPDATE_FRAMES=1"));
    assert_eq!(got, want, "frame {name} does not match committed fixture");
}

#[test]
fn projects_view() {
    for (w, h) in [(80, 24), (200, 60)] {
        let app = App::new(fixture_report(), "/Users/dev/src".into());
        check(&format!("projects_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn tree_view() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.set_view(ViewKind::Tree);
        app.selected_project = Some("mole".into());
        check(&format!("tree_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn kinds_view() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.set_view(ViewKind::Kinds);
        check(&format!("kinds_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn docker_view() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.set_view(ViewKind::Docker);
        check(&format!("docker_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn unowned_view() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.set_view(ViewKind::Unowned);
        check(&format!("unowned_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn marked_rows_state() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.clear_filter();
        app.set_view(ViewKind::Tree);
        app.selected_project = Some("mole".into());
        // Rows sort by growth desc within the worktree now (tree.rs):
        // build (index 1, +1.0GB) before deps/node_modules (index 2,
        // +180.0MB) before source (index 3).
        app.selected = 2; // node_modules (deps)
        app.mark_selected();
        check(&format!("marked_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn confirm_summary_state() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.clear_filter();
        app.set_view(ViewKind::Tree);
        app.selected_project = Some("mole".into());
        app.selected = 2; // node_modules (deps); see marked_rows_state.
        app.mark_selected();
        app.open_confirm();
        check(&format!("confirm_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn refusal_footer_state() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.clear_filter();
        app.set_view(ViewKind::Tree);
        app.selected_project = Some("mole".into());
        app.selected = 3; // source/src, not markable (last after the
        // growth-desc sort: build, deps, then source).
        app.mark_selected();
        check(&format!("refusal_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn docker_mark_refusal_state() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.clear_filter();
        app.set_view(ViewKind::Tree);
        app.selected_project = Some("slop-livin".into());
        // slop-livin's worktree has one build row (higher growth) then
        // one Docker image row; select the Docker row.
        app.selected = 2;
        app.mark_selected();
        check(
            &format!("docker_mark_refusal_{w}x{h}"),
            &capture(&app, w, h),
        );
    }
}

#[test]
fn empty_filter_state() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.filter_text = "growth > 900GB in 7d".into();
        app.commit_filter();
        check(&format!("empty_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn help_overlay_state() {
    for (w, h) in [(80, 24), (200, 60)] {
        let mut app = App::new(fixture_report(), "/Users/dev/src".into());
        app.toggle_help();
        check(&format!("help_{w}x{h}"), &capture(&app, w, h));
    }
}

#[test]
fn picker_frame() {
    let mut app = App::new(fixture_report(), std::path::PathBuf::from("/Users/dev/src"));
    app.width = 200;
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Char('/'));
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Down);
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Down);
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Right);
    let got = capture(&app, 200, 60);
    assert!(got.contains("▸ kind"), "kind field selected:\n{got}");
    assert!(
        got.contains("kind:BuildOutput"),
        "composed filter shown:\n{got}"
    );
    check("picker_200x60", &got);
}

#[test]
fn drill_shows_view_scope_and_esc_returns_to_projects() {
    let mut app = App::new(fixture_report(), std::path::PathBuf::from("/Users/dev/src"));
    app.width = 200;
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Char('0'));
    let before = capture(&app, 200, 60);
    assert!(before.contains("view: projects · filter: 0"), "{before}");
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Enter);
    assert_eq!(app.view, ViewKind::Tree);
    let tree = capture(&app, 200, 60);
    assert!(
        tree.contains("view: tree of "),
        "second line must name the scope:\n{tree}"
    );
    assert!(tree.contains("(Esc back)"), "{tree}");
    slop_livin_tui::handle_key(&mut app, crossterm::event::KeyCode::Esc);
    assert_eq!(app.view, ViewKind::Projects);
    let back = capture(&app, 200, 60);
    assert!(back.contains("view: projects"), "{back}");
}

#[test]
fn linked_worktree_is_markable_but_main_checkout_and_dirty_are_refused() {
    use crossterm::event::KeyCode;
    let mut app = App::new(fixture_report(), std::path::PathBuf::from("/Users/dev/src"));
    app.width = 200;
    slop_livin_tui::handle_key(&mut app, KeyCode::Char('0'));
    slop_livin_tui::handle_key(&mut app, KeyCode::Enter); // drill into first project
    assert_eq!(app.view, ViewKind::Tree);
    // Row 0 is the Main checkout: refused with the fact.
    slop_livin_tui::handle_key(&mut app, KeyCode::Backspace);
    let f = capture(&app, 200, 60);
    assert!(
        f.contains("main checkout"),
        "main checkout refusal expected:\n{f}"
    );
    assert!(app.marked.is_empty());
}

#[test]
fn worktree_mark_rules() {
    use slop_livin_tui::model::{Row, WorktreeMark};
    let mark = |linked: bool, dirty: Option<bool>, unpushed: Option<u32>, locked: Option<bool>| {
        WorktreeMark {
            path: "/x/wt".into(),
            linked,
            dirty,
            unpushed,
            locked,
            merge_complete: false,
            pr: None,
        }
    };
    let row = |m: WorktreeMark| Row {
        depth: 1,
        rail: String::new(),
        label: "linked /x/wt".into(),
        bytes: 1,
        growth: None,
        signals: vec![],
        unit: Some(slop_livin_tui::units::UnitId::for_artifact(
            std::path::Path::new("/x/wt"),
        )),
        kind: None,
        worktree: Some(m),
        track: None,
        collapsed_children: None,
        expandable: false,
    };
    let mut app = App::new(fixture_report(), std::path::PathBuf::from("/Users/dev/src"));
    for (m, expect_marked, cause) in [
        (
            mark(false, Some(false), Some(0), Some(false)),
            false,
            "main checkout",
        ),
        (mark(true, Some(true), Some(0), Some(false)), false, "dirty"),
        (
            mark(true, Some(false), Some(3), Some(false)),
            false,
            "3 unpushed",
        ),
        (
            mark(true, Some(false), Some(0), Some(true)),
            false,
            "locked",
        ),
        (
            mark(true, Some(false), None, Some(false)),
            false,
            "unpushed count unknown",
        ),
        (mark(true, Some(false), Some(0), Some(false)), true, ""),
    ] {
        app.marked.clear();
        app.refusal = None;
        app.mark_row(&row(m));
        assert_eq!(
            !app.marked.is_empty(),
            expect_marked,
            "cause {cause}: {:?}",
            app.refusal
        );
        if !expect_marked {
            assert!(
                app.refusal.as_ref().is_some_and(|r| r.0.contains(cause)),
                "{cause}: {:?}",
                app.refusal
            );
        }
    }
}
