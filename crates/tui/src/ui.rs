//! ratatui rendering. Diffstat-ledger world: box-drawing rail, reverse
//! video selection, yellow `✗` for marked rows, no other color. See
//! DESIGN.md.

use crate::app::{App, ViewKind};
use crate::model::{
    growth_bar, human_bytes, human_signed_bytes, is_flat, max_abs_growth, net_change, pad_display,
    spark_points, trend, truncate_middle,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Sparkline},
};

/// Width of every history sparkline, header and rows alike.
const SPARK_WIDTH: u16 = 12;

/// Draws a byte series with ratatui's `Sparkline`, scaled from zero to
/// the row's own max so a step reads as the size of the step. Buckets
/// before the first observation render as a dim `·`; a row observed
/// gone renders as empty (it is zero). Colour carries the trend.
fn draw_spark(frame: &mut Frame, series: &[Option<u64>], area: Rect, reversed: bool) {
    let pts = spark_points(series, area.width as usize);
    let color = match trend(series) {
        1 => Color::Green,
        -1 => Color::Red,
        _ => Color::DarkGray,
    };
    let mut style = Style::default().fg(color);
    if reversed {
        style = style.add_modifier(Modifier::REVERSED);
    }
    let max = pts.iter().filter_map(|v| *v).max().unwrap_or(0);
    frame.render_widget(
        Sparkline::default()
            .data(pts)
            .max(max.max(1))
            .style(style)
            .absent_value_symbol("·")
            .absent_value_style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

/// The growth window the header reports. When the asked-for window is
/// longer than the observations the store holds, the effective window is
/// the history itself, and the label says so instead of implying a week
/// of growth from four hours of data.
fn since_label(app: &App) -> Option<String> {
    let asked = crate::filter::growth_window_secs(&app.filter)?;
    Some(match app.history_secs {
        Some(hist) if hist < asked => format!(
            "{} (asked {}; history is {})",
            human_duration(hist),
            human_duration(asked),
            human_duration(hist)
        ),
        _ => human_duration(asked),
    })
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

fn header_line(app: &App, width: usize) -> String {
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
    let obs = if app.observing.is_some() {
        // Live counters from the walk thread; percent against the last
        // observation's walked total (an incremental walk stops early, so
        // the percent is a floor, never a promise).
        let (bytes, dirs, _) = slop_livin_core::walk::progress::snapshot();
        let total = app.report.reconciliation.walked_total;
        let pct = if total > 0 {
            format!(" · {}%", (bytes * 100 / total).min(99))
        } else {
            String::new()
        };
        format!("observing… {} · {dirs} dirs{pct}", human_bytes(bytes))
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
    fit_clauses(&clauses, width)
}

/// Joins clauses with " · " while the result fits in `width`; always keeps
/// the first clause.
pub fn fit_clauses(clauses: &[String], width: usize) -> String {
    let mut out = String::new();
    for (i, c) in clauses.iter().filter(|c| !c.is_empty()).enumerate() {
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
    "↑↓ move  →/← expand  Enter open/confirm  Space mark  ⌫ delete  / filter  v view  g/s/n/t/a sort  r reverse  ? help  q quit"
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

    draw_header(frame, app, chunks[0]);

    draw_filter_line(frame, app, chunks[1]);
    draw_body(frame, app, chunks[2]);

    if app.confirm_open {
        frame.render_widget(
            Paragraph::new(app.confirm_summary()).style(Style::default().fg(Color::Yellow)),
            chunks[3],
        );
    }

    // The footer is the key legend for the state you are actually in.
    let footer_text = if let Some(msg) = app.refusal_active() {
        msg.to_string()
    } else if app.confirm_open {
        "Enter yes · Esc no".to_string()
    } else if app.picker.is_some() {
        "↑↓ field · ←→ value · Space grew/shrank · type to narrow project · Enter apply · Esc cancel · e edit as text · 0 clear".to_string()
    } else if app.editing_filter {
        "Tab complete · Enter apply · Esc cancel".to_string()
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
    let h = area.height.min(18);
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

/// Header: facts on the left, the whole root's history on the right as a
/// sparkline with its net change over the window (first observed to last).
fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let total = &app.report.total_series;
    let net = net_change(total);
    let right_width: u16 = match net {
        Some(d) if area.width >= 80 => SPARK_WIDTH + 1 + human_signed_bytes(d).len() as u16,
        _ => 0,
    };
    let left = Rect {
        width: area.width.saturating_sub(right_width + 2),
        ..area
    };
    frame.render_widget(
        Paragraph::new(header_line(app, left.width as usize))
            .style(Style::default().add_modifier(Modifier::DIM)),
        left,
    );
    if let Some(d) = net.filter(|_| right_width > 0) {
        let x = area.x + area.width - right_width;
        draw_spark(
            frame,
            total,
            Rect {
                x,
                y: area.y,
                width: SPARK_WIDTH,
                height: 1,
            },
            false,
        );
        frame.render_widget(
            Paragraph::new(human_signed_bytes(d))
                .style(Style::default().add_modifier(Modifier::DIM)),
            Rect {
                x: x + SPARK_WIDTH + 1,
                y: area.y,
                width: right_width - SPARK_WIDTH - 1,
                height: 1,
            },
        );
    }
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
        let sort_name = match app.sort {
            crate::model::Sort::Growth => Some("growth"),
            crate::model::Sort::Size => Some("size"),
            crate::model::Sort::Name => Some("name"),
            crate::model::Sort::Type => Some("type"),
            crate::model::Sort::Age => Some("age"),
            crate::model::Sort::None => None,
        };
        let sort = match sort_name {
            Some(n) if app.reverse => format!(" · sort: {n} ↑"),
            Some(n) => format!(" · sort: {n}"),
            None => String::new(),
        };
        let filter = if app.filter_text == "0" {
            "none".to_string()
        } else {
            app.filter_text.clone()
        };
        format!("view: {scope} · filter: {filter}{sort}")
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
            Paragraph::new("no rows match — / to change the filter, 0 to clear"),
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
    let spark_width: usize = if width >= 120 {
        SPARK_WIDTH as usize
    } else {
        0
    };
    let fixed =
        10 + 1 + 10 + 1 + bar_width + 2 + 1 + if spark_width > 0 { spark_width + 1 } else { 0 };
    let flexible = width.saturating_sub(fixed).max(40);
    let signals_width: usize = if narrow {
        flexible / 4
    } else {
        (flexible * 2 / 5).min(70)
    };
    let name_width: usize = flexible.saturating_sub(signals_width + 1).max(30);
    // Sparklines are widgets, drawn over the text after the paragraph:
    // (row index, series) for every row that has a non-flat history.
    let spark_x = area.x + (name_width + 10 + 1 + 10 + 2 + bar_width + 1 + 1) as u16;
    let mut sparks: Vec<(usize, &Vec<Option<u64>>)> = Vec::new();
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
        // Badges trail the name so names stay left-aligned and scannable.
        let badge = if row.badges.is_empty() {
            String::new()
        } else {
            format!("  {}", row.badges)
        };
        let raw_name = format!("{}{mark_prefix}{}{badge}{track}", row.rail, row.label);
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
            .map(|n| format!("  ▸ {n} more"))
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
            Span::styled(pad_display(&name, name_width), name_style),
            Span::raw(bytes),
            Span::raw(" "),
            Span::raw(growth),
            Span::raw(" ▕"),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::raw("▏"),
            {
                // History column: blank in the text; a flat history stays
                // blank (nothing happened), anything else gets a Sparkline
                // widget drawn over this slot below.
                if let Some(s) = row
                    .series
                    .as_ref()
                    .filter(|s| spark_width > 0 && !is_flat(s))
                {
                    sparks.push((i, s));
                }
                Span::raw(" ".repeat(if spark_width > 0 { spark_width + 1 } else { 0 }))
            },
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
    for (i, series) in sparks {
        if i as u16 >= area.height {
            break;
        }
        draw_spark(
            frame,
            series,
            Rect {
                x: spark_x,
                y: area.y + i as u16,
                width: spark_width as u16,
                height: 1,
            },
            i == app.selected,
        );
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let w = area.width.min(90);
    let h = area.height.min(30);
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
        Line::from(
            "  Space     mark / unmark the row
  Backspace delete what is under the cursor (or the marks), asks once",
        ),
        Line::from("  /         filter picker (form) · : edit filter as text, Tab completes"),
        Line::from("  0         clear filter"),
        Line::from(
            "  v, 1-8    switch view (projects · tree · builds · deps · docker · kinds · unowned · types)",
        ),
        Line::from(
            "  g/s/n/t/a sort by growth / size / name / type / age · r reverses (remembered)",
        ),
        Line::from(
            "  k         keep executables: copy target/{release,debug} binaries, dist/*.whl to bin/ before trashing",
        ),
        Line::from("  ?         toggle this help"),
        Line::from("  q         quit"),
        Line::from(""),
        Line::from("Columns: bytes · growth in window · growth bar · history sparkline · facts"),
        Line::from("  [tracked] [ignored] [untracked]: git status; untracked has no copy anywhere"),
        Line::from(""),
        Line::from("Filter grammar"),
        Line::from("  growth [><] <size> in <duration>   (window capped at stored history)"),
        Line::from(
            "  kind:<k>   project:<name|glob*>   type:rs|js|py|go|…   pr:open|merged|closed|none",
        ),
        Line::from("  idle > <duration>   merge-complete   size > <bytes>   age > <duration>"),
        Line::from(""),
        Line::from(
            "Badges  🦀 rs  ⬢ js  🦕 deno  🐍 py  🐹 go  ☕ java  🔺 scala  🔧 cpp  🐦 swift  🟣 net",
        ),
        Line::from(
            "        💎 rb  💧 ex  🐘 php  λ hs  🎯 dart  ⚡ zig  🌍 tf  🐳 docker  🎲 unity  🎮 ue",
        ),
        Line::from("        🔨 has build output   ⎇N  N linked worktrees"),
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
        ViewKind::Types => 8,
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
