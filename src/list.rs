//! Non-interactive output for `--list` and `--json`.

use serde_json::{Value, json};

use crate::app::{App, Row, RowStatus, SortKey};
use crate::format::{format_age, format_size};
use crate::model::Upstream;

/// Rows in size order, regardless of the app's current sort or filter.
fn rows_by_size(app: &App) -> Vec<&Row> {
    let mut sorted = app.clone();
    sorted.sort = SortKey::Size;
    sorted.filter.clear();
    sorted.visible().into_iter().map(|i| &app.rows[i]).collect()
}

pub fn size_cell(row: &Row) -> String {
    match row.size {
        Some(s) if s.partial => format!("≥ {}", format_size(s.bytes)),
        Some(s) => format_size(s.bytes),
        None => "-".to_string(),
    }
}

pub fn branch_cell(row: &Row) -> String {
    match &row.git {
        Some(Ok(g)) => g.branch.clone().unwrap_or_else(|| format!("({})", g.head_short)),
        _ => "—".to_string(),
    }
}

pub fn age_cell(row: &Row, now_unix: i64) -> String {
    match &row.git {
        Some(Ok(g)) => g.last_commit_unix.map_or("—".to_string(), |t| format_age(now_unix - t)),
        _ => "—".to_string(),
    }
}

/// Plain-text table: header plus one line per row, largest first.
pub fn table(app: &App, now_unix: i64) -> String {
    let rows = rows_by_size(app);
    let cells: Vec<[String; 5]> = rows
        .iter()
        .map(|r| {
            [
                size_cell(r),
                r.state().map_or("…", |s| s.label()).to_string(),
                r.display_path(),
                branch_cell(r),
                age_cell(r, now_unix),
            ]
        })
        .collect();
    let header = ["SIZE", "STATE", "PATH", "BRANCH", "AGE"].map(String::from);
    let mut widths = [0usize; 5];
    for row in std::iter::once(&header).chain(&cells) {
        for (w, c) in widths.iter_mut().zip(row) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |r: &[String; 5]| {
        let pad = |s: &str, w: usize| format!("{s}{}", " ".repeat(w - s.chars().count()));
        let size = format!("{}{}", " ".repeat(widths[0] - r[0].chars().count()), r[0]);
        format!("{size}  {}  {}  {}  {}", pad(&r[1], widths[1]), pad(&r[2], widths[2]), pad(&r[3], widths[3]), r[4])
            .trim_end()
            .to_string()
    };
    let mut out = line(&header);
    out.push('\n');
    for c in &cells {
        out.push_str(&line(c));
        out.push('\n');
    }
    out
}

/// JSON array with the spec's keys, largest first.
pub fn json(app: &App) -> Value {
    let rows = rows_by_size(app);
    Value::Array(
        rows.iter()
            .map(|r| {
                let g = r.git.as_ref().and_then(|g| g.as_ref().ok());
                let upstream = g.map(|g| match &g.upstream {
                    Upstream::None => Value::Null,
                    Upstream::Tracking { name, ahead, behind } => json!({"name": name, "ahead": ahead, "behind": behind}),
                    Upstream::Gone { name } => json!({"name": name, "gone": true}),
                });
                let error = match (&r.git, &r.status) {
                    (Some(Err(e)), _) => Some(e.clone()),
                    (_, RowStatus::Failed(e)) => Some(e.clone()),
                    _ => r.note.clone(),
                };
                json!({
                    "path": r.entry.path,
                    "kind": r.entry.kind,
                    "main_repo": r.entry.main_repo,
                    "size_bytes": r.size.map(|s| s.bytes),
                    "size_partial": r.size.is_some_and(|s| s.partial),
                    "state": r.state().map(|s| s.label()),
                    "branch": g.and_then(|g| g.branch.clone()),
                    "dirty_count": g.map(|g| g.dirty_count),
                    "local_only_commits": g.map(|g| g.local_only),
                    "upstream": upstream.unwrap_or(Value::Null),
                    "last_commit_unix": g.and_then(|g| g.last_commit_unix),
                    "error": error,
                })
            })
            .collect(),
    )
}
