mod common;

use std::path::PathBuf;

use common::Fixture;
use wtkill::delete::{Action, check_guardrails, delete_entry};
use wtkill::gitinfo::inspect;
use wtkill::model::{Entry, Kind, compute_state};
use wtkill::scan::discover;

fn entry(f: &Fixture, name: &str) -> Entry {
    let mut found = None;
    discover(std::slice::from_ref(&f.root), |e| {
        if e.path.file_name().unwrap() == name {
            found = Some(e);
        }
    });
    found.unwrap_or_else(|| panic!("{name} not found"))
}

fn run(f: &Fixture, name: &str, action: Action) -> Result<Option<String>, String> {
    let e = entry(f, name);
    let git = (e.kind == Kind::Worktree).then(|| inspect(&e.path));
    let strict = compute_state(&e.kind, git.as_ref()).unwrap().is_strict();
    let info = git.and_then(Result::ok);
    delete_entry(
        &e,
        info.as_ref(),
        strict,
        action,
        std::slice::from_ref(&f.root),
        f.tmp.path(),
    )
}

fn worktree_list(f: &Fixture) -> String {
    f.git(&f.main, &["worktree", "list"])
}

#[test]
fn remove_done_keeps_branch() {
    let f = Fixture::standard();
    assert_eq!(run(&f, "app-done", Action::Remove), Ok(None));
    assert!(!f.wt("app-done").exists());
    assert!(!worktree_list(&f).contains("app-done"));
    assert!(
        !f.git(&f.main, &["branch", "--list", "done"])
            .trim()
            .is_empty()
    );
}

#[test]
fn remove_and_branch() {
    let f = Fixture::standard();
    assert_eq!(run(&f, "app-feat x+1", Action::RemoveAndBranch), Ok(None));
    assert!(!f.wt("app-feat x+1").exists());
    assert!(
        f.git(&f.main, &["branch", "--list", "feat/x"])
            .trim()
            .is_empty()
    );
}

#[test]
fn risky_uses_force() {
    let f = Fixture::standard();
    assert_eq!(run(&f, "app-dirty", Action::Remove), Ok(None));
    assert!(!f.wt("app-dirty").exists());
    assert!(!worktree_list(&f).contains("app-dirty"));
}

#[test]
fn broken_rm_and_prune() {
    let f = Fixture::standard();
    assert_eq!(run(&f, "app-broken", Action::Remove), Ok(None));
    assert!(!f.wt("app-broken").exists());
}

#[test]
fn orphans() {
    let f = Fixture::standard();
    assert_eq!(run(&f, "orphan-dir", Action::Remove), Ok(None));
    assert_eq!(run(&f, "notes.md", Action::Remove), Ok(None));
    assert!(!f.wt("orphan-dir").exists());
    assert!(!f.wt("notes.md").exists());
}

#[test]
fn clone_refused() {
    let f = Fixture::standard();
    let err = run(&f, "clone", Action::Remove).unwrap_err();
    assert!(err.contains("delete manually"), "{err}");
    assert!(f.wt("clone").exists());
}

#[test]
fn symlink_removed_as_link_only() {
    let f = Fixture::standard();
    std::os::unix::fs::symlink(&f.main, f.wt("link")).unwrap();
    assert_eq!(run(&f, "link", Action::Remove), Ok(None));
    assert!(!f.wt("link").exists());
    assert!(f.main.join("README.md").exists());
}

#[test]
fn detached_branch_delete_is_note() {
    let f = Fixture::standard();
    f.git(&f.wt("app-pushed"), &["checkout", "-q", "--detach"]);
    let note = run(&f, "app-pushed", Action::RemoveAndBranch)
        .unwrap()
        .expect("note");
    assert!(note.contains("detached"), "{note}");
    assert!(!f.wt("app-pushed").exists());
    assert!(
        !f.git(&f.main, &["branch", "--list", "pushed"])
            .trim()
            .is_empty()
    );
}

#[test]
fn guardrails() {
    let f = Fixture::standard();
    let roots = [f.root.clone()];
    let cwd = f.tmp.path();
    assert!(check_guardrails(&f.wt("app-done"), &roots, cwd).is_ok());
    assert!(
        check_guardrails(&f.main, &roots, cwd).is_err(),
        "parent not .worktree"
    );
    let outside: PathBuf = f.tmp.path().join("elsewhere/.worktree/x");
    std::fs::create_dir_all(&outside).unwrap();
    assert!(
        check_guardrails(&outside, &roots, cwd).is_err(),
        "outside roots"
    );
    let inside = f.wt("app-done").join("sub");
    std::fs::create_dir_all(&inside).unwrap();
    assert!(
        check_guardrails(&f.wt("app-done"), &roots, &inside).is_err(),
        "contains cwd"
    );
    assert!(
        check_guardrails(&f.wt("app-done"), &roots, &f.wt("app-done")).is_err(),
        "is cwd"
    );
}

#[test]
fn commit_after_scan_aborts() {
    let f = Fixture::standard();
    let e = entry(&f, "app-pushed");
    let scanned = inspect(&e.path).unwrap();
    std::fs::write(e.path.join("late.txt"), "late\n").unwrap();
    f.git(&e.path, &["add", "late.txt"]);
    f.git(&e.path, &["commit", "-q", "-m", "late commit"]);
    let err = delete_entry(
        &e,
        Some(&scanned),
        false,
        Action::RemoveAndBranch,
        std::slice::from_ref(&f.root),
        f.tmp.path(),
    )
    .unwrap_err();
    assert!(err.contains("re-confirm"), "{err}");
    assert!(e.path.exists());
    assert!(
        !f.git(&f.main, &["branch", "--list", "pushed"])
            .trim()
            .is_empty()
    );
}

#[test]
fn dirty_after_nonstrict_confirm_aborts() {
    let f = Fixture::standard();
    let e = entry(&f, "app-pushed");
    let scanned = inspect(&e.path).unwrap();
    std::fs::write(e.path.join("new.txt"), "unsaved\n").unwrap();
    let err = delete_entry(
        &e,
        Some(&scanned),
        false,
        Action::Remove,
        std::slice::from_ref(&f.root),
        f.tmp.path(),
    )
    .unwrap_err();
    assert!(err.contains("re-confirm"), "{err}");
    assert!(e.path.join("new.txt").exists());
}

#[test]
fn branch_checked_out_elsewhere_is_note() {
    let f = Fixture::standard();
    let other = f.tmp.path().join("elsewhere");
    f.git(
        &f.main,
        &[
            "worktree",
            "add",
            "-q",
            "--force",
            other.to_str().unwrap(),
            "pushed",
        ],
    );
    let note = run(&f, "app-pushed", Action::RemoveAndBranch)
        .unwrap()
        .expect("note");
    assert!(note.contains("checked out"), "{note}");
    assert!(!f.wt("app-pushed").exists());
    assert!(
        !f.git(&f.main, &["branch", "--list", "pushed"])
            .trim()
            .is_empty()
    );
}
