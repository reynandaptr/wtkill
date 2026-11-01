mod common;

use common::Fixture;
use wtkill::model::{Entry, Kind};
use wtkill::scan::discover;

fn scan(roots: &[std::path::PathBuf]) -> Vec<Entry> {
    let mut got = vec![];
    discover(roots, |e| got.push(e));
    got
}

#[test]
fn classifies_every_child() {
    let f = Fixture::standard();
    let got = scan(std::slice::from_ref(&f.root));
    let by = |n: &str| {
        got.iter()
            .find(|e| e.path.file_name().unwrap() == n)
            .unwrap_or_else(|| panic!("{n} not found in {got:#?}"))
            .clone()
    };
    let main = f.main.canonicalize().unwrap();
    for n in [
        "app-done",
        "app-pushed",
        "app-dirty",
        "app-local",
        "app-feat x+1",
    ] {
        assert_eq!(by(n).kind, Kind::Worktree, "{n}");
        assert_eq!(by(n).main_repo.as_deref(), Some(main.as_path()), "{n}");
    }
    assert_eq!(by("app-broken").kind, Kind::Broken);
    assert_eq!(by("app-broken").main_repo.as_deref(), Some(main.as_path()));
    assert_eq!(by("orphan-dir").kind, Kind::Orphan);
    assert_eq!(by("notes.md").kind, Kind::Orphan);
    assert_eq!(by("clone").kind, Kind::Clone);
    assert_eq!(got.len(), 9, "{got:#?}");
    let ids: Vec<_> = got.iter().map(|e| e.id).collect();
    assert_eq!(ids, (0..9).collect::<Vec<_>>());
}

#[test]
fn overlapping_roots_dedupe() {
    let f = Fixture::standard();
    assert_eq!(scan(&[f.root.clone(), f.root.join("proj")]).len(), 9);
}

#[test]
fn relative_gitdir_is_worktree() {
    // worktree.useRelativePaths needs git 2.48+ (macOS runners may be older).
    if wtkill::git::git_version() < Some((2, 48, 0)) {
        eprintln!("skipped: needs git >= 2.48");
        return;
    }
    let f = Fixture::standard();
    let p = f.wt("app-rel");
    f.git(
        &f.main,
        &[
            "-c",
            "worktree.useRelativePaths=true",
            "worktree",
            "add",
            "-q",
            "-b",
            "rel",
            p.to_str().unwrap(),
        ],
    );
    let gitfile = std::fs::read_to_string(p.join(".git")).unwrap();
    assert!(
        !gitfile
            .trim_start_matches("gitdir:")
            .trim()
            .starts_with('/'),
        "fixture should be relative: {gitfile}"
    );
    let got = scan(std::slice::from_ref(&f.root));
    let e = got.iter().find(|e| e.path.ends_with("app-rel")).unwrap();
    assert_eq!(e.kind, Kind::Worktree);
}

#[test]
fn symlinked_child_not_followed() {
    let f = Fixture::standard();
    std::os::unix::fs::symlink(&f.main, f.wt("link")).unwrap();
    let got = scan(std::slice::from_ref(&f.root));
    let e = got.iter().find(|e| e.path.ends_with("link")).unwrap();
    assert!(e.is_symlink);
    assert_eq!(e.kind, Kind::Orphan);
}
