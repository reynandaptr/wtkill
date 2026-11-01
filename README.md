# wtkill

Like `gdu` / `npkill`, but only for git worktrees kept under `.worktree/`
directories. wtkill finds every `.worktree/` folder under a root, shows each
entry's disk usage and git state, and deletes the finished ones **git-aware**
(`git worktree remove` + `prune`), so main repos stay clean and uncommitted or
unpushed work isn't lost by accident.

## Install

Works on **macOS** and **Linux**. Requires **git 2.38 or newer** (check with
`git --version`; macOS users may need `brew install git`).

Homebrew (macOS and Linux):

```sh
brew install reynandaptr/tap/wtkill
```

Install script (macOS and Linux, installs into `~/.cargo/bin`):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/reynandaptr/wtkill/releases/latest/download/wtkill-installer.sh | sh
```

Prebuilt archives for each platform are on the
[Releases](https://github.com/reynandaptr/wtkill/releases) page.

From source (needs a Rust toolchain):

```sh
cargo install --git https://github.com/reynandaptr/wtkill
```

## Usage

```
wtkill [OPTIONS] [ROOTS]...

  [ROOTS]...  Directories to scan [default: current directory]
  --fetch     Run `git fetch --prune` once per main repo before computing status
  --list      Print the table to stdout and exit (no TUI)
  --json      With --list, print JSON instead of a table
```

Without `--fetch` (or the `f` key) wtkill never touches the network.

### Keys

| Key | Action |
|-----|--------|
| `↑` `↓` / `k` `j` | Move |
| `space` | Mark / unmark (deletes apply to all marked rows, or the cursor row) |
| `d` | Delete |
| `D` | Delete and also delete the local branch |
| `f` | `git fetch --prune` every main repo, then refresh states |
| `s` | Sort: size → age → state |
| `/` | Filter by path (`Enter` apply, `Esc` clear) |
| `q` / `Ctrl-C` | Quit (asks first while deletions are running) |

## States

| State | Meaning |
|-------|---------|
| 🟢 done | Clean, upstream branch gone, and nothing local-only — or its changes already landed on the remote's default branch (squash/rebase-merged PR) |
| 🟡 pushed | Clean and fully pushed; upstream still exists (PR probably open) |
| 🔴 risky | Dirty files, or commits that exist nowhere else. Deleting loses them |
| ⚪ orphan | Plain folder or loose file under `.worktree/` |
| ⚪ broken | Has a `.git` file but its main repo / worktree record is gone |
| ⚪ clone | A full git clone (own `.git` dir) — listed, **never deleted** |
| ❔ unknown | git inspection failed (treated like risky) |

## What delete does

| Entry | `d` | `D` adds | Prompt |
|-------|-----|----------|--------|
| done / pushed worktree | `git worktree remove <path>` + `git worktree prune` | delete the local branch (only if its tip is unchanged and no other worktree has it) | No/Yes buttons |
| risky / unknown worktree | `git worktree remove --force <path>` + prune | delete the local branch (only if its tip is unchanged and no other worktree has it) | type `yes` |
| broken | delete the folder, prune the main repo if it still exists | — | No/Yes buttons |
| orphan | delete the file or folder | — | No/Yes buttons |
| clone | refused — delete it manually | — | — |

The No/Yes prompt opens with **No** focused: press `Tab` (or `←`/`→`) to move
to **Yes**, then `Enter`. `Esc` or `n` cancels. There is no one-key yes.

A skipped branch delete (detached HEAD, branch checked out elsewhere) is shown
as a note, not an error.

### Guardrails

Before every delete wtkill refuses when:

1. the entry's parent directory isn't named `.worktree`,
2. the entry is outside the scanned roots, or
3. the entry contains the current working directory.

Symlinks are never followed while scanning, sizing, or deleting.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

Design: `docs/superpowers/specs/2026-11-01-wtkill-design.md`.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
