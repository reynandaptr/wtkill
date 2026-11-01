//! Background work: discovery, sizes, git inspection, deletes. Everything
//! reports back over an `mpsc` channel as `Update`s.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use rayon::prelude::*;

use crate::delete::{Action, delete_entry};
use crate::gitinfo;
use crate::model::{Entry, GitInfo, Kind, Update};
use crate::scan::discover;
use crate::size::disk_usage;

/// Discover entries, then size and inspect them. Sends `ScanDone` once every
/// entry has its size and (for worktrees) its git status.
pub fn start(roots: Vec<PathBuf>, fetch: bool, tx: Sender<Update>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut worktrees = Vec::new();
        rayon::scope(|s| {
            discover(&roots, |e| {
                let (id, path) = (e.id, e.path.clone());
                if e.kind == Kind::Worktree {
                    worktrees.push(e.clone());
                }
                let _ = tx.send(Update::Found(e));
                let tx = tx.clone();
                s.spawn(move |_| {
                    let _ = tx.send(Update::Size { id, size: disk_usage(&path) });
                });
            });
            inspect_groups(worktrees, fetch, &tx);
        });
        let _ = tx.send(Update::ScanDone);
    })
}

/// Inspect worktrees grouped by main repo, fetching each repo first when
/// `fetch` is set. A failed fetch is reported and inspection continues.
pub fn inspect_groups(entries: Vec<Entry>, fetch: bool, tx: &Sender<Update>) {
    let mut groups: BTreeMap<Option<PathBuf>, Vec<Entry>> = BTreeMap::new();
    for e in entries.into_iter().filter(|e| e.kind == Kind::Worktree) {
        groups.entry(e.main_repo.clone()).or_default().push(e);
    }
    groups.into_par_iter().for_each(|(main, entries)| {
        if fetch
            && let Some(main) = &main
                && let Err(error) = gitinfo::fetch(main) {
                    let _ = tx.send(Update::FetchFailed { main_repo: main.clone(), error });
                }
        entries.par_iter().for_each(|e| {
            let _ = tx.send(Update::Git { id: e.id, result: gitinfo::inspect(&e.path) });
        });
    });
}

/// Delete rows one after another on a background thread.
pub fn spawn_delete(
    rows: Vec<(Entry, Option<GitInfo>)>,
    action: Action,
    confirmed_strict: bool,
    roots: Vec<PathBuf>,
    cwd: PathBuf,
    tx: Sender<Update>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for (entry, git) in rows {
            let update = match delete_entry(&entry, git.as_ref(), confirmed_strict, action, &roots, &cwd) {
                Ok(note) => Update::Deleted { id: entry.id, note },
                Err(error) => Update::DeleteFailed { id: entry.id, error },
            };
            let _ = tx.send(update);
        }
    })
}
