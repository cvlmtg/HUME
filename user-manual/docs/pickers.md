# Fuzzy Finder

A panel for jumping to a file or an open buffer without leaving the keyboard: type a few characters, watch the list narrow, press `Enter` to go there.

## Setup

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers")
```

## Picking files

`z f` opens a file picker over HUME's working directory (`:pwd`; change it with `:cd`):

- **Inside a git repository**, it lists the files git knows about: tracked files, plus untracked ones that aren't ignored.
- **Outside a git repository**, it lists files with [`fd`](https://github.com/sharkdp/fd) (or Debian's `fdfind`) if you have it installed.
- If neither applies, an error message tells you to install `fd`.

A file deleted from disk without `git rm` still appears in the list, and fails to open.

## Picking buffers

`z b` opens a picker over every open buffer. Each row shows the full path, so two open files with the same name stay distinct.

## Picking modified files

`z m` opens a picker over every file with staged or unstaged changes, as `git status` reports them. Each row shows the path and its two-letter status (`M ` staged, ` M` unstaged, `??` untracked, and so on). Selecting a row opens that file, whichever subdirectory `:pwd` points at.

Outside a git repository, `z m` reports an error instead of opening a picker. In a clean tree, the picker opens with no rows.

Untracked files are included by default. To leave them out, configure the plugin when loading it:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers" #:config (hash 'untracked #f))
```

| Value | Effect |
|-------|--------|
| `#t` (default) | Untracked files are included |
| `#f` | Untracked files are left out |

## Keys

| Key | Effect |
|-----|--------|
| `z f` | Open the file picker |
| `z b` | Open the buffer picker |
| `z m` | Open the modified-files picker |

Once a picker is open:

| Key | Effect |
|-----|--------|
| Type | Filter the list |
| `Backspace` | Edit the query |
| `Down` / `Ctrl-n` | Move the selection down |
| `Up` / `Ctrl-p` | Move the selection up |
| `PageDown` / `PageUp` | Page the list |
| `Ctrl-d` / `Ctrl-u` | Move the selection by half a page |
| `Enter` / `Ctrl-o` | Open the selected item in the current pane |
| `Ctrl-t` | Open the selected item in a new tab |
| `Ctrl-v` | Open the selected item in a side-by-side split |
| `Ctrl-s` | Open the selected item in a stacked split |
| `Esc` | Close without opening anything |
