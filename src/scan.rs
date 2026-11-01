//! Discovery: find `.worktree/` directories and classify their children.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git::run_git;
use crate::model::{Entry, Kind};

const WORKTREE_DIR: &str = ".worktree";
const SKIP_DIRS: [&str; 3] = [".git", "node_modules", "target"];

/// Walk `roots` and call `on_found` for every child of every `.worktree/`
/// directory. Symlinks are never followed; entries reachable from several
/// roots are reported once. Ids are assigned sequentially from 0.
pub fn discover(roots: &[PathBuf], mut on_found: impl FnMut(Entry)) {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut next_id = 0;
    for root in roots {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        let mut emit = |path: PathBuf| {
            if !seen.insert(path.clone()) {
                return;
            }
            let (kind, main_repo, is_symlink) = classify(&path);
            on_found(Entry {
                id: next_id,
                path,
                root: root.clone(),
                kind,
                main_repo,
                is_symlink,
            });
            next_id += 1;
        };
        if root.file_name().is_some_and(|n| n == WORKTREE_DIR) {
            sorted_children(&root).into_iter().for_each(&mut emit);
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let mut subdirs = vec![];
            for child in sorted_children(&dir) {
                let Ok(meta) = fs::symlink_metadata(&child) else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                let name = child.file_name().unwrap_or_default();
                if name == WORKTREE_DIR {
                    sorted_children(&child).into_iter().for_each(&mut emit);
                } else if !SKIP_DIRS.iter().any(|s| name == *s) {
                    subdirs.push(child);
                }
            }
            // Reverse so the stack pops in name order (deterministic ids).
            stack.extend(subdirs.into_iter().rev());
        }
    }
}

fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

fn classify(path: &Path) -> (Kind, Option<PathBuf>, bool) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (Kind::Orphan, None, false);
    };
    if meta.file_type().is_symlink() {
        return (Kind::Orphan, None, true);
    }
    if !meta.is_dir() {
        return (Kind::Orphan, None, false);
    }
    let dotgit = path.join(".git");
    match fs::symlink_metadata(&dotgit) {
        Ok(m) if m.is_dir() => (Kind::Clone, None, false),
        Ok(m) if m.is_file() => match read_gitdir(path) {
            Some(gitdir) if gitdir.exists() => (Kind::Worktree, main_repo_of(path), false),
            Some(gitdir) => (Kind::Broken, main_repo_from_gitdir(&gitdir), false),
            None => (Kind::Broken, None, false),
        },
        _ => (Kind::Orphan, None, false),
    }
}

/// The `gitdir:` target of `<worktree>/.git`, resolved against the worktree
/// directory when relative.
fn read_gitdir(worktree: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(worktree.join(".git")).ok()?;
    let value = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let p = PathBuf::from(value);
    Some(if p.is_absolute() { p } else { worktree.join(p) })
}

/// `<main>/.git/worktrees/<name>` → `<main>`, if `<main>/.git` still exists.
fn main_repo_from_gitdir(gitdir: &Path) -> Option<PathBuf> {
    let common = gitdir.parent()?.parent()?;
    let common = common.canonicalize().ok()?;
    if common.file_name()? == ".git" {
        common.parent().map(Path::to_path_buf)
    } else {
        Some(common)
    }
}

/// The main repository a worktree belongs to.
pub fn main_repo_of(worktree: &Path) -> Option<PathBuf> {
    let out = run_git(
        worktree,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let common = PathBuf::from(out.trim()).canonicalize().ok()?;
    if common.file_name()? == ".git" {
        common.parent().map(Path::to_path_buf)
    } else {
        Some(common)
    }
}
