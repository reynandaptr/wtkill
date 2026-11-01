# wtkill — design

Date: 2026-11-01
Status: approved in conversation, pending written-spec review

## Purpose

A terminal UI, in the spirit of `gdu` / `npkill`, that finds every `.worktree/`
folder under a root, shows each worktree inside with its disk usage and git
state, and deletes the ones that are done — **safely and git-aware**.

Context: the user's convention is to create git worktrees at
`<start-dir>/.worktree/<repo>-<branch>`. These accumulate; on 2026-11-01 there
were 25 `.worktree` folders totalling ~55 GB under `~/Development/sourcecodes`
(one alone was 38 GB). Plain `rm -rf` leaves stale worktree metadata in the main
repo and risks losing uncommitted or unpushed work.

**Success:** in one run, the user can see which worktrees take space and which
are safe, and remove the finished ones, leaving each main repo's
`git worktree list` clean.

### Decisions made with the user

- Deletion is **git-aware** (`git worktree remove` + prune, optional branch delete).
- Stack: **Rust + ratatui**.
- "Done" detection uses **local git only** (no GitHub API). The only network
  access is an explicit, opt-in `git fetch --prune`.

### Non-goals

- No GitHub / `gh` integration (PR state).
- No scanning for general disk hogs (`node_modules`, `target`) — `.worktree` only.
- No config file. CLI flags only.
- Linux (WSL) is the target; other platforms are not tested.

## CLI

```
wtkill [OPTIONS] [ROOT]...

ROOT              One or more directories to scan. Default: the current directory
--fetch           Run `git fetch --prune` once per main repo before computing status
--list            Print the table to stdout and exit (no TUI)
--json            With --list, print JSON instead of a table
```

## Architecture

Results stream in: discovery is fast, so rows appear immediately; size and git
status for each row are computed on a background worker pool and sent to the UI
thread over a channel. The `git` CLI is invoked as a subprocess (no libgit2).

```
main (clap) ──► scan::discover(roots) ──► Vec<Entry> (kind + path, no size/status)
                     │
                     ▼
             worker pool (rayon) ── per entry ──► size::disk_usage(path)
                     │                         └► gitinfo::inspect(entry)
                     ▼
               mpsc::Sender<Update> ──► app::App (pure state machine) ──► ui::render
                                             ▲
                     delete::run (bg thread) ┘  Update::Deleted / Update::Failed
```

### Modules

Each module has one job and can be tested on its own.

| Module    | Responsibility | Depends on |
|-----------|----------------|------------|
| `scan`    | Walk roots, find `.worktree` dirs, classify each child into an `Entry`. | std fs |
| `size`    | Disk usage of a path: sum of `st_blocks * 512`, hard links counted once (by `(dev, ino)`), symlinks not followed. Parallel walk. | rayon / jwalk |
| `gitinfo` | Run `git` commands for a worktree and parse output into `GitInfo`; compute `State`. Parsing functions are pure and take strings. | `git` binary |
| `delete`  | Perform a delete action for an entry, with guardrails. Returns bytes freed or an error with stderr. | `git` binary, std fs |
| `app`     | Application state: entries, selection, marks, sort, filter, modal (confirm prompts), freed counter. Handles `Update` and key events. No I/O. | — |
| `ui`      | Render `App` with ratatui. No state mutation. | ratatui |
| `list`    | `--list` / `--json` output. | serde_json |
| `main`    | Parse CLI, wire threads, run the terminal loop (crossterm). | clap, crossterm |

## Discovery (`scan`)

- Walk each root recursively. Do not follow symlinks.
- Skip descending into directories named `.git`, `node_modules`, `target`.
- When a directory named `.worktree` is found, list its **immediate children**;
  do not recurse further inside `.worktree` (worktrees are never searched for
  nested `.worktree` dirs).
- Each child becomes an `Entry` with one `Kind`:
  - **Worktree** — child is a directory containing a `.git` *file* whose
    `gitdir:` target exists. The main repo is resolved from
    `git -C <child> rev-parse --path-format=absolute --git-common-dir`
    (parent of the common dir when it ends in `.git`).
  - **Broken** — child has a `.git` file but the `gitdir:` target does not exist.
    The main repo path is derived from the `gitdir:` value
    (`<main>/.git/worktrees/<name>` → `<main>`) and recorded only if that
    directory exists.
  - **Orphan** — anything else: a directory with no `.git`, or a loose file
    (e.g. `TICKET-9-spec.md`).
  - **Clone** — child is a directory containing a `.git` *directory* (a full
    clone, not a worktree). Listed with its size, but **never deleted** by
    wtkill, since it may hold unpushed work that worktree checks can't see.
- Empty `.worktree` dirs produce no rows.

Display path: relative to the root it was found under.

## Size (`size`)

- Disk usage, matching `du` semantics: allocated blocks (`st_blocks * 512`),
  each `(st_dev, st_ino)` counted once, symlinks counted as the link itself and
  never followed.
- Unreadable subpaths are skipped and the row is flagged `size_partial = true`
  (shown as `≥ 1.2 GB`).

## Git status (`gitinfo`)

For **Worktree** entries only. All commands run with `git -C <worktree>`:

| Field | Command | Parse |
|-------|---------|-------|
| branch | `git symbolic-ref --short -q HEAD` | empty / exit 1 → detached (show short SHA from `rev-parse --short HEAD`) |
| dirty files | `git status --porcelain=v1 -z` | count of entries; keep first 20 paths for detail pane |
| local-only commits | `git rev-list --count HEAD --not --remotes` | integer; also `git log --format=%s -n 10 HEAD --not --remotes` for detail pane |
| upstream | `git for-each-ref --format='%(upstream:short)%00%(upstream:track)' refs/heads/<branch>` | none / tracking (ahead N, behind M) / **gone** (`[gone]`) |
| last commit | `git log -1 --format=%ct` | unix time → age |

If any command fails, the entry is marked with an error (stderr kept) and its
state is `Unknown` — treated like Risky for confirmation purposes.

### State

Computed in a pure function from `Kind` + `GitInfo`:

| State | Rule |
|-------|------|
| 🔴 **Risky** | Worktree with dirty files > 0 **or** local-only commits > 0 that are not squash/rebase-merged (see below) |
| 🟢 **Done** | Worktree, clean, upstream **gone**, and either 0 local-only commits or its changes already landed on the remote's default branch |
| 🟡 **Pushed** | Worktree, clean, 0 local-only commits, upstream exists or no upstream configured |
| ⚪ **Orphan** | Kind Orphan |
| ⚪ **Broken** | Kind Broken |
| ⚪ **Clone** | Kind Clone |
| ❔ **Unknown** | git inspection failed |

**Squash-merge detection** (added during implementation): after a GitHub
squash merge deletes the PR branch, the branch's commits are no longer
reachable from any remote ref, so they count as local-only. When the upstream
is gone and local-only > 0, wtkill checks the remote's default branch
(`<remote>/HEAD`, else `<remote>/main`, else `<remote>/master`) with
`git merge-tree --write-tree <base> HEAD`: if merging HEAD into it would leave
its tree unchanged, every change is already there and the state is Done. This
is content-exact (whitespace, merge commits and conflict resolutions count);
a conflict means not merged. (Patch-id based `git cherry` checks were tried
first and rejected in review: they ignore whitespace and skip merge commits.)

Note: "no upstream but 0 local-only commits" means every commit is reachable
from some remote ref, so nothing is lost — hence Pushed, not Risky.

### Fetch

`--fetch` at startup, or `f` in the TUI, runs `git -C <main> fetch --prune`
once per **distinct main repo**, then recomputes git status for that repo's
entries. Fetches run on the worker pool; a failed fetch marks the affected rows
with the error but does not block others. Without fetch, wtkill does no network
access.

## TUI (`app` + `ui`)

```
 wtkill  ~/Development/sourcecodes        found 25 · 55.2 GB · freed 0 B   [scanning…]
 ─────────────────────────────────────────────────────────────────────────────────────
   SIZE     STATE        WORKTREE                                     BRANCH            AGE
 ▶ 38.0 GB  🟡 pushed    personal/vault/.worktree/vault-folders       folders           12d
   3.9 GB   🔴 risky 3✎  acme/.worktree/api-ci-env-gate               ci-env-gate        4d
   1.2 GB   🟢 done      acme/billing/…/billing-feat-refunds          feat/refunds      30d
   12 KB    ⚪ orphan    acme/web/.worktree/TICKET-9-spec.md           —                  —
 ─────────────────────────────────────────────────────────────────────────────────────
 [detail pane: main repo, full path, dirty files, local-only commit subjects, errors]
 ↑↓/jk move · space mark · d delete · D delete+branch · f fetch · s sort · / filter · q quit
```

- Rows appear as discovered; size shows a spinner until computed; state shows
  `…` until git status is known.
- Default sort: size descending. `s` cycles size → age → state.
- `/` opens a path substring filter; `Esc` clears it.
- `space` toggles a mark; `d`/`D` act on all marked rows, or the cursor row if
  none are marked.
- Detail pane always shows the cursor row.
- `q` / `Ctrl-C` quit. Quitting while deletions are in progress asks for
  confirmation; in-flight deletions are allowed to finish.

## Deletion (`delete`)

| Kind / State | `d` | `D` (adds) | Confirmation |
|---|---|---|---|
| Worktree Done / Pushed | `git -C <main> worktree remove <path>` then `git -C <main> worktree prune` | `git -C <main> branch -D <branch>` | No/Yes buttons |
| Worktree Risky / Unknown | `git -C <main> worktree remove --force <path>` then prune | `git -C <main> branch -D <branch>` | Red prompt listing N dirty files and M local-only commits; user must type `yes` |
| Broken | recursive delete of the folder; then `git -C <main> worktree prune` if main exists | — (no branch delete) | No/Yes buttons |
| Orphan | recursive delete (file or folder) | — | No/Yes buttons |
| Clone | refused: detail pane says "full git clone — delete manually" | — | — |

**Re-check at delete time.** Just before removing a worktree, wtkill inspects
it again and aborts that row ("re-confirm") if HEAD moved, it gained dirty
files, or — after a No/Yes buttons prompt — its state became Risky/Unknown. `--force` is
used only when the user answered the strict `yes` prompt. The branch delete is
`git update-ref -d refs/heads/<b> <oid>` (only if the tip is unchanged) and is
skipped when another worktree has the branch checked out. Pressing `f` resets
worktree states to pending until the post-fetch inspection arrives.

When several rows are marked, one prompt covers all of them; if any marked row
is Risky/Unknown, the stricter `yes` prompt is used and lists the risky rows.

- Branch delete is skipped (with a note in the detail pane) when the branch is
  checked out in another worktree, or the worktree was detached.
- Locked worktrees (`git worktree lock`) fail `worktree remove`; the row shows
  the error. wtkill does not unlock.
- Deletions run on a background thread. The row shows `deleting…`, then is
  removed, and **freed** increases by the row's measured size.

### Guardrails

Checked before every delete; a violation aborts that row with an error:

1. The canonical path's parent directory must be named `.worktree`.
2. The path must be inside one of the scan roots.
3. The path must not contain the process's current working directory.
4. Symlinks are never followed: a symlinked entry is removed as a link only.

## Error handling

- Any failing `git` command or fs operation marks only that row with ✖; stderr
  (or the io error) is shown in the detail pane.
- One failure never stops the scan, other workers, or other deletions.
- If `git` is not on `PATH`, wtkill exits at startup with a clear message.
- Terminal state is always restored on exit, including on panic (panic hook).

## Non-interactive mode (`list`)

`wtkill --list` waits for the scan to finish and prints a table with the same
columns (size, state, path, branch, age) sorted by size. `--json` prints an
array of objects with all fields (path, kind, main_repo, size_bytes,
size_partial, state, branch, dirty_count, local_only_commits, upstream,
last_commit_unix, error). Exit code 0 on success.

## Testing

- **Unit (`gitinfo`)**: parsing functions for each command's output, including
  detached HEAD, `[gone]`, ahead/behind, empty output; `State` rules table.
- **Unit (`app`)**: key handling, sort, filter, marking, prompt selection
  (y/N vs `yes`), freed counter, applying `Update`s.
- **Unit (`delete` guardrails)**: paths outside `.worktree`, outside roots,
  containing cwd.
- **Integration**: build real temporary repos (`tempfile`) with a bare
  "remote", a main clone, and worktrees under `.worktree/` in every state —
  done (remote branch deleted + fetched), pushed, dirty, local-only commits,
  broken (gitdir removed), orphan dir, orphan file. Assert `--list --json`
  output, then run each delete action and assert the folder is gone,
  `git worktree list` no longer shows it, and `branch -D` happened for `D`.
- **Render**: ratatui `TestBackend` snapshot of the main screen with a fixed
  `App` state.

## Repo

- Location: `~/Development/sourcecodes/personal/wtkill`.
- GitHub repo `reynandaptr/wtkill`; `main` holds this spec; feature
  work happens in worktrees under
  `~/Development/sourcecodes/personal/.worktree/wtkill-<branch>` branched from
  `origin/main`.
- Binary installs via `cargo install --path .` (the user runs it).
