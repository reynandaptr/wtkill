//! Delete actions, guarded so nothing outside a `.worktree/` dir is touched.

use std::fs;
use std::path::{Path, PathBuf};

use crate::git::run_git;
use crate::gitinfo::inspect;
use crate::model::{Entry, GitInfo, Kind, State, compute_state};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Remove the worktree / folder.
    Remove,
    /// Also delete the worktree's local branch in the main repo.
    RemoveAndBranch,
}

/// Refuse paths whose parent isn't a `.worktree` dir, that lie outside every
/// scan root, or that contain the current working directory.
pub fn check_guardrails(path: &Path, roots: &[PathBuf], cwd: &Path) -> Result<(), String> {
    let name = path.file_name().ok_or_else(|| format!("{} has no file name", path.display()))?;
    let parent = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| format!("cannot resolve parent of {}", path.display()))?;
    if parent.file_name().is_none_or(|n| n != ".worktree") {
        return Err(format!("refusing: {} is not inside a .worktree directory", path.display()));
    }
    let in_root = roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| parent.starts_with(&r));
    if !in_root {
        return Err(format!("refusing: {} is outside the scanned roots", path.display()));
    }
    let target = parent.join(name);
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if cwd.starts_with(&target) {
        return Err(format!("refusing: {} contains the current directory", path.display()));
    }
    Ok(())
}

/// Delete one entry per the spec's Deletion table. `Ok(Some(note))` reports a
/// skipped or failed branch delete, which is not an error.
pub fn delete_entry(
    entry: &Entry,
    git: Option<&GitInfo>,
    confirmed_strict: bool,
    action: Action,
    roots: &[PathBuf],
    cwd: &Path,
) -> Result<Option<String>, String> {
    check_guardrails(&entry.path, roots, cwd)?;
    if entry.is_symlink {
        fs::remove_file(&entry.path).map_err(|e| format!("remove link: {e}"))?;
        return Ok(None);
    }
    match entry.kind {
        Kind::Clone => Err("full git clone — delete manually".to_string()),
        Kind::Orphan => remove_path(&entry.path).map(|_| None),
        Kind::Broken => {
            remove_path(&entry.path)?;
            Ok(entry.main_repo.as_deref().and_then(|m| prune(m).err()))
        }
        Kind::Worktree => {
            let main = entry.main_repo.as_deref().ok_or("main repository unknown")?;
            let path = entry.path.to_str().ok_or("path is not valid UTF-8")?;
            // Re-check right before acting: the TUI may have been open a while.
            let fresh = match inspect(&entry.path) {
                Ok(fresh) => Some(fresh),
                Err(_) if confirmed_strict => None,
                Err(e) => return Err(format!("cannot re-check git state ({e}) — re-confirm")),
            };
            if let Some(fresh) = &fresh {
                let now = compute_state(&Kind::Worktree, Some(&Ok(fresh.clone()))).unwrap_or(State::Unknown);
                if !confirmed_strict && now.is_strict() {
                    return Err(format!("state changed to {} since you confirmed — re-confirm", now.label()));
                }
                if let Some(g) = git
                    && (fresh.head_oid != g.head_oid || fresh.dirty_count > g.dirty_count) {
                        return Err("worktree changed since the scan — re-confirm".to_string());
                    }
            }
            let mut args = vec!["worktree", "remove"];
            if confirmed_strict {
                args.push("--force");
            }
            args.push(path);
            run_git(main, &args)?;
            let mut notes: Vec<String> = prune(main).err().into_iter().collect();
            if action == Action::RemoveAndBranch {
                let info = fresh.as_ref().or(git);
                match info.and_then(|g| g.branch.as_deref().map(|b| (b, g.head_oid.as_str()))) {
                    None => notes.push("branch not deleted: HEAD was detached or unknown".to_string()),
                    Some((b, oid)) => {
                        if let Some(note) = delete_branch(main, b, oid) {
                            notes.push(note);
                        }
                    }
                }
            }
            Ok((!notes.is_empty()).then(|| notes.join("; ")))
        }
    }
}

/// Delete `refs/heads/<branch>` only if no other worktree has it checked out
/// and its tip is still `oid`. Returns a note when the branch is kept.
fn delete_branch(main: &Path, branch: &str, oid: &str) -> Option<String> {
    let full = format!("refs/heads/{branch}");
    match run_git(main, &["worktree", "list", "--porcelain"]) {
        Ok(list) if list.lines().any(|l| l.strip_prefix("branch ") == Some(full.as_str())) => {
            return Some(format!("branch {branch} not deleted: checked out in another worktree"));
        }
        Ok(_) => {}
        Err(e) => return Some(format!("branch {branch} not deleted: {e}")),
    }
    run_git(main, &["update-ref", "-d", &full, oid])
        .err()
        .map(|e| format!("branch {branch} not deleted: {e}"))
}

fn prune(main: &Path) -> Result<(), String> {
    run_git(main, &["worktree", "prune"]).map(|_| ()).map_err(|e| format!("worktree prune failed: {e}"))
}

fn remove_path(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let res = if meta.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    res.map_err(|e| format!("remove {}: {e}", path.display()))
}
