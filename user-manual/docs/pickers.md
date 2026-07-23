# Fuzzy Finder

A modal panel for jumping straight to a file or an open buffer, without leaving the
keyboard: type a few characters, watch the list narrow, `Enter` to go there.

## Setup

```scheme
(load-plugin "core:pickers")
```

Must be loaded eagerly — it binds `g f` and `g b`, which need to be live from the first
keystroke.

## Picking files

`g f` opens a file picker scoped to HUME's working directory (`:pwd`; change it with
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

`g b` opens a picker over every open buffer, showing each one's path (relative to `:pwd`
when possible) rather than just its filename — so two open files that happen to share a
name, like two different `mod.rs` files, show up as distinct, disambiguated rows.

## Keys

| Key   | Effect |
|-------|--------|
| `g f` | Open the file picker |
| `g b` | Open the buffer picker |

Once a picker is open:

| Key                        | Effect |
|-----------------------------|--------|
| Type                        | Filter the list |
| `Backspace`                 | Edit the query |
| `Down` / `Ctrl+n`            | Move selection down |
| `Up` / `Ctrl+p`               | Move selection up |
| `PageDown` / `PageUp`        | Page the list |
| `Enter`                     | Open the selected item |
| `Esc`                       | Dismiss without opening anything |
