use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use wtkill::size::disk_usage;

fn blocks(p: &Path) -> u64 {
    fs::symlink_metadata(p).unwrap().blocks() * 512
}

#[test]
fn counts_blocks_hardlinks_once_and_symlinks_as_links() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("d");
    fs::create_dir_all(d.join("sub")).unwrap();
    fs::write(d.join("big"), vec![7u8; 100_000]).unwrap();
    fs::hard_link(d.join("big"), d.join("sub/big-link")).unwrap();
    std::os::unix::fs::symlink("/usr", d.join("usr-link")).unwrap();
    let expected = blocks(&d) + blocks(&d.join("sub")) + blocks(&d.join("big")) + blocks(&d.join("usr-link"));
    let s = disk_usage(&d);
    assert_eq!(s.bytes, expected);
    assert!(!s.partial);
}

#[test]
fn single_file() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("notes.md");
    fs::write(&f, vec![1u8; 5000]).unwrap();
    assert_eq!(disk_usage(&f).bytes, blocks(&f));
}

#[test]
fn unreadable_subdir_is_partial() {
    if std::env::var("USER").as_deref() == Ok("root") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let locked = tmp.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("x"), "x").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let s = disk_usage(tmp.path());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(s.partial);
}
