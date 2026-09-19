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
        mtime_max: 0,
        ecosystem: None,
        hardlinked: false,
        local_bytes: 0,
        track: None,
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
                ecosystems: Vec::new(),
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
                ecosystems: Vec::new(),
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
        series_by_key: Default::default(),
        total_series: Vec::new(),
        series_window_secs: 0,
        summary: Default::default(),
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
    app.history_secs = Some(30 * 86_400);
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
    assert!(before.contains("view: projects · filter: none"), "{before}");
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
fn checkout_without_a_remote_marks_and_the_confirm_line_warns() {
    use crossterm::event::KeyCode;
    let mut app = App::new(fixture_report(), std::path::PathBuf::from("/Users/dev/src"));
    app.width = 200;
    slop_livin_tui::handle_key(&mut app, KeyCode::Char('0'));
    slop_livin_tui::handle_key(&mut app, KeyCode::Enter); // drill into first project
    assert_eq!(app.view, ViewKind::Tree);
    // The fixture's project has no remote: Backspace still marks it and
    // asks once, with that fact on the confirm line.
    slop_livin_tui::handle_key(&mut app, KeyCode::Backspace);
    let f = capture(&app, 200, 60);
    assert_eq!(app.marked.len(), 1);
    assert!(app.confirm_open);
    assert!(
        f.contains("no remote to restore from"),
        "warning expected:\n{f}"
    );
    assert!(f.contains("Enter yes"), "{f}");
}

#[test]
fn worktree_rows_always_mark_and_carry_their_warnings() {
    use slop_livin_tui::model::{Row, WorktreeMark};
    let mark = |linked: bool, dirty: Option<bool>, unpushed: Option<u32>, locked: Option<bool>| {
        WorktreeMark {
            path: "/x/wt".into(),
            linked,
            remote: Some("github.com/o/r".to_string()),
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
        series: None,
        badges: String::new(),
        ecosystems: Vec::new(),
        mtime_max: 0,
        collapsed_children: None,
        expandable: false,
    };
    let mut app = App::new(fixture_report(), std::path::PathBuf::from("/Users/dev/src"));
    for (m, expect_warning) in [
        (mark(true, Some(false), Some(0), Some(false)), ""),
        (mark(true, Some(true), Some(0), Some(false)), "dirty"),
        (mark(true, Some(false), Some(3), Some(false)), "3 unpushed"),
        (mark(true, Some(false), Some(0), Some(true)), "locked"),
        (
            mark(true, Some(false), None, Some(false)),
            "unpushed unknown",
        ),
        (
            WorktreeMark {
                remote: None,
                ..mark(false, Some(false), Some(0), Some(false))
            },
            "no remote",
        ),
    ] {
        app.marked.clear();
        app.mark_row(&row(m));
        assert_eq!(
            app.marked.len(),
            1,
            "every worktree row marks; the bar is the human"
        );
        let unit = app.marked.values().next().unwrap();
        if expect_warning.is_empty() {
            assert!(unit.warnings.is_empty(), "{:?}", unit.warnings);
        } else {
            assert!(
                unit.warnings.iter().any(|w| w.contains(expect_warning)),
                "expected {expect_warning:?} in {:?}",
                unit.warnings
            );
        }
    }
}

/// The archive bar, end to end against real git: a clean, fully pushed
/// checkout is archived to Trash; the same checkout with one untracked
/// file is refused, because nothing would bring that file back.
#[test]
fn archiving_a_checkout_trashes_it_and_records_the_warnings_shown() {
    use slop_livin_tui::actions::{MarkedUnit, WorktreeTerms, authorize, execute_plan};
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success(),
            "git {args:?}"
        );
    }

    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    let work = tmp.path().join("work");
    let trash = tmp.path().join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&origin)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("git")
            .args(["clone", "-q"])
            .arg(&origin)
            .arg(&work)
            .output()
            .unwrap()
            .status
            .success()
    );
    git(&work, &["config", "user.email", "t@e"]);
    git(&work, &["config", "user.name", "t"]);
    std::fs::write(work.join("README.md"), "hi").unwrap();
    std::fs::write(work.join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir_all(work.join("build")).unwrap();
    std::fs::write(work.join("build/out.bin"), vec![b'x'; 4096]).unwrap();
    git(&work, &["add", "README.md", ".gitignore"]);
    git(&work, &["commit", "-qm", "init"]);
    git(&work, &["push", "-q", "-u", "origin", "HEAD"]);

    let unit = |path: &std::path::Path| MarkedUnit {
        path: path.to_path_buf(),
        docker: None,
        worktree_path: PathBuf::new(),
        bytes: 4096,
        observed_at: slop_livin_core::entities::now(),
        label: String::new(),
        warnings: Vec::new(),
        worktree: Some(WorktreeTerms {
            merge_complete: false,
            pr: None,
            whole_checkout: true,
            remote: Some("github.com/o/r".into()),
        }),
    };
    let ledger = slop_livin_core::ledger::Ledger::open(tmp.path().join("ledger.jsonl")).unwrap();

    // Untracked content present: no longer a bar — the human saw it on
    // the confirm line. The sink moves the checkout and the ledger keeps
    // the warnings that were shown.
    std::fs::write(work.join("secrets.env"), vec![b'k'; 2048]).unwrap();
    let mut u = unit(&work);
    u.warnings = vec!["secrets.env untracked 2.0KB".into()];
    let (plan, grant) = authorize(std::slice::from_ref(&u), "human");
    let res = execute_plan(
        std::slice::from_ref(&u),
        &plan,
        &grant,
        &ledger,
        &trash,
        "human",
        false,
    );
    assert!(res[0].outcome.is_ok(), "{:?}", res[0].outcome);
    assert!(!work.exists(), "checkout moved to Trash");
    let recs = ledger.all().unwrap();
    let last = recs.last().unwrap();
    assert!(matches!(last.verb, slop_livin_core::grants::Verb::Archive));
    assert!(
        last.evidence["recover"]
            .as_str()
            .unwrap()
            .contains("git clone")
    );
    assert_eq!(
        last.evidence["warnings_shown"][0], "secrets.env untracked 2.0KB",
        "the confirm-line facts travel into the ledger"
    );
}
