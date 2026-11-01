# Changelog

## 0.1.0

First release.

- Scan for `.worktree/` folders (default: the current directory) and list each
  entry's disk usage and git state: done, pushed, risky, orphan, broken, clone.
- Squash- and rebase-merged PRs count as done (content-exact `git merge-tree`
  check).
- Git-aware delete (`git worktree remove` + prune, optional branch delete),
  re-checked just before acting; No/Yes buttons, and a typed `yes` for risky
  rows.
- `--list` / `--json` output and opt-in `--fetch`.
- Requires git 2.38 or newer.
