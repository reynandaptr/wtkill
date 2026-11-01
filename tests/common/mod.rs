//! Test fixture: a bare "remote", a main clone, and worktrees under
//! `.worktree/` in every state the spec names.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub struct Fixture {
    pub tmp: TempDir,
    /// `tmp/src` — the scan root.
    pub root: PathBuf,
    /// `tmp/remote.git` — bare repo acting as `origin`.
    pub remote: PathBuf,
    /// `root/proj/app` — main working copy.
    pub main: PathBuf,
    /// `root/proj/.worktree`.
    pub wt_dir: PathBuf,
}

impl Fixture {
    /// Run git in `dir` with an isolated config; panics on failure.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        git_in(dir, args)
    }

    pub fn wt(&self, name: &str) -> PathBuf {
        self.wt_dir.join(name)
    }

    fn add_worktree(&self, name: &str, branch: &str) -> PathBuf {
        let p = self.wt(name);
        git_in(
            &self.main,
            &["worktree", "add", "-q", "-b", branch, p.to_str().unwrap()],
        );
        p
    }

    fn commit(&self, wt: &Path, file: &str, msg: &str) {
        fs::write(wt.join(file), format!("{msg}\n")).unwrap();
        git_in(wt, &["add", file]);
        git_in(wt, &["commit", "-q", "-m", msg]);
    }

    fn push(&self, wt: &Path, branch: &str) {
        git_in(wt, &["push", "-q", "-u", "origin", branch]);
    }

    /// Every state from the spec; see the plan's Task 2 table.
    pub fn standard() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("src");
        let remote = tmp.path().join("remote.git");
        let main = root.join("proj/app");
        let wt_dir = root.join("proj/.worktree");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(&wt_dir).unwrap();
        git_in(
            tmp.path(),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        git_in(&main, &["init", "-q"]);
        let f = Fixture {
            tmp,
            root,
            remote,
            main,
            wt_dir,
        };

        f.commit(&f.main, "README.md", "init");
        git_in(
            &f.main,
            &["remote", "add", "origin", f.remote.to_str().unwrap()],
        );
        f.push(&f.main, "main");

        let done = f.add_worktree("app-done", "done");
        f.commit(&done, "f.txt", "done work");
        f.push(&done, "done");
        // Squash-merge the PR into main on the remote, then delete its branch.
        git_in(&f.main, &["merge", "-q", "--squash", "done"]);
        git_in(&f.main, &["commit", "-q", "-m", "squash: done"]);
        f.push(&f.main, "main");
        git_in(&f.remote, &["branch", "-q", "-D", "done"]);
        git_in(&f.main, &["fetch", "-q", "--prune"]);

        let pushed = f.add_worktree("app-pushed", "pushed");
        f.commit(&pushed, "f.txt", "pushed work");
        f.push(&pushed, "pushed");

        let dirty = f.add_worktree("app-dirty", "dirty");
        f.commit(&dirty, "f.txt", "dirty work");
        f.push(&dirty, "dirty");
        fs::write(dirty.join("untracked.txt"), "u\n").unwrap();
        fs::write(dirty.join("f.txt"), "modified\n").unwrap();

        let local = f.add_worktree("app-local", "local");
        f.push(&local, "local");
        f.commit(&local, "a.txt", "local one");
        f.commit(&local, "b.txt", "local two");

        let feat = f.add_worktree("app-feat x+1", "feat/x");
        f.commit(&feat, "f.txt", "feat work");
        f.push(&feat, "feat/x");

        f.add_worktree("app-broken", "broken");
        fs::remove_dir_all(f.main.join(".git/worktrees/app-broken")).unwrap();

        fs::create_dir_all(f.wt("orphan-dir")).unwrap();
        fs::write(f.wt("orphan-dir/file.txt"), "x\n").unwrap();
        fs::write(f.wt("notes.md"), "# notes\n").unwrap();
        git_in(
            &f.wt_dir,
            &["clone", "-q", f.remote.to_str().unwrap(), "clone"],
        );

        fs::create_dir_all(f.main.join("node_modules/x/.worktree/ignored")).unwrap();
        fs::create_dir_all(f.root.join("other/.worktree")).unwrap();
        f
    }
}

pub fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
