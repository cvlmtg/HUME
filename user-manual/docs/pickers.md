# Fuzzy Finder

A modal panel for jumping straight to a file or an open buffer, without leaving the
keyboard: type a few characters, watch the list narrow, `Enter` to go there.

## Setup

```scheme
(declare-plugin "core:stdlib")
(load-plugin "core:pickers")
```

Must be loaded eagerly — `z f` and `z b` are the only way to reach its commands, so declared lazily it would have no trigger to ever wake it up. `core:stdlib` only needs to be declared or loaded before it.

## Picking files

`z f` opens a file picker scoped to HUME's working directory (`:pwd`; change it with
`:cd`):

- **Inside a git repository**, it lists every file `git` knows about — tracked files plus
  untracked-but-not-ignored ones. This reads straight from git's index, so it stays fast
  even in huge repos.
- **Outside a git repository**, it lists files with [`fd`](https://github.com/sharkdp/fd)
  (or Debian's `fdfind` package) if you have it installed.
- If neither applies, the picker tells you to install `fd`.

Selecting a tracked-but-deleted file (removed from disk without `git rm`) fails to open —
git's index doesn't know it's gone.

## Picking buffers

`z b` opens a picker over every open buffer, showing each one's full path rather than just
its filename — so two open files that happen to share a name, like two different `mod.rs`
files, show up as distinct, disambiguated rows.

## Picking modified files

`z m` opens a picker over every file with staged or unstaged changes, as reported by
`git status` — the row shows the two-letter status (`M ` staged, ` M` unstaged, `??`
untracked, and so on) alongside the path, so you can tell at a glance what kind of change
each file has. Selecting a row opens that file, regardless of which subdirectory `:pwd`
currently points at.

A working directory outside any git repository shows a status message instead of opening a
picker. A clean tree (nothing changed) opens the picker with no rows.

By default, untracked files are included, each shown as its own row. Turn them off when
loading the plugin:

```scheme
(declare-plugin "core:stdlib")
(load-plugin "core:pickers" #:config (hash "untracked" #f))
```

| Value          | Effect |
|-----------------|--------|
| `#t` (default) | Untracked files are included. |
| `#f`           | Untracked files are left out entirely. |

## Keys

| Key   | Effect |
|-------|--------|
| `z f` | Open the file picker |
| `z b` | Open the buffer picker |
| `z m` | Open the modified-files picker |

Once a picker is open:

| Key                        | Effect |
|-----------------------------|--------|
| Type                        | Filter the list |
| `Backspace`                 | Edit the query |
| `Down` / `Ctrl+n`            | Move selection down |
| `Up` / `Ctrl+p`               | Move selection up |
| `PageDown` / `PageUp`        | Page the list |
| `Ctrl+d` / `Ctrl+u`           | Move selection by half a page |
| `Enter`                     | Open the selected item |
| `Esc`                       | Dismiss without opening anything |
