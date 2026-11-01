//! The one place that spawns `git`. Paths are passed as separate args,
//! never through a shell.

use std::path::Path;
use std::process::Command;

/// Run `git -C <dir> <args>`; stdout bytes on success, trimmed stderr on failure.
pub fn run_git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("git {} exited with {}", args.join(" "), out.status)
        } else {
            err
        })
    }
}

/// Oldest git wtkill supports: `git merge-tree --write-tree` (squash-merge
/// detection) arrived in 2.38.
pub const MIN_GIT: (u32, u32, u32) = (2, 38, 0);

/// Parse `git --version` output such as `git version 2.39.5 (Apple Git-154)`.
pub fn parse_git_version(out: &str) -> Option<(u32, u32, u32)> {
    let version = out.split_whitespace().nth(2)?;
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

/// The installed git's version, or `None` if git can't be run or parsed.
pub fn git_version() -> Option<(u32, u32, u32)> {
    let out = Command::new("git").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_git_version(&String::from_utf8_lossy(&out.stdout))
}

/// Like [`run_git_bytes`] but decodes stdout as (lossy) UTF-8.
pub fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git_bytes(dir, args).map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_versions() {
        assert_eq!(parse_git_version("git version 2.53.0\n"), Some((2, 53, 0)));
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39, 5))
        );
        assert_eq!(
            parse_git_version("git version 2.45.1.windows.1"),
            Some((2, 45, 1))
        );
        assert_eq!(parse_git_version("git version 2.40"), Some((2, 40, 0)));
        assert_eq!(parse_git_version("not git"), None);
    }

    #[test]
    fn minimum_is_2_38() {
        assert_eq!(MIN_GIT, (2, 38, 0));
        assert!((2, 37, 9) < MIN_GIT && (2, 38, 0) >= MIN_GIT);
    }
}
