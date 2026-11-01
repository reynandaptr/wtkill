mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::Fixture;
use serde_json::Value;

fn wtkill(args: &[&str], root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wtkill"))
        .args(args)
        .arg(root)
        .output()
        .unwrap()
}

fn json(f: &Fixture, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["--list", "--json"];
    args.extend_from_slice(extra);
    let out = wtkill(&args, &f.root);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice::<Value>(&out.stdout)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn by<'a>(rows: &'a [Value], name: &str) -> &'a Value {
    rows.iter()
        .find(|r| Path::new(r["path"].as_str().unwrap()).file_name().unwrap() == name)
        .unwrap_or_else(|| panic!("{name} missing"))
}

#[test]
fn list_json_reports_every_state() {
    let f = Fixture::standard();
    let rows = json(&f, &[]);
    assert_eq!(rows.len(), 9);
    for (name, state) in [
        ("app-done", "done"),
        ("app-pushed", "pushed"),
        ("app-feat x+1", "pushed"),
        ("app-dirty", "risky"),
        ("app-local", "risky"),
        ("app-broken", "broken"),
        ("orphan-dir", "orphan"),
        ("notes.md", "orphan"),
        ("clone", "clone"),
    ] {
        assert_eq!(by(&rows, name)["state"], state, "{name}");
    }
    assert_eq!(by(&rows, "app-feat x+1")["branch"], "feat/x");
    assert_eq!(by(&rows, "app-dirty")["dirty_count"], 2);
    assert_eq!(by(&rows, "app-local")["local_only_commits"], 2);
    assert_eq!(by(&rows, "app-done")["kind"], "worktree");
    assert!(by(&rows, "app-done")["upstream"]["gone"].as_bool().unwrap());
    for r in &rows {
        assert!(r["size_bytes"].as_u64().unwrap() > 0, "{r}");
        for k in ["main_repo", "size_partial", "last_commit_unix", "error"] {
            assert!(r.get(k).is_some(), "missing {k} in {r}");
        }
    }
}

#[test]
fn list_table_has_header_and_rows() {
    let f = Fixture::standard();
    let out = wtkill(&["--list"], &f.root);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].contains("SIZE") && lines[0].contains("STATE"),
        "{text}"
    );
    assert_eq!(lines.len(), 10, "{text}");
    assert!(text.contains("proj/.worktree/app-feat x+1"), "{text}");
}

#[test]
fn fetch_flag_marks_gone() {
    let f = Fixture::standard();
    // Squash-merge `pushed` on the remote and delete its branch, without
    // fetching in the main repo afterwards.
    f.git(&f.main, &["merge", "-q", "--squash", "pushed"]);
    f.git(&f.main, &["commit", "-q", "-m", "squash: pushed"]);
    f.git(&f.main, &["push", "-q", "origin", "main"]);
    f.git(&f.remote, &["branch", "-q", "-D", "pushed"]);
    assert_eq!(by(&json(&f, &[]), "app-pushed")["state"], "pushed");
    assert_eq!(by(&json(&f, &["--fetch"]), "app-pushed")["state"], "done");
}

#[test]
fn missing_root_exits_2() {
    let f = Fixture::standard();
    let out = wtkill(&["--list"], &f.root.join("nope"));
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn json_without_list_is_usage_error() {
    let f = Fixture::standard();
    assert!(!wtkill(&["--json"], &f.root).status.success());
}

#[test]
fn no_root_scans_current_directory_only() {
    let f = Fixture::standard();
    let outside = f.root.join("elsewhere/.worktree/stray-dir");
    std::fs::create_dir_all(&outside).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_wtkill"))
        .args(["--list", "--json"])
        .current_dir(f.root.join("proj"))
        .env("HOME", f.tmp.path()) // never fall back to the real home
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: Vec<Value> = serde_json::from_slice::<Value>(&out.stdout)
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 9, "{rows:#?}");
    assert!(
        rows.iter()
            .all(|r| !r["path"].as_str().unwrap().contains("stray-dir"))
    );
}

#[test]
fn too_old_git_is_rejected() {
    let f = Fixture::standard();
    let bin = f.tmp.path().join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("git");
    std::fs::write(&fake, "#!/bin/sh\necho 'git version 2.30.0'\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_wtkill"))
        .args(["--list"])
        .arg(&f.root)
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("2.38") && err.contains("2.30.0"), "{err}");
}
