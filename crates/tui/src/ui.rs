//! ratatui rendering. Diffstat-ledger world: box-drawing rail, reverse
//! video selection, yellow `✗` for marked rows, no other color. See
//! DESIGN.md.

use crate::app::{App, ViewKind};
use crate::model::{growth_bar, human_bytes, human_signed_bytes, max_abs_growth, truncate_middle};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

fn since_label(app: &App) -> Option<String> {
    crate::filter::growth_window_secs(&app.filter).map(human_duration)
}

fn human_duration(secs: u64) -> String {
    if secs.is_multiple_of(604_800) {
        format!("{}w", secs / 604_800)
    } else if secs.is_multiple_of(86_400) {
        format!("{}d", secs / 86_400)
    } else if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else {
        format!("{}m", (secs / 60).max(1))
    }
}

fn header_line(app: &App) -> String {
    let projects = app.report.projects.len();
    let attributed: u64 = app
        .report
        .projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .flat_map(|w| &w.artifacts)
        .map(|a| a.bytes)
        .sum();
    let docker_unowned = crate::model::docker_unowned_bytes(&app.report);
    let unowned: u64 = app
        .report
        .unowned
        .iter()
        .map(|u| u.bytes)
        .sum::<u64>()
        .saturating_sub(docker_unowned);
    let obs = if let Some((n, d)) = app.observing {
        format!("observing… {}%", if d == 0 { 0 } else { n * 100 / d })
    } else {
        format!("observed {}", app.observed_label)
    };
    let since = since_label(app)
        .map(|s| format!(" · since {s}"))
        .unwrap_or_default();
    // Clauses in priority order; the renderer drops trailing clauses that
    // do not fit the terminal width rather than truncating mid-word.
    let clauses = vec![
        app.root.display().to_string(),
        format!("{obs}{since}"),
        format!("{projects} projects"),
        format!("{} attributed", human_bytes(attributed)),
        format!("{} unowned", human_bytes(unowned)),
        format!("docker {} unowned", human_bytes(docker_unowned)),
    ];
    fit_clauses(&clauses, app.width as usize)
}

/// Joins clauses with " · " while the result fits in `width`; always keeps
/// the first clause.
pub fn fit_clauses(clauses: &[String], width: usize) -> String {
    let mut out = String::new();
    for (i, c) in clauses.iter().enumerate() {
        let candidate = if i == 0 {
            c.clone()
        } else {
            format!("{out} · {c}")
        };
        if i > 0 && width > 0 && candidate.chars().count() > width {
            break;
        }
        out = candidate;
    }
    out
}

fn footer_line() -> &'static str {
    "↑↓ move  →/← expand  Enter open/confirm  ⌫ mark delete  / filter  v view  g growth-sort  s size-sort  ? help  q quit"
}

pub fn draw(frame: &mut Frame, app: &App) {
    let size = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Length(1), // filter line
            Constraint::Min(1),    // body
            Constraint::Length(if app.confirm_open { 1 } else { 0 }),
            Constraint::Length(1), // footer
        ])
        .split(size);

    frame.render_widget(
        Paragraph::new(header_line(app)).style(Style::default().add_modifier(Modifier::DIM)),
        chunks[0],
    );

    draw_filter_line(frame, app, chunks[1]);
    draw_body(frame, app, chunks[2]);

    if app.confirm_open {
        frame.render_widget(Paragraph::new(app.confirm_summary()), chunks[3]);
    }

    let footer_text = if let Some(msg) = app.refusal_active() {
        msg.to_string()
    } else if let Some(r) = &app.last_result {
        r.clone()
    } else {
        footer_line().to_string()
    };
    let footer_style = if app.refusal_active().is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    frame.render_widget(Paragraph::new(footer_text).style(footer_style), chunks[4]);

    if app.help_open {
        draw_help(frame, size);
    }
    if let Some(p) = &app.picker {
        draw_picker(frame, app, p, size);
    }
}

fn draw_picker(frame: &mut Frame, app: &App, p: &crate::picker::Picker, area: Rect) {
    let w = area.width.min(78);
    let h = area.height.min(15);
    let popup = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);
    let composed = p.compose();
    let count = app
        .match_count_for(&composed)
        .map(|n| format!("{n} row{}", if n == 1 { "" } else { "s" }))
        .unwrap_or_else(|| "—".into());
    let mut lines: Vec<Line> = Vec::new();
    for (name, value, selected) in p.lines() {
        let marker = if selected { "▸ " } else { "  " };
        let l = Line::from(format!("{marker}{name:<15}{value}"));
        lines.push(if selected {
            l.style(Style::default().add_modifier(ratatui::style::Modifier::REVERSED))
        } else {
            l
        });
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!("  filter: {composed}    → {count}")));
    lines.push(Line::from("  ↑↓ field · ←→ value · type to narrow project"));
    lines.push(Line::from(
        "  Enter apply · Esc cancel · e edit as text · 0 clear",
    ));
    let block = Block::default().borders(Borders::ALL).title(" filter ");
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_filter_line(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.editing_filter {
        let hint = if app.completions.is_empty() {
            "(Tab complete · Enter apply · Esc cancel)".to_string()
        } else {
            format!("(Tab: {})", app.completions.join("  "))
        };
        format!("filter › {}▏  {hint}", app.filter_text)
    } else {
        let scope = match (app.view, app.selected_project.as_deref()) {
            (crate::app::ViewKind::Projects, _) => "projects".to_string(),
            (crate::app::ViewKind::Tree, Some(p)) => format!("tree of {p}  (Esc back)"),
            (v, Some(p)) => format!("{} of {p}  (Esc back)", v.label()),
            (v, None) => format!("{}  (Esc back)", v.label()),
        };
        format!("view: {scope} · filter: {}", app.filter_text)
    };
    frame.render_widget(Paragraph::new(text), area);
    if let Some(err) = &app.filter_error {
        // Parse errors show inline in red under the line; with only one
        // row budgeted here we overlay on the same line's tail instead
        // of stealing a row from the body, keeping the one-screen rhythm.
        let msg = format!("  parse error: {err}");
        let x = area.x + (app.filter_text.len() as u16) + 9;
        if x < area.x + area.width {
            let sub = Rect {
                x,
                y: area.y,
                width: area.width.saturating_sub(x - area.x),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().fg(Color::Red)),
                sub,
            );
        }
    }
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    if app.filter_has_no_data() {
        frame.render_widget(Paragraph::new("no data yet"), area);
        return;
    }
    let rows = app.rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("no rows match — Backspace to widen, 0 to clear"),
            area,
        );
        return;
    }
    let max_abs = max_abs_growth(rows.iter());
    // Columns scale with the terminal: fixed bytes (10) + growth (10) +
    // bar; the rest is split between the name and the signals so a wide
    // terminal shows whole paths and spelled-out signals instead of the
    // 80-column layout centered in empty space.
    let width = area.width as usize;
    let narrow = width < 120;
    let bar_width: usize = ((width.saturating_sub(90)) / 5).clamp(8, 32);
    let fixed = 10 + 1 + 10 + 1 + bar_width + 2 + 1;
    let flexible = width.saturating_sub(fixed).max(40);
    let signals_width: usize = if narrow {
        flexible / 4
    } else {
        (flexible * 2 / 5).min(70)
    };
    let name_width: usize = flexible.saturating_sub(signals_width + 1).max(30);
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let marked = row
            .unit
            .as_ref()
            .is_some_and(|u| app.marked.contains_key(&u.0));
        let mark_prefix = if marked { "✗ " } else { "" };
        let track = match row.track {
            Some(t) if !t.label().is_empty() => format!("  [{}]", t.label()),
            _ => String::new(),
        };
        let raw_name = format!("{}{mark_prefix}{}{track}", row.rail, row.label);
        let name = truncate_middle(&raw_name, name_width);
        let bytes = format!("{:>10}", human_bytes(row.bytes));
        let growth = format!(
            "{:>10}",
            row.growth
                .map(human_signed_bytes)
                .unwrap_or_else(|| "—".into())
        );
        let bar = growth_bar(row.growth, max_abs, bar_width);
        let bar_color = match row.growth {
            Some(g) if g > 0 => Color::Green,
            Some(g) if g < 0 => Color::Red,
            _ => Color::DarkGray,
        };
        let hidden = row
            .collapsed_children
            .map(|n| format!("  ({n} hidden)"))
            .unwrap_or_default();
        // DESIGN.md: "80x24 ... signals drop to a single glyph column;
        // uses width up to 200 ... signals spell out."
        // Narrow terminals show the two most decision-relevant signals
        // spelled out (never a glyph code); wide ones show them all.
        let mut signals_text = if row.signals.is_empty() {
            String::new()
        } else if narrow {
            pick_signals(&row.signals, 2).join(" · ")
        } else {
            row.signals.join(" · ")
        };
        if signals_text.chars().count() > signals_width {
            // Never overflow the row: prefer the loud signals, then cut.
            signals_text = pick_signals(&row.signals, 3).join(" · ");
            if signals_text.chars().count() > signals_width {
                signals_text = signals_text
                    .chars()
                    .take(signals_width.saturating_sub(1))
                    .collect::<String>()
                    + "…";
            }
        }

        let name_style = if marked {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let spans = vec![
            Span::styled(format!("{name:<width$}", width = name_width), name_style),
            Span::raw(bytes),
            Span::raw(" "),
            Span::raw(growth),
            Span::raw(" ▕"),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::raw("▏"),
            Span::styled(
                format!(" {signals_text}"),
                Style::default().add_modifier(Modifier::DIM),
            ),
            Span::styled(hidden, Style::default().add_modifier(Modifier::DIM)),
        ];

        let mut line = Line::from(spans);
        if i == app.selected {
            line = line.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let w = area.width.min(70);
    let h = area.height.min(15);
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let popup = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::from("Keys"),
        Line::from("  ↑↓        move selection"),
        Line::from("  →/←       expand / collapse"),
        Line::from("  Enter     open project / confirm delete"),
        Line::from("  Backspace mark selected unit for delete"),
        Line::from("  /         filter picker (form) · : edit filter as text, Tab completes"),
        Line::from("  0         clear filter"),
        Line::from("  v, 1-5    switch view"),
        Line::from("  g / s     sort by growth / size"),
        Line::from("  ?         toggle this help"),
        Line::from("  q         quit"),
        Line::from(""),
        Line::from("Filter grammar"),
        Line::from("  growth [><] <size> in <duration>"),
        Line::from("  kind:<k>   project:<name>"),
        Line::from("  idle > <duration>   merge-complete"),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title("help (? to close)");
    frame.render_widget(Paragraph::new(text).block(block), popup);
}

#[allow(dead_code)]
pub fn view_index(v: ViewKind) -> usize {
    match v {
        ViewKind::Projects => 1,
        ViewKind::Tree => 2,
        ViewKind::Builds => 3,
        ViewKind::Deps => 4,
        ViewKind::Docker => 5,
        ViewKind::Kinds => 6,
        ViewKind::Unowned => 7,
    }
}

/// Chooses up to `n` signals worth a narrow column: anything that is not
/// the quiet default (`clean`, `0 unpushed`, `unlocked`, `unknown`, `no PR`)
/// first, then the last-commit age.
pub fn pick_signals(signals: &[String], n: usize) -> Vec<String> {
    let quiet = |s: &str| {
        matches!(s, "clean" | "0 unpushed" | "unlocked" | "unknown" | "no PR")
            || s.starts_with("unknown (")
    };
    let mut out: Vec<String> = signals.iter().filter(|s| !quiet(s)).cloned().collect();
    if out.is_empty() {
        out.extend(signals.iter().take(1).cloned());
    }
    out.truncate(n);
    out
}
