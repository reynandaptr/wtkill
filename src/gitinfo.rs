//! Read a worktree's git state by running `git` and parsing its output.

use std::path::Path;

use crate::git::{run_git, run_git_bytes};
use crate::model::{GitInfo, Upstream};

const MAX_DIRTY_PATHS: usize = 20;
const MAX_SUBJECTS: usize = 10;

/// Parse `git status --porcelain=v1 -z`: entry count plus the first 20 paths.
/// A rename/copy entry is followed by its source path, which is skipped.
pub fn parse_porcelain_z(out: &[u8]) -> (usize, Vec<String>) {
    let mut fields = out.split(|b| *b == 0).filter(|f| !f.is_empty());
    let mut count = 0;
    let mut paths = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        count += 1;
        if paths.len() < MAX_DIRTY_PATHS {
            paths.push(String::from_utf8_lossy(&field[3..]).into_owned());
        }
        if matches!(field[0], b'R' | b'C') || matches!(field[1], b'R' | b'C') {
            fields.next();
        }
    }
    (count, paths)
}

/// Parse `for-each-ref --format='%(upstream:short)%00%(upstream:track)'`.
pub fn parse_upstream(line: &str) -> Upstream {
    let mut parts = line.splitn(2, '\0');
    let name = parts.next().unwrap_or("").trim();
    if name.is_empty() {
        return Upstream::None;
    }
    let track = parts.next().unwrap_or("").trim();
    if track == "[gone]" {
        return Upstream::Gone { name: name.to_string() };
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in track.trim_start_matches('[').trim_end_matches(']').split(',') {
        let mut words = part.split_whitespace();
        match (words.next(), words.next().and_then(|n| n.parse().ok())) {
            (Some("ahead"), Some(n)) => ahead = n,
            (Some("behind"), Some(n)) => behind = n,
            _ => {}
        }
    }
    Upstream::Tracking { name: name.to_string(), ahead, behind }
}

/// Inspect a worktree. Any failing git command fails the whole inspection.
pub fn inspect(worktree: &Path) -> Result<GitInfo, String> {
    let branch = run_git(worktree, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let head_oid = run_git(worktree, &["rev-parse", "HEAD"])?.trim().to_string();
    let head_short = head_oid.chars().take(7).collect();
    let (dirty_count, dirty_paths) =
        parse_porcelain_z(&run_git_bytes(worktree, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=none"])?);
    let local_only = run_git(worktree, &["rev-list", "--count", "HEAD", "--not", "--remotes"])?
        .trim()
        .parse()
        .map_err(|e| format!("unexpected rev-list output: {e}"))?;
    let max = format!("-n{MAX_SUBJECTS}");
    let local_only_subjects = run_git(worktree, &["log", "--format=%s", &max, "HEAD", "--not", "--remotes"])?
        .lines()
        .map(str::to_string)
        .collect();
    let upstream = match &branch {
        None => Upstream::None,
        Some(b) => parse_upstream(&run_git(
            worktree,
            &["for-each-ref", "--format=%(upstream:short)%00%(upstream:track)", &format!("refs/heads/{b}")],
        )?),
    };
    let last_commit_unix = run_git(worktree, &["log", "-1", "--format=%ct"])?.trim().parse().ok();
    let squash_merged = match &upstream {
        Upstream::Gone { name } if local_only > 0 => is_merged_into_default(worktree, name),
        _ => false,
    };
    Ok(GitInfo {
        branch,
        head_short,
        dirty_count,
        dirty_paths,
        local_only,
        local_only_subjects,
        upstream,
        last_commit_unix,
        squash_merged,
        head_oid,
    })
}

/// Whether HEAD's changes already landed on the default branch of the remote
/// named by `upstream` (e.g. `origin/feat` → `origin`): merging HEAD into that
/// branch would change nothing. Content-exact (whitespace, merge commits and
/// conflict resolutions all count); any merge conflict means "not merged".
/// Writes only tree objects, which git gc collects.
fn is_merged_into_default(worktree: &Path, upstream: &str) -> bool {
    let Some(remote) = upstream.split('/').next() else { return false };
    let Some(base) = default_branch(worktree, remote) else { return false };
    let Ok(merged) = run_git(worktree, &["merge-tree", "--write-tree", &base, "HEAD"]) else { return false };
    let Ok(base_tree) = run_git(worktree, &["rev-parse", &format!("{base}^{{tree}}")]) else { return false };
    merged.lines().next().map(str::trim) == Some(base_tree.trim())
}

/// `<remote>/HEAD` if set, else `<remote>/main`, else `<remote>/master`.
fn default_branch(worktree: &Path, remote: &str) -> Option<String> {
    [format!("{remote}/HEAD"), format!("{remote}/main"), format!("{remote}/master")]
        .into_iter()
        .find(|r| run_git(worktree, &["rev-parse", "--verify", "-q", &format!("refs/remotes/{r}")]).is_ok())
}

/// `git fetch --prune` in a main repository.
pub fn fetch(main_repo: &Path) -> Result<(), String> {
    run_git(main_repo, &["fetch", "--prune", "--quiet"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parsing() {
        assert_eq!(parse_porcelain_z(b""), (0, vec![]));
        assert_eq!(
            parse_porcelain_z(b" M a.rs\0?? new file.txt\0R  new.rs\0old.rs\0"),
            (3, vec!["a.rs".to_string(), "new file.txt".into(), "new.rs".into()])
        );
    }

    #[test]
    fn porcelain_keeps_first_20_paths() {
        let mut out = Vec::new();
        for i in 0..25 {
            out.extend_from_slice(format!("?? f{i}\0").as_bytes());
        }
        let (n, paths) = parse_porcelain_z(&out);
        assert_eq!(n, 25);
        assert_eq!(paths.len(), 20);
    }

    #[test]
    fn upstream_parsing() {
        let t = |a, b| Upstream::Tracking { name: "origin/x".into(), ahead: a, behind: b };
        assert_eq!(parse_upstream(""), Upstream::None);
        assert_eq!(parse_upstream("\0"), Upstream::None);
        assert_eq!(parse_upstream("origin/x\0[gone]"), Upstream::Gone { name: "origin/x".into() });
        assert_eq!(parse_upstream("origin/x\0"), t(0, 0));
        assert_eq!(parse_upstream("origin/x\0[ahead 2, behind 1]"), t(2, 1));
        assert_eq!(parse_upstream("origin/x\0[behind 4]"), t(0, 4));
        assert_eq!(parse_upstream("origin/x\0[ahead 3]\n"), t(3, 0));
    }
}
