//! Disk usage with `du` semantics: allocated blocks, hard links counted once,
//! symlinks counted as the link itself and never followed.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crate::model::Size;

pub fn disk_usage(path: &Path) -> Size {
    let seen = Mutex::new(HashSet::new());
    let partial = AtomicBool::new(false);
    let bytes = walk(path, &seen, &partial);
    Size { bytes, partial: partial.load(Ordering::Relaxed) }
}

fn walk(path: &Path, seen: &Mutex<HashSet<(u64, u64)>>, partial: &AtomicBool) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        partial.store(true, Ordering::Relaxed);
        return 0;
    };
    let mut own = meta.blocks() * 512;
    // Directories cannot be hard-linked; only dedupe other inodes.
    if !meta.is_dir() && meta.nlink() > 1 && !seen.lock().unwrap().insert((meta.dev(), meta.ino())) {
        own = 0;
    }
    if !meta.is_dir() {
        return own;
    }
    let Ok(rd) = fs::read_dir(path) else {
        partial.store(true, Ordering::Relaxed);
        return own;
    };
    let children: Vec<PathBuf> = rd
        .filter_map(|e| match e {
            Ok(e) => Some(e.path()),
            Err(_) => {
                partial.store(true, Ordering::Relaxed);
                None
            }
        })
        .collect();
    own + children.par_iter().map(|c| walk(c, seen, partial)).sum::<u64>()
}
