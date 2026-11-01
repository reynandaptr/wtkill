//! Application state: a pure state machine driven by `Update`s and keys.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::delete::Action;
use crate::model::{Entry, GitInfo, Kind, Size, State, Update, compute_state};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowStatus {
    Idle,
    Deleting,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub entry: Entry,
    pub size: Option<Size>,
    pub git: Option<Result<GitInfo, String>>,
    pub status: RowStatus,
    pub note: Option<String>,
}

impl Row {
    pub fn state(&self) -> Option<State> {
        compute_state(&self.entry.kind, self.git.as_ref())
    }

    /// Path relative to the root it was found under.
    pub fn display_path(&self) -> String {
        let p = &self.entry.path;
        p.strip_prefix(&self.entry.root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    }

    fn git_ok(&self) -> Option<&GitInfo> {
        self.git.as_ref().and_then(|g| g.as_ref().ok())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Size,
    Age,
    State,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filter,
    /// `yes_selected`: which button has focus in the non-strict prompt.
    Confirm {
        ids: Vec<usize>,
        action: Action,
        strict: bool,
        input: String,
        yes_selected: bool,
    },
    ConfirmQuit {
        yes_selected: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `strict`: the user answered the strict (`yes`) prompt, so `--force` is allowed.
    Delete {
        ids: Vec<usize>,
        action: Action,
        strict: bool,
    },
    Fetch,
    Quit,
}

#[derive(Debug, Clone)]
pub struct App {
    pub rows: Vec<Row>,
    /// Index into `visible()`.
    pub cursor: usize,
    /// Marked entry ids.
    pub marks: BTreeSet<usize>,
    pub sort: SortKey,
    pub filter: String,
    pub mode: Mode,
    pub freed: u64,
    pub scanning: bool,
    pub roots: Vec<PathBuf>,
    /// Last delete note or error, shown in the header.
    pub message: Option<String>,
    /// Entry id under the cursor; the cursor follows it when the list re-sorts.
    pub selected_id: Option<usize>,
}

/// What a key does in a No/Yes button prompt. There is deliberately no
/// one-key "yes": the user must move focus to Yes, then press Enter.
enum ButtonKey {
    Toggle,
    Activate,
    Cancel,
    Ignore,
}

fn button_key(key: KeyEvent) -> ButtonKey {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return ButtonKey::Cancel;
    }
    match key.code {
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => ButtonKey::Toggle,
        KeyCode::Enter => ButtonKey::Activate,
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => ButtonKey::Cancel,
        _ => ButtonKey::Ignore,
    }
}

const CLONE_NOTE: &str = "full git clone — delete manually";

fn state_rank(s: Option<State>) -> u8 {
    match s {
        Some(State::Done) => 0,
        Some(State::Pushed) => 1,
        Some(State::Orphan) => 2,
        Some(State::Broken) => 3,
        Some(State::Unknown) => 4,
        Some(State::Risky) => 5,
        Some(State::Clone) => 6,
        None => 7,
    }
}

impl App {
    pub fn new(roots: Vec<PathBuf>) -> App {
        App {
            rows: Vec::new(),
            cursor: 0,
            marks: BTreeSet::new(),
            sort: SortKey::Size,
            filter: String::new(),
            mode: Mode::Normal,
            freed: 0,
            scanning: true,
            roots,
            message: None,
            selected_id: None,
        }
    }

    fn index_of(&self, id: usize) -> Option<usize> {
        self.rows.iter().position(|r| r.entry.id == id)
    }

    pub fn row(&self, id: usize) -> Option<&Row> {
        self.index_of(id).map(|i| &self.rows[i])
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Found(entry) => self.rows.push(Row {
                entry,
                size: None,
                git: None,
                status: RowStatus::Idle,
                note: None,
            }),
            Update::Size { id, size } => {
                if let Some(i) = self.index_of(id) {
                    self.rows[i].size = Some(size);
                }
            }
            Update::Git { id, result } => {
                if let Some(i) = self.index_of(id) {
                    self.rows[i].git = Some(result);
                }
            }
            Update::FetchFailed { main_repo, error } => {
                for r in self
                    .rows
                    .iter_mut()
                    .filter(|r| r.entry.main_repo.as_ref() == Some(&main_repo))
                {
                    r.note = Some(format!("fetch failed: {error}"));
                }
            }
            Update::ScanDone => self.scanning = false,
            Update::Deleted { id, note } => {
                if let Some(i) = self.index_of(id) {
                    let row = self.rows.remove(i);
                    self.freed += row.size.map_or(0, |s| s.bytes);
                }
                self.marks.remove(&id);
                if note.is_some() {
                    self.message = note;
                }
            }
            Update::DeleteFailed { id, error } => {
                if let Some(i) = self.index_of(id) {
                    self.rows[i].status = RowStatus::Failed(error.clone());
                }
                self.message = Some(error);
            }
        }
        self.clamp();
    }

    /// Keep the cursor on the selected entry if it's still visible; otherwise
    /// clamp the index and select whatever is under it.
    fn clamp(&mut self) {
        let visible = self.visible();
        if let Some(id) = self.selected_id
            && let Some(pos) = visible.iter().position(|&i| self.rows[i].entry.id == id)
        {
            self.cursor = pos;
            return;
        }
        self.cursor = self.cursor.min(visible.len().saturating_sub(1));
        self.selected_id = visible.get(self.cursor).map(|&i| self.rows[i].entry.id);
    }

    fn move_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.selected_id = None;
        self.clamp();
    }

    fn compare(&self, a: &Row, b: &Row) -> Ordering {
        let primary = match self.sort {
            SortKey::Size => match (a.size, b.size) {
                (Some(x), Some(y)) => y.bytes.cmp(&x.bytes),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            SortKey::Age => {
                let t = |r: &Row| r.git_ok().and_then(|g| g.last_commit_unix);
                match (t(a), t(b)) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
            }
            SortKey::State => state_rank(a.state()).cmp(&state_rank(b.state())),
        };
        primary.then_with(|| a.display_path().cmp(&b.display_path()))
    }

    /// Indices into `rows`, filtered and sorted.
    pub fn visible(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        let mut idx: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                needle.is_empty() || self.rows[i].display_path().to_lowercase().contains(&needle)
            })
            .collect();
        idx.sort_by(|&a, &b| self.compare(&self.rows[a], &self.rows[b]));
        idx
    }

    pub fn selected(&self) -> Option<&Row> {
        self.visible().get(self.cursor).map(|&i| &self.rows[i])
    }

    pub fn deleting(&self) -> bool {
        self.rows.iter().any(|r| r.status == RowStatus::Deleting)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Command> {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => {
                if ctrl_c {
                    return self.quit();
                }
                self.normal_key(key)
            }
            Mode::Filter => {
                match key.code {
                    KeyCode::Enter => {}
                    KeyCode::Esc => self.filter.clear(),
                    KeyCode::Backspace => {
                        self.filter.pop();
                        self.mode = Mode::Filter;
                    }
                    KeyCode::Char(c) if !ctrl_c => {
                        self.filter.push(c);
                        self.mode = Mode::Filter;
                    }
                    _ => {}
                }
                self.clamp();
                vec![]
            }
            Mode::Confirm {
                ids,
                action,
                strict,
                mut input,
                yes_selected,
            } => {
                let confirmed = if strict {
                    match key.code {
                        KeyCode::Enter if input == "yes" => true,
                        KeyCode::Esc => false,
                        KeyCode::Char(_) if ctrl_c => false,
                        KeyCode::Char(c) => {
                            input.push(c);
                            self.mode = Mode::Confirm {
                                ids,
                                action,
                                strict,
                                input,
                                yes_selected,
                            };
                            return vec![];
                        }
                        _ => {
                            if key.code == KeyCode::Backspace {
                                input.pop();
                            }
                            self.mode = Mode::Confirm {
                                ids,
                                action,
                                strict,
                                input,
                                yes_selected,
                            };
                            return vec![];
                        }
                    }
                } else {
                    match button_key(key) {
                        ButtonKey::Toggle => {
                            let yes_selected = !yes_selected;
                            self.mode = Mode::Confirm {
                                ids,
                                action,
                                strict,
                                input,
                                yes_selected,
                            };
                            return vec![];
                        }
                        ButtonKey::Activate => yes_selected,
                        ButtonKey::Cancel => false,
                        ButtonKey::Ignore => {
                            self.mode = Mode::Confirm {
                                ids,
                                action,
                                strict,
                                input,
                                yes_selected,
                            };
                            return vec![];
                        }
                    }
                };
                if !confirmed {
                    return vec![];
                }
                for id in &ids {
                    if let Some(i) = self.index_of(*id) {
                        self.rows[i].status = RowStatus::Deleting;
                    }
                }
                self.marks.clear();
                vec![Command::Delete {
                    ids,
                    action,
                    strict,
                }]
            }
            Mode::ConfirmQuit { yes_selected } => match button_key(key) {
                ButtonKey::Toggle => {
                    self.mode = Mode::ConfirmQuit {
                        yes_selected: !yes_selected,
                    };
                    vec![]
                }
                ButtonKey::Activate if yes_selected => vec![Command::Quit],
                ButtonKey::Activate | ButtonKey::Cancel => vec![],
                ButtonKey::Ignore => {
                    self.mode = Mode::ConfirmQuit { yes_selected };
                    vec![]
                }
            },
        }
    }

    fn quit(&mut self) -> Vec<Command> {
        if self.deleting() {
            self.mode = Mode::ConfirmQuit {
                yes_selected: false,
            };
            vec![]
        } else {
            vec![Command::Quit]
        }
    }

    fn normal_key(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(self.cursor.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(self.cursor + 1),
            KeyCode::Char(' ') => {
                if let Some(id) = self.selected().map(|r| r.entry.id)
                    && !self.marks.remove(&id)
                {
                    self.marks.insert(id);
                }
            }
            KeyCode::Char('d') => self.start_delete(Action::Remove),
            KeyCode::Char('D') => self.start_delete(Action::RemoveAndBranch),
            KeyCode::Char('f') => {
                // Show states as pending until the post-fetch inspection lands,
                // so nobody deletes based on pre-fetch data.
                for r in self
                    .rows
                    .iter_mut()
                    .filter(|r| r.entry.kind == Kind::Worktree && r.status != RowStatus::Deleting)
                {
                    r.git = None;
                }
                return vec![Command::Fetch];
            }
            KeyCode::Char('s') => {
                self.sort = match self.sort {
                    SortKey::Size => SortKey::Age,
                    SortKey::Age => SortKey::State,
                    SortKey::State => SortKey::Size,
                }
            }
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Esc => {
                self.filter.clear();
                self.clamp();
            }
            KeyCode::Char('q') => return self.quit(),
            _ => {}
        }
        vec![]
    }

    fn start_delete(&mut self, action: Action) {
        let mut targets: Vec<usize> = if self.marks.is_empty() {
            self.selected().map(|r| r.entry.id).into_iter().collect()
        } else {
            self.marks.iter().copied().collect()
        };
        targets.retain(|id| {
            self.row(*id)
                .is_some_and(|r| r.status != RowStatus::Deleting)
        });
        let (clones, ids): (Vec<usize>, Vec<usize>) = targets
            .into_iter()
            .partition(|id| self.row(*id).is_some_and(|r| r.entry.kind == Kind::Clone));
        if ids.is_empty() {
            for id in clones {
                if let Some(i) = self.index_of(id) {
                    self.rows[i].note = Some(CLONE_NOTE.to_string());
                }
            }
            return;
        }
        // Pending git status counts as strict: we can't yet tell it's safe.
        let strict = ids.iter().any(|id| {
            self.row(*id)
                .and_then(Row::state)
                .is_none_or(|s| s.is_strict())
        });
        self.mode = Mode::Confirm {
            ids,
            action,
            strict,
            input: String::new(),
            yes_selected: false,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Upstream;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn git(dirty: usize, up: Upstream) -> GitInfo {
        GitInfo {
            branch: Some("b".into()),
            head_short: "abc".into(),
            dirty_count: dirty,
            dirty_paths: vec![],
            local_only: 0,
            local_only_subjects: vec![],
            upstream: up,
            last_commit_unix: Some(0),
            squash_merged: false,
            head_oid: "abc".into(),
        }
    }
    fn done() -> Option<GitInfo> {
        Some(git(
            0,
            Upstream::Gone {
                name: "origin/b".into(),
            },
        ))
    }
    fn risky() -> Option<GitInfo> {
        Some(git(3, Upstream::None))
    }

    /// Build an app; each row is (name, kind, size, git).
    fn app(rows: Vec<(&str, Kind, Option<u64>, Option<GitInfo>)>) -> App {
        let mut a = App::new(vec![PathBuf::from("/r")]);
        for (id, (name, kind, size, g)) in rows.into_iter().enumerate() {
            let main_repo = Some(PathBuf::from("/r/p/main"));
            a.apply(Update::Found(Entry {
                id,
                path: PathBuf::from(format!("/r/p/.worktree/{name}")),
                root: PathBuf::from("/r"),
                kind,
                main_repo,
                is_symlink: false,
            }));
            if let Some(bytes) = size {
                a.apply(Update::Size {
                    id,
                    size: Size {
                        bytes,
                        partial: false,
                    },
                });
            }
            if let Some(g) = g {
                a.apply(Update::Git { id, result: Ok(g) });
            }
        }
        a
    }

    fn names(a: &App) -> Vec<String> {
        a.visible()
            .iter()
            .map(|&i| {
                a.rows[i]
                    .entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn sort_by_size_desc_pending_last() {
        let a = app(vec![
            ("a", Kind::Orphan, Some(10), None),
            ("b", Kind::Orphan, None, None),
            ("c", Kind::Orphan, Some(30), None),
        ]);
        assert_eq!(names(&a), ["c", "a", "b"]);
    }

    #[test]
    fn sort_cycles_to_state() {
        let mut a = app(vec![
            ("r", Kind::Worktree, Some(30), risky()),
            ("d", Kind::Worktree, Some(10), done()),
        ]);
        a.handle_key(key('s')); // age
        a.handle_key(key('s')); // state
        assert_eq!(a.sort, SortKey::State);
        assert_eq!(names(&a), ["d", "r"]);
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut a = app(vec![
            ("App-One", Kind::Orphan, Some(1), None),
            ("other", Kind::Orphan, Some(2), None),
        ]);
        for c in ['/', 'a', 'p', 'P'] {
            a.handle_key(key(c));
        }
        a.handle_key(code(KeyCode::Enter));
        assert_eq!(a.mode, Mode::Normal);
        assert_eq!(names(&a), ["App-One"]);
        a.handle_key(key('/'));
        a.handle_key(code(KeyCode::Esc));
        assert_eq!(a.filter, "");
        assert_eq!(a.visible().len(), 2);
    }

    #[test]
    fn d_opens_button_prompt_with_no_selected_and_enter_cancels() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
        assert!(a.handle_key(key('d')).is_empty());
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                strict: false,
                yes_selected: false,
                ..
            }
        ));
        assert!(a.handle_key(code(KeyCode::Enter)).is_empty());
        assert_eq!(a.mode, Mode::Normal);
        assert_eq!(a.rows[0].status, RowStatus::Idle);
    }

    #[test]
    fn tab_to_yes_then_enter_deletes() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
        a.handle_key(key('d'));
        assert!(a.handle_key(code(KeyCode::Tab)).is_empty());
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                yes_selected: true,
                ..
            }
        ));
        assert_eq!(
            a.handle_key(code(KeyCode::Enter)),
            vec![Command::Delete {
                ids: vec![0],
                action: Action::Remove,
                strict: false
            }]
        );
        assert_eq!(a.rows[0].status, RowStatus::Deleting);
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn y_key_does_not_delete() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
        a.handle_key(key('d'));
        assert!(a.handle_key(key('y')).is_empty());
        assert!(a.handle_key(key('Y')).is_empty());
        assert_eq!(a.rows[0].status, RowStatus::Idle);
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                yes_selected: false,
                ..
            }
        ));
    }

    #[test]
    fn arrows_and_backtab_move_selection() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
        a.handle_key(key('d'));
        a.handle_key(code(KeyCode::Right));
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                yes_selected: true,
                ..
            }
        ));
        a.handle_key(code(KeyCode::Left));
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                yes_selected: false,
                ..
            }
        ));
        a.handle_key(code(KeyCode::BackTab));
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                yes_selected: true,
                ..
            }
        ));
        a.handle_key(code(KeyCode::Tab));
        assert!(matches!(
            a.mode,
            Mode::Confirm {
                yes_selected: false,
                ..
            }
        ));
    }

    #[test]
    fn n_and_esc_cancel_prompt() {
        for cancel in [key('n'), code(KeyCode::Esc)] {
            let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
            a.handle_key(key('d'));
            a.handle_key(code(KeyCode::Tab));
            assert!(a.handle_key(cancel).is_empty());
            assert_eq!(a.mode, Mode::Normal);
            assert_eq!(a.rows[0].status, RowStatus::Idle);
        }
    }

    #[test]
    fn shift_d_emits_remove_and_branch() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
        a.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        a.handle_key(code(KeyCode::Tab));
        assert_eq!(
            a.handle_key(code(KeyCode::Enter)),
            vec![Command::Delete {
                ids: vec![0],
                action: Action::RemoveAndBranch,
                strict: false
            }]
        );
    }

    #[test]
    fn risky_requires_typing_yes() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), risky())]);
        a.handle_key(key('d'));
        assert!(matches!(a.mode, Mode::Confirm { strict: true, .. }));
        a.handle_key(key('y'));
        assert!(a.handle_key(code(KeyCode::Enter)).is_empty());
        assert!(matches!(a.mode, Mode::Confirm { .. }));
        a.handle_key(code(KeyCode::Backspace));
        for c in ['y', 'e', 's'] {
            a.handle_key(key(c));
        }
        assert_eq!(
            a.handle_key(code(KeyCode::Enter)),
            vec![Command::Delete {
                ids: vec![0],
                action: Action::Remove,
                strict: true
            }]
        );
    }

    #[test]
    fn pending_worktree_is_strict() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), None)]);
        a.handle_key(key('d'));
        assert!(matches!(a.mode, Mode::Confirm { strict: true, .. }));
    }

    #[test]
    fn marks_apply_to_all_and_strict_if_any_risky() {
        let mut a = app(vec![
            ("d", Kind::Worktree, Some(20), done()),
            ("r", Kind::Worktree, Some(10), risky()),
        ]);
        a.handle_key(key(' '));
        a.handle_key(key('j'));
        a.handle_key(key(' '));
        assert_eq!(a.marks.len(), 2);
        a.handle_key(key('d'));
        match &a.mode {
            Mode::Confirm { ids, strict, .. } => {
                assert_eq!(ids, &vec![0, 1]);
                assert!(strict);
            }
            m => panic!("{m:?}"),
        }
    }

    #[test]
    fn clone_target_opens_no_prompt() {
        let mut a = app(vec![("c", Kind::Clone, Some(1), None)]);
        assert!(a.handle_key(key('d')).is_empty());
        assert_eq!(a.mode, Mode::Normal);
        assert!(
            a.rows[0]
                .note
                .as_deref()
                .unwrap()
                .contains("delete manually")
        );
    }

    #[test]
    fn clone_dropped_from_mixed_marks() {
        let mut a = app(vec![
            ("c", Kind::Clone, Some(20), None),
            ("o", Kind::Orphan, Some(10), None),
        ]);
        a.handle_key(key(' '));
        a.handle_key(key('j'));
        a.handle_key(key(' '));
        a.handle_key(key('d'));
        assert!(matches!(&a.mode, Mode::Confirm { ids, .. } if ids == &vec![1]));
    }

    #[test]
    fn deleted_adds_freed_and_removes_row() {
        let mut a = app(vec![
            ("x", Kind::Orphan, Some(4096), None),
            ("y", Kind::Orphan, Some(1), None),
        ]);
        a.apply(Update::Deleted {
            id: 0,
            note: Some("x".into()),
        });
        assert_eq!(a.freed, 4096);
        assert_eq!(a.rows.len(), 1);
        assert_eq!(a.message.as_deref(), Some("x"));
    }

    #[test]
    fn delete_failed_marks_row() {
        let mut a = app(vec![("x", Kind::Orphan, Some(1), None)]);
        a.apply(Update::DeleteFailed {
            id: 0,
            error: "locked".into(),
        });
        assert_eq!(a.rows[0].status, RowStatus::Failed("locked".into()));
    }

    #[test]
    fn fetch_failed_notes_rows_of_that_repo() {
        let mut a = app(vec![("x", Kind::Worktree, Some(1), done())]);
        a.apply(Update::FetchFailed {
            main_repo: PathBuf::from("/r/p/main"),
            error: "offline".into(),
        });
        assert!(a.rows[0].note.as_deref().unwrap().contains("offline"));
    }

    #[test]
    fn cursor_clamps_after_last_row_deleted() {
        let mut a = app(vec![
            ("big", Kind::Orphan, Some(30), None),
            ("small", Kind::Orphan, Some(10), None),
        ]);
        a.handle_key(key('j'));
        a.handle_key(key(' '));
        assert_eq!(a.cursor, 1);
        a.apply(Update::Deleted { id: 1, note: None });
        assert_eq!(a.cursor, 0);
        assert!(a.marks.is_empty());
        a.apply(Update::Deleted { id: 0, note: None });
        assert_eq!(a.cursor, 0);
        assert!(a.selected().is_none());
    }

    #[test]
    fn quit_immediately_or_ask_while_deleting() {
        let mut a = app(vec![("x", Kind::Orphan, Some(1), None)]);
        assert_eq!(a.handle_key(key('q')), vec![Command::Quit]);
        a.handle_key(key('d'));
        a.handle_key(code(KeyCode::Tab));
        a.handle_key(code(KeyCode::Enter));
        assert!(a.deleting());
        assert!(a.handle_key(key('q')).is_empty());
        assert_eq!(
            a.mode,
            Mode::ConfirmQuit {
                yes_selected: false
            }
        );
        assert!(a.handle_key(key('y')).is_empty(), "y is not a shortcut");
        assert!(
            a.handle_key(code(KeyCode::Enter)).is_empty(),
            "Enter on No stays"
        );
        assert_eq!(a.mode, Mode::Normal);
        a.handle_key(key('q'));
        a.handle_key(code(KeyCode::Tab));
        assert_eq!(a.handle_key(code(KeyCode::Enter)), vec![Command::Quit]);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut b = app(vec![]);
        assert_eq!(b.handle_key(ctrl_c), vec![Command::Quit]);
    }

    #[test]
    fn f_emits_fetch_and_marks_worktree_states_pending() {
        let mut a = app(vec![
            ("w", Kind::Worktree, Some(2), done()),
            ("o", Kind::Orphan, Some(1), None),
        ]);
        assert_eq!(a.handle_key(key('f')), vec![Command::Fetch]);
        assert_eq!(a.rows[0].git, None);
        assert_eq!(a.rows[0].state(), None);
        assert_eq!(a.rows[1].state(), Some(State::Orphan));
    }

    #[test]
    fn selection_follows_row_when_list_resorts() {
        let mut a = app(vec![
            ("x", Kind::Orphan, Some(20), None),
            ("y", Kind::Orphan, None, None),
        ]);
        a.handle_key(key('j'));
        assert_eq!(names(&a)[a.cursor], "y");
        a.apply(Update::Size {
            id: 1,
            size: Size {
                bytes: 30,
                partial: false,
            },
        });
        assert_eq!(names(&a), ["y", "x"]);
        assert_eq!(a.selected().unwrap().entry.id, 1);
    }
}
