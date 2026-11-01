use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use wtkill::app::{App, Command};
use wtkill::model::Update;
use wtkill::{git, list, pipeline, ui};

/// Find git worktrees under `.worktree/` directories and delete finished ones.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Directories to scan [default: current directory]
    roots: Vec<PathBuf>,
    /// Run `git fetch --prune` once per main repo before computing status
    #[arg(long)]
    fetch: bool,
    /// Print the table to stdout and exit (no TUI)
    #[arg(long)]
    list: bool,
    /// With --list, print JSON instead of a table
    #[arg(long, requires = "list")]
    json: bool,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match git::git_version() {
        None => {
            eprintln!("wtkill: git not found on PATH");
            return ExitCode::from(1);
        }
        Some(v) if v < git::MIN_GIT => {
            let (a, b, _) = git::MIN_GIT;
            eprintln!(
                "wtkill: git {a}.{b} or newer is required (found {}.{}.{})",
                v.0, v.1, v.2
            );
            return ExitCode::from(1);
        }
        Some(_) => {}
    }
    let roots = if cli.roots.is_empty() {
        match std::env::current_dir() {
            Ok(cwd) => vec![cwd],
            Err(e) => {
                eprintln!("wtkill: cannot read current directory ({e}); pass a ROOT");
                return ExitCode::from(2);
            }
        }
    } else {
        cli.roots
    };
    let mut canonical = Vec::new();
    for r in &roots {
        match r.canonicalize() {
            Ok(p) if p.is_dir() => canonical.push(p),
            _ => {
                eprintln!("wtkill: {}: not a directory", r.display());
                return ExitCode::from(2);
            }
        }
    }

    if cli.list {
        let (tx, rx) = mpsc::channel();
        let handle = pipeline::start(canonical.clone(), cli.fetch, tx);
        let mut app = App::new(canonical);
        for update in rx {
            let done = update == Update::ScanDone;
            app.apply(update);
            if done {
                break;
            }
        }
        let _ = handle.join();
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&list::json(&app)).unwrap_or_default()
            );
        } else {
            print!("{}", list::table(&app, now_unix()));
        }
        return ExitCode::SUCCESS;
    }

    match run_tui(canonical, cli.fetch) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wtkill: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_tui(roots: Vec<PathBuf>, fetch: bool) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel();
    pipeline::start(roots.clone(), fetch, tx.clone());
    let cwd = std::env::current_dir()?;
    let mut app = App::new(roots.clone());
    let mut deletes = Vec::new();
    let mut tick = 0usize;
    // ratatui::init installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = (|| -> std::io::Result<()> {
        loop {
            while let Ok(update) = rx.try_recv() {
                app.apply(update);
            }
            terminal.draw(|f| ui::render(f, &app, now_unix(), tick))?;
            tick = tick.wrapping_add(1);
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            for command in app.handle_key(key) {
                match command {
                    Command::Delete {
                        ids,
                        action,
                        strict,
                    } => {
                        let rows = ids
                            .iter()
                            .filter_map(|id| app.row(*id))
                            .map(|r| (r.entry.clone(), r.git.clone().and_then(Result::ok)))
                            .collect();
                        deletes.push(pipeline::spawn_delete(
                            rows,
                            action,
                            strict,
                            roots.clone(),
                            cwd.clone(),
                            tx.clone(),
                        ));
                    }
                    Command::Fetch => {
                        let entries = app.rows.iter().map(|r| r.entry.clone()).collect();
                        let tx = tx.clone();
                        thread::spawn(move || pipeline::inspect_groups(entries, true, &tx));
                    }
                    Command::Quit => return Ok(()),
                }
            }
        }
    })();
    ratatui::restore();
    if deletes.iter().any(|h| !h.is_finished()) {
        eprintln!("wtkill: waiting for running deletions to finish…");
    }
    for handle in deletes {
        let _ = handle.join();
    }
    result
}
