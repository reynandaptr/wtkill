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
        Err(if err.is_empty() { format!("git {} exited with {}", args.join(" "), out.status) } else { err })
    }
}

/// Like [`run_git_bytes`] but decodes stdout as (lossy) UTF-8.
pub fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git_bytes(dir, args).map(|b| String::from_utf8_lossy(&b).into_owned())
}
