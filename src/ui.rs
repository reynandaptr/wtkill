//! Rendering only: draws an `App`, never mutates it.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row as TableRow, Table, TableState, Wrap};

use crate::app::{App, Mode, Row, RowStatus, SortKey};
use crate::delete::Action;
use crate::format::format_size;
use crate::list::{age_cell, branch_cell, size_cell};
use crate::model::{Kind, State, Upstream};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn state_cell(row: &Row) -> (String, Color) {
    match (&row.status, row.state()) {
        (RowStatus::Deleting, _) => ("deleting…".into(), Color::DarkGray),
        (RowStatus::Failed(_), _) => ("✖ failed".into(), Color::Red),
        (_, None) => ("…".into(), Color::DarkGray),
        (_, Some(State::Risky)) => {
            let dirty = row.git.as_ref().and_then(|g| g.as_ref().ok()).map_or(0, |g| g.dirty_count);
            let suffix = if dirty > 0 { format!(" {dirty}✎") } else { String::new() };
            (format!("🔴 risky{suffix}"), Color::Red)
        }
        (_, Some(State::Done)) => ("🟢 done".into(), Color::Green),
        (_, Some(State::Pushed)) => ("🟡 pushed".into(), Color::Yellow),
        (_, Some(State::Unknown)) => ("❔ unknown".into(), Color::Magenta),
        (_, Some(s)) => (format!("⚪ {}", s.label()), Color::Gray),
    }
}

pub fn render(frame: &mut Frame, app: &App, now_unix: i64, spinner_tick: usize) {
    let area = frame.area();
    let [header, table, detail, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(8), Constraint::Length(1)])
            .areas(area);
    render_header(frame, app, header);
    render_table(frame, app, table, now_unix, spinner_tick);
    render_detail(frame, app, detail);
    render_footer(frame, app, footer);
    match &app.mode {
        Mode::Confirm { ids, action, strict, input, yes_selected } => {
            render_confirm(frame, app, ids, *action, *strict, input, *yes_selected)
        }
        Mode::ConfirmQuit { yes_selected } => {
            let lines = vec![
                Line::from("Deletions are still running."),
                Line::from("Quit anyway? They finish before wtkill exits."),
                Line::from(""),
                buttons(*yes_selected),
                Line::from("Tab switch · Enter choose · Esc stay").dark_gray(),
            ];
            popup(frame, " Quit? ", lines, Color::Yellow);
        }
        _ => {}
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let total: u64 = app.rows.iter().filter_map(|r| r.size).map(|s| s.bytes).sum();
    let roots: Vec<String> = app.roots.iter().map(|r| r.display().to_string()).collect();
    // Counts first: a long root path must not push them off-screen.
    let mut spans = vec![
        " wtkill ".bold().reversed(),
        Span::raw(format!("  found {} · {} · ", app.rows.len(), format_size(total))),
        Span::styled(format!("freed {}", format_size(app.freed)), Style::new().fg(Color::Green)),
    ];
    if app.scanning {
        spans.push(Span::styled("  [scanning…]", Style::new().fg(Color::DarkGray)));
    }
    if let Some(m) = &app.message {
        spans.push(Span::styled(format!("  {m}"), Style::new().fg(Color::Yellow)));
    }
    spans.push(Span::styled(format!("   {}", roots.join(", ")), Style::new().fg(Color::DarkGray)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_table(frame: &mut Frame, app: &App, area: Rect, now_unix: i64, tick: usize) {
    let visible = app.visible();
    let rows: Vec<TableRow> = visible
        .iter()
        .map(|&i| {
            let r = &app.rows[i];
            let mark = if app.marks.contains(&r.entry.id) { "●" } else { " " };
            let size = if r.size.is_none() { SPINNER[tick % SPINNER.len()].to_string() } else { size_cell(r) };
            let (state, color) = state_cell(r);
            TableRow::new(vec![
                Cell::from(mark),
                Cell::from(Line::from(size).right_aligned()),
                Cell::from(state).style(Style::new().fg(color)),
                Cell::from(r.display_path()),
                Cell::from(branch_cell(r)),
                Cell::from(age_cell(r, now_unix)),
            ])
        })
        .collect();
    let sort = match app.sort {
        SortKey::Size => "SIZE ▼",
        SortKey::Age => "AGE ▲",
        SortKey::State => "STATE",
    };
    let header = TableRow::new(vec!["", "SIZE", "STATE", "WORKTREE", "BRANCH", "AGE"])
        .style(Style::new().add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(1),
        Constraint::Length(10),
        Constraint::Length(13),
        Constraint::Fill(1),
        Constraint::Length(22),
        Constraint::Length(5),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::new().borders(Borders::TOP | Borders::BOTTOM).title(format!(" sort: {sort} ")))
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶");
    let mut state = TableState::default().with_selected((!visible.is_empty()).then_some(app.cursor));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(r) = app.selected() else {
        let msg = if app.scanning { "scanning…" } else { "no .worktree entries found" };
        frame.render_widget(Paragraph::new(msg).dark_gray(), area);
        return;
    };
    let mut lines = vec![Line::from(format!("path  {}", r.entry.path.display()))];
    let repo = r.entry.main_repo.as_ref().map_or("—".to_string(), |m| m.display().to_string());
    lines.push(Line::from(format!("repo  {repo}")));
    match &r.git {
        Some(Ok(g)) => {
            let up = match &g.upstream {
                Upstream::None => "no upstream".to_string(),
                Upstream::Tracking { name, ahead, behind } => format!("{name} (ahead {ahead}, behind {behind})"),
                Upstream::Gone { name } if g.squash_merged => format!("{name} gone — changes merged into default branch"),
                Upstream::Gone { name } => format!("{name} gone"),
            };
            lines.push(Line::from(format!("git   {} · {up}", branch_cell(r))));
            if g.dirty_count > 0 {
                lines.push(Line::from(format!("dirty {} file(s): {}", g.dirty_count, g.dirty_paths.join(", "))).red());
            }
            if g.local_only > 0 {
                let label = if g.squash_merged { "commits (merged)" } else { "local-only commits" };
                lines.push(Line::from(format!("{} {label}: {}", g.local_only, g.local_only_subjects.join(" · "))));
            }
        }
        Some(Err(e)) => lines.push(Line::from(format!("git error: {e}")).red()),
        None => {}
    }
    if let RowStatus::Failed(e) = &r.status {
        lines.push(Line::from(format!("✖ {e}")).red());
    }
    if let Some(n) = &r.note {
        lines.push(Line::from(n.clone()).yellow());
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let text = match app.mode {
        Mode::Filter => format!(" filter: {}▏  Enter apply · Esc clear", app.filter),
        _ => {
            let filter = if app.filter.is_empty() { String::new() } else { format!(" [filter: {}]", app.filter) };
            format!(" ↑↓/jk move · space mark · d delete · D delete+branch · f fetch · s sort · / filter · q quit{filter}")
        }
    };
    frame.render_widget(Paragraph::new(text).dark_gray(), area);
}

/// `[ No ]   [ Yes ]` with the focused button highlighted.
fn buttons(yes_selected: bool) -> Line<'static> {
    let focused = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    let (no, yes) = if yes_selected { (Style::new(), focused) } else { (focused, Style::new()) };
    Line::from(vec![Span::styled("[ No ]", no), Span::raw("   "), Span::styled("[ Yes ]", yes)]).centered()
}

fn render_confirm(
    frame: &mut Frame,
    app: &App,
    ids: &[usize],
    action: Action,
    strict: bool,
    input: &str,
    yes_selected: bool,
) {
    let rows: Vec<&Row> = ids.iter().filter_map(|id| app.row(*id)).collect();
    let bytes: u64 = rows.iter().filter_map(|r| r.size).map(|s| s.bytes).sum();
    let what = if action == Action::RemoveAndBranch { " and their local branches" } else { "" };
    let mut lines = vec![Line::from(format!("Delete {} item(s){what}, freeing {}?", rows.len(), format_size(bytes))), Line::from("")];
    // Each item is one line: "• " + path + suffix, with the path shortened
    // from the left to fit, so the bullet never ends up on its own line.
    let inner = popup_width(frame.area()).saturating_sub(2) as usize;
    for r in rows.iter().take(8) {
        let mut suffix = String::new();
        if let Some(Ok(g)) = &r.git {
            let lost = if g.squash_merged { 0 } else { g.local_only };
            if g.dirty_count > 0 || lost > 0 {
                suffix = format!(" — {} dirty, {lost} local-only commit(s)", g.dirty_count);
            }
        } else if r.entry.kind == Kind::Worktree {
            suffix = " — git status unknown".to_string();
        }
        let room = inner.saturating_sub(2 + suffix.chars().count()).max(12);
        lines.push(Line::from(format!("• {}{suffix}", truncate_left(&r.display_path(), room))));
    }
    if rows.len() > 8 {
        lines.push(Line::from(format!("… and {} more", rows.len() - 8)));
    }
    lines.push(Line::from(""));
    if strict {
        lines.push(Line::from("Uncommitted or unpushed work will be lost.").red().bold());
        lines.push(Line::from(format!("type yes and press Enter: {input}▏")));
        lines.push(Line::from("Esc cancels").dark_gray());
        popup(frame, " ⚠ Delete — may lose work ", lines, Color::Red);
    } else {
        lines.push(buttons(yes_selected));
        lines.push(Line::from("Tab switch · Enter choose · Esc cancel").dark_gray().centered());
        popup(frame, " Delete ", lines, Color::Cyan);
    }
}

const POPUP_MAX_WIDTH: u16 = 100;

fn popup_width(area: Rect) -> u16 {
    POPUP_MAX_WIDTH.min(area.width)
}

/// Shorten `s` to at most `max` chars, replacing its start with `…` so the
/// distinctive tail (the worktree name) stays visible.
fn truncate_left(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let tail: String = s.chars().skip(n - (max - 1)).collect();
    format!("…{tail}")
}

/// Centered, bordered box sized to the *rendered* (wrapped) height of its
/// lines, so the last lines (the buttons) are never clipped.
fn popup(frame: &mut Frame, title: &str, lines: Vec<Line>, color: Color) {
    let area = frame.area();
    let width = popup_width(area);
    let block = Block::bordered().title(title.to_string()).border_style(Style::new().fg(color));
    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    let rendered = paragraph.line_count(width.saturating_sub(2));
    let height = u16::try_from(rendered).unwrap_or(u16::MAX).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::Mode;
    use crate::delete::Action;
    use crate::model::{Entry, GitInfo, Kind, Size, Update};

    fn git(dirty: usize, up: Upstream) -> GitInfo {
        GitInfo {
            branch: Some("feat/x".into()),
            head_short: "abc".into(),
            dirty_count: dirty,
            dirty_paths: vec!["src/a.rs".into()],
            local_only: 0,
            local_only_subjects: vec![],
            upstream: up,
            last_commit_unix: Some(0),
            squash_merged: false,
            head_oid: "abc".into(),
        }
    }

    fn sample() -> App {
        let mut a = App::new(vec![PathBuf::from("/r")]);
        let rows = [
            ("app-done", Kind::Worktree, 1_288_490_189u64, Some(git(0, Upstream::Gone { name: "origin/x".into() }))),
            ("app-dirty", Kind::Worktree, 4096, Some(git(3, Upstream::None))),
            ("notes.md", Kind::Orphan, 4096, None),
        ];
        for (id, (name, kind, bytes, g)) in rows.into_iter().enumerate() {
            a.apply(Update::Found(Entry {
                id,
                path: PathBuf::from(format!("/r/p/.worktree/{name}")),
                root: PathBuf::from("/r"),
                kind,
                main_repo: Some(PathBuf::from("/r/p/app")),
                is_symlink: false,
            }));
            a.apply(Update::Size { id, size: Size { bytes, partial: false } });
            if let Some(g) = g {
                a.apply(Update::Git { id, result: Ok(g) });
            }
        }
        a
    }

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| render(f, app, 86_400 * 3, 0)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_header_rows_and_footer() {
        let text = draw(&sample(), 120, 30);
        for s in ["found 3", "freed 0 B", "done", "risky 3✎", "orphan", "1.2 GB", "d delete", "feat/x"] {
            assert!(text.contains(s), "missing {s:?} in\n{text}");
        }
    }

    #[test]
    fn counts_visible_with_long_root() {
        let mut a = sample();
        a.roots = vec![PathBuf::from(format!("/{}", "very-long-directory-name/".repeat(8)))];
        let text = draw(&a, 80, 20);
        let header = text.lines().next().unwrap();
        assert!(header.contains("found 3") && header.contains("freed 0 B"), "{header}");
    }

    #[test]
    fn strict_confirm_shows_type_yes() {
        let mut a = sample();
        a.mode = Mode::Confirm { ids: vec![1], action: Action::Remove, strict: true, input: "ye".into(), yes_selected: false };
        let text = draw(&a, 120, 30);
        assert!(text.contains("type yes"), "{text}");
        assert!(text.contains("3 dirty"), "{text}");
    }

    #[test]
    fn button_confirm_shows_no_and_yes() {
        let mut a = sample();
        a.mode = Mode::Confirm { ids: vec![0], action: Action::Remove, strict: false, input: String::new(), yes_selected: false };
        let text = draw(&a, 120, 30);
        assert!(text.contains("[ No ]") && text.contains("[ Yes ]"), "{text}");
        assert!(text.contains("Tab"), "{text}");
        assert!(!text.contains("[y] delete"), "{text}");
        a.mode = Mode::ConfirmQuit { yes_selected: true };
        let text = draw(&a, 120, 30);
        assert!(text.contains("[ No ]") && text.contains("[ Yes ]"), "{text}");
    }

    fn with_long_path(name: &str) -> App {
        let mut a = sample();
        a.apply(Update::Found(Entry {
            id: 9,
            path: PathBuf::from(format!("/r/{name}")),
            root: PathBuf::from("/r"),
            kind: Kind::Orphan,
            main_repo: None,
            is_symlink: false,
        }));
        a.apply(Update::Size { id: 9, size: Size { bytes: 365_900_000, partial: false } });
        a
    }

    #[test]
    fn long_path_keeps_buttons_visible_and_bullet_on_its_line() {
        // Screenshot repro: a 71-char path wrapped and pushed the buttons out.
        let mut a = with_long_path("acmecorp/acme-webapp/.worktree/acme-webapp-chore-regional-backend-origin");
        a.mode = Mode::Confirm { ids: vec![9], action: Action::Remove, strict: false, input: String::new(), yes_selected: false };
        for (w, h) in [(100, 30), (80, 24), (60, 20)] {
            let text = draw(&a, w, h);
            assert!(text.contains("[ No ]") && text.contains("[ Yes ]"), "{w}x{h}\n{text}");
            assert!(text.lines().any(|l| l.contains("•") && l.contains("origin")), "bullet split from path at {w}x{h}\n{text}");
        }
    }

    #[test]
    fn long_path_keeps_strict_input_visible() {
        let long = format!("acmecorp/{}/.worktree/x", "very-long-segment-".repeat(8));
        let mut a = with_long_path(&long);
        a.mode = Mode::Confirm { ids: vec![9, 1], action: Action::Remove, strict: true, input: String::new(), yes_selected: false };
        let text = draw(&a, 80, 24);
        assert!(text.contains("type yes"), "{text}");
        assert!(text.contains("Esc cancels"), "{text}");
    }

    #[test]
    fn truncate_left_keeps_tail() {
        assert_eq!(truncate_left("abc", 5), "abc");
        assert_eq!(truncate_left("abcdef", 4), "…def");
        assert_eq!(truncate_left("abcdef", 0), "");
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let mut a = sample();
        draw(&a, 40, 10);
        draw(&a, 10, 3);
        a.mode = Mode::Confirm { ids: vec![0, 1, 2], action: Action::Remove, strict: true, input: String::new(), yes_selected: false };
        draw(&a, 10, 3);
        draw(&a, 1, 1);
    }
}
