mod common;

use common::Fixture;
use wtkill::gitinfo::{fetch, inspect};
use wtkill::model::{compute_state, Kind, State, Upstream};

fn state(f: &Fixture, name: &str) -> State {
    compute_state(&Kind::Worktree, Some(&inspect(&f.wt(name)))).unwrap()
}

#[test]
fn done_pushed_and_slash_branch() {
    let f = Fixture::standard();
    assert_eq!(state(&f, "app-done"), State::Done);
    let done = inspect(&f.wt("app-done")).unwrap();
    assert!(matches!(done.upstream, Upstream::Gone { .. }));
    assert!(done.squash_merged, "{done:?}");
    assert_eq!(state(&f, "app-pushed"), State::Pushed);
    let feat = inspect(&f.wt("app-feat x+1")).unwrap();
    assert_eq!(feat.branch.as_deref(), Some("feat/x"));
    assert_eq!(state(&f, "app-feat x+1"), State::Pushed);
    assert!(feat.last_commit_unix.is_some());
}

#[test]
fn dirty_is_risky() {
    let f = Fixture::standard();
    let g = inspect(&f.wt("app-dirty")).unwrap();
    assert_eq!(g.dirty_count, 2);
    assert_eq!(state(&f, "app-dirty"), State::Risky);
}

#[test]
fn local_only_commits_are_risky() {
    let f = Fixture::standard();
    let g = inspect(&f.wt("app-local")).unwrap();
    assert_eq!(g.local_only, 2);
    assert_eq!(g.local_only_subjects.len(), 2);
    assert_eq!(state(&f, "app-local"), State::Risky);
}

#[test]
fn detached_head() {
    let f = Fixture::standard();
    f.git(&f.wt("app-pushed"), &["checkout", "-q", "--detach"]);
    let g = inspect(&f.wt("app-pushed")).unwrap();
    assert_eq!(g.branch, None);
    assert!(!g.head_short.is_empty());
    assert_eq!(state(&f, "app-pushed"), State::Pushed);
}

#[test]
fn non_repo_is_error() {
    let f = Fixture::standard();
    assert!(inspect(&f.wt("orphan-dir")).is_err());
}

#[test]
fn fetch_ok_and_err() {
    let f = Fixture::standard();
    assert_eq!(fetch(&f.main), Ok(()));
    assert!(fetch(&f.wt("orphan-dir")).is_err());
}

#[test]
fn gone_without_merge_is_risky() {
    let f = Fixture::standard();
    f.git(&f.remote, &["branch", "-q", "-D", "pushed"]);
    f.git(&f.main, &["fetch", "-q", "--prune"]);
    let g = inspect(&f.wt("app-pushed")).unwrap();
    assert!(matches!(g.upstream, Upstream::Gone { .. }));
    assert!(!g.squash_merged);
    assert_eq!(state(&f, "app-pushed"), State::Risky);
}

#[test]
fn whitespace_change_after_squash_is_risky() {
    // Patch-ids ignore whitespace; a re-indent after the squash merge is real work.
    let f = Fixture::standard();
    let wt = f.wt("app-done");
    std::fs::write(wt.join("f.txt"), "    done work\n").unwrap();
    f.git(&wt, &["commit", "-q", "-am", "reindent"]);
    let g = inspect(&wt).unwrap();
    assert!(!g.squash_merged, "{g:?}");
    assert_eq!(state(&f, "app-done"), State::Risky);
}

#[test]
fn local_merge_commit_with_new_work_is_risky() {
    // `git cherry` skips merge commits: here commit A lands upstream via
    // cherry-pick, but a never-pushed merge commit carries extra work.
    let f = Fixture::standard();
    let wt = f.wt("app-evil");
    f.git(&f.main, &["worktree", "add", "-q", "-b", "evilb", wt.to_str().unwrap()]);
    std::fs::write(f.main.join("other.txt"), "other PR\n").unwrap();
    f.git(&f.main, &["add", "other.txt"]);
    f.git(&f.main, &["commit", "-q", "-m", "other PR"]);
    f.git(&f.main, &["push", "-q", "origin", "main"]);
    std::fs::write(wt.join("b.txt"), "b\n").unwrap();
    f.git(&wt, &["add", "b.txt"]);
    f.git(&wt, &["commit", "-q", "-m", "A: add b"]);
    f.git(&wt, &["push", "-q", "-u", "origin", "evilb"]);
    f.git(&wt, &["fetch", "-q"]);
    f.git(&wt, &["merge", "-q", "--no-ff", "--no-commit", "origin/main"]);
    std::fs::write(wt.join("evil.txt"), "precious\n").unwrap();
    f.git(&wt, &["add", "evil.txt"]);
    f.git(&wt, &["commit", "-q", "-m", "merge main + extra work"]);
    let a = f.git(&wt, &["rev-parse", "HEAD^1"]);
    f.git(&f.main, &["cherry-pick", "-x", a.trim()]);
    f.git(&f.main, &["push", "-q", "origin", "main"]);
    f.git(&f.remote, &["branch", "-q", "-D", "evilb"]);
    f.git(&f.main, &["fetch", "-q", "--prune"]);
    assert_eq!(state(&f, "app-evil"), State::Risky);
}

#[test]
fn untracked_counted_even_if_repo_hides_them() {
    let f = Fixture::standard();
    f.git(&f.main, &["config", "status.showUntrackedFiles", "no"]);
    std::fs::write(f.wt("app-pushed").join("notes.txt"), "mine\n").unwrap();
    assert_eq!(inspect(&f.wt("app-pushed")).unwrap().dirty_count, 1);
    assert_eq!(state(&f, "app-pushed"), State::Risky);
}

#[test]
fn head_oid_is_full_sha() {
    let f = Fixture::standard();
    let g = inspect(&f.wt("app-pushed")).unwrap();
    assert_eq!(g.head_oid, f.git(&f.wt("app-pushed"), &["rev-parse", "HEAD"]).trim());
}
