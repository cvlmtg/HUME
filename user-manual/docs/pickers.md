# Fuzzy Finder

A modal panel for jumping straight to a file or an open buffer, without leaving the
keyboard: type a few characters, watch the list narrow, press `Enter` to go there.

## Setup

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers")
```

Requires `core:stdlib` loaded first.

## Picking files

`z f` opens a file picker scoped to HUME's working directory (`:pwd`; change it with
`:cd`):

- **Inside a git repository**, it lists every file `git` knows about: tracked files plus
  untracked ones that aren't ignored. It reads git's index directly, so it stays fast
  even in huge repos.
- **Outside a git repository**, it lists files with [`fd`](https://github.com/sharkdp/fd)
  (or Debian's `fdfind` package) if you have it installed.
- If neither applies, the picker tells you to install `fd`.

Selecting a tracked-but-deleted file (removed from disk without `git rm`) fails to open:
git's index doesn't know it's gone.

## Picking buffers

`z b` opens a picker over every open buffer. Each row shows the full path, not just the
filename, so two open files that share a name, like two different `mod.rs` files, stay
distinct.

## Picking modified files

`z m` opens a picker over every file with staged or unstaged changes, as reported by
`git status`. Each row shows the path and its two-letter status (`M ` staged, ` M`
unstaged, `??` untracked, and so on). Selecting a row opens that file, whichever
subdirectory `:pwd` points at.

Outside a git repository, `z m` shows a status message instead of opening a picker. In a
clean tree (nothing changed), the picker opens with no rows.

Untracked files are included by default, each as its own row. To leave them out, configure
the plugin when loading it:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers" #:config (hash 'untracked #f))
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
| `Down` / `Ctrl-n`            | Move selection down |
| `Up` / `Ctrl-p`               | Move selection up |
| `PageDown` / `PageUp`        | Page the list |
| `Ctrl-d` / `Ctrl-u`           | Move selection by half a page |
| `Enter` / `Ctrl-o`           | Open the selected item in the current pane |
| `Ctrl-t`                    | Open the selected item in a new tab |
| `Ctrl-v`                    | Open the selected item in a side-by-side split |
| `Ctrl-s`                    | Open the selected item in a stacked split |
| `Esc`                       | Dismiss without opening anything |
