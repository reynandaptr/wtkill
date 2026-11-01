//! Core data types shared by every module.

use std::path::PathBuf;

use serde::Serialize;

/// What a child of a `.worktree/` directory is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Worktree,
    Broken,
    Orphan,
    Clone,
}

/// One row discovered under a `.worktree/` directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub id: usize,
    pub path: PathBuf,
    pub root: PathBuf,
    pub kind: Kind,
    pub main_repo: Option<PathBuf>,
    pub is_symlink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Upstream {
    None,
    Tracking {
        name: String,
        ahead: u32,
        behind: u32,
    },
    Gone {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitInfo {
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub head_short: String,
    pub dirty_count: usize,
    pub dirty_paths: Vec<String>,
    pub local_only: u32,
    pub local_only_subjects: Vec<String>,
    pub upstream: Upstream,
    pub last_commit_unix: Option<i64>,
    /// Upstream is gone and the branch's changes already landed on the
    /// remote's default branch (squash- or rebase-merged PR).
    pub squash_merged: bool,
    /// Full HEAD commit id at inspection time.
    pub head_oid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Risky,
    Done,
    Pushed,
    Orphan,
    Broken,
    Clone,
    Unknown,
}

impl State {
    pub fn label(&self) -> &'static str {
        match self {
            State::Risky => "risky",
            State::Done => "done",
            State::Pushed => "pushed",
            State::Orphan => "orphan",
            State::Broken => "broken",
            State::Clone => "clone",
            State::Unknown => "unknown",
        }
    }

    /// Deleting in this state may lose work, so the strict `yes` prompt applies.
    pub fn is_strict(&self) -> bool {
        matches!(self, State::Risky | State::Unknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Size {
    pub bytes: u64,
    pub partial: bool,
}

/// Messages from background work to the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Found(Entry),
    Size {
        id: usize,
        size: Size,
    },
    Git {
        id: usize,
        result: Result<GitInfo, String>,
    },
    FetchFailed {
        main_repo: PathBuf,
        error: String,
    },
    ScanDone,
    Deleted {
        id: usize,
        note: Option<String>,
    },
    DeleteFailed {
        id: usize,
        error: String,
    },
}

/// State of an entry; `None` while a worktree's git status is still pending.
pub fn compute_state(kind: &Kind, git: Option<&Result<GitInfo, String>>) -> Option<State> {
    match kind {
        Kind::Orphan => Some(State::Orphan),
        Kind::Broken => Some(State::Broken),
        Kind::Clone => Some(State::Clone),
        Kind::Worktree => match git? {
            Err(_) => Some(State::Unknown),
            Ok(g) if g.dirty_count > 0 || (g.local_only > 0 && !g.squash_merged) => {
                Some(State::Risky)
            }
            Ok(g) if matches!(g.upstream, Upstream::Gone { .. }) => Some(State::Done),
            Ok(_) => Some(State::Pushed),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gi(dirty: usize, local: u32, up: Upstream) -> GitInfo {
        GitInfo {
            branch: Some("b".into()),
            head_short: "abc1234".into(),
            dirty_count: dirty,
            dirty_paths: vec![],
            local_only: local,
            local_only_subjects: vec![],
            upstream: up,
            last_commit_unix: None,
            squash_merged: false,
            head_oid: "abc1234abc".into(),
        }
    }

    #[test]
    fn state_rules() {
        let gone = || Upstream::Gone {
            name: "origin/b".into(),
        };
        assert_eq!(compute_state(&Kind::Worktree, None), None);
        assert_eq!(
            compute_state(&Kind::Worktree, Some(&Ok(gi(1, 0, Upstream::None)))),
            Some(State::Risky)
        );
        assert_eq!(
            compute_state(&Kind::Worktree, Some(&Ok(gi(0, 2, gone())))),
            Some(State::Risky)
        );
        assert_eq!(
            compute_state(&Kind::Worktree, Some(&Ok(gi(0, 0, gone())))),
            Some(State::Done)
        );
        let squashed = GitInfo {
            squash_merged: true,
            ..gi(0, 2, gone())
        };
        assert_eq!(
            compute_state(&Kind::Worktree, Some(&Ok(squashed))),
            Some(State::Done)
        );
        assert_eq!(
            compute_state(
                &Kind::Worktree,
                Some(&Ok(gi(
                    0,
                    0,
                    Upstream::Tracking {
                        name: "origin/b".into(),
                        ahead: 0,
                        behind: 3
                    }
                )))
            ),
            Some(State::Pushed)
        );
        assert_eq!(
            compute_state(&Kind::Worktree, Some(&Ok(gi(0, 0, Upstream::None)))),
            Some(State::Pushed)
        );
        assert_eq!(
            compute_state(&Kind::Worktree, Some(&Err("x".into()))),
            Some(State::Unknown)
        );
        assert_eq!(compute_state(&Kind::Orphan, None), Some(State::Orphan));
        assert_eq!(compute_state(&Kind::Broken, None), Some(State::Broken));
        assert_eq!(compute_state(&Kind::Clone, None), Some(State::Clone));
        assert!(State::Risky.is_strict() && State::Unknown.is_strict() && !State::Done.is_strict());
    }
}
