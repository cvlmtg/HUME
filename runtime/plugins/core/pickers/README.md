# core:pickers

Fuzzy file, buffer, and git-modified-file pickers — `git`/`fd`-backed file
finder, a buffer switcher, and a `git status`-backed modified-file finder,
built on HUME's generic picker widget (see
[docs/FUZZY-FINDERS.md](../../../../docs/FUZZY-FINDERS.md)).

## Usage

Loads eagerly — it binds keys, which must be live from the first keystroke,
not deferred to a first-use trigger:

```scheme
(load-plugin "core:pickers")
```

## Keys

| Key   | Command                | Effect                                       |
|-------|-------------------------|-----------------------------------------------|
| `g f` | `picker-files`          | Fuzzy-pick a file in the current tree and open it |
| `g b` | `picker-buffers`        | Fuzzy-pick an open buffer and switch to it    |
| `g m` | `picker-git-modified`   | Fuzzy-pick a file with staged or unstaged git changes and open it |

Inside an open picker: type to filter, `Up`/`Down`/`Ctrl+p`/`Ctrl+n` move the
selection, `PageUp`/`PageDown` page it, `Backspace` edits the query, `Enter`
accepts, `Esc` dismisses.

## File source

`picker-files` picks its source per invocation (not at load time, so `:cd`
re-scopes it):

1. Inside a git work tree — `git ls-files -z --cached --others --exclude-standard`
   (reads the index, no filesystem walk; includes untracked-but-not-ignored
   files).
2. Otherwise, if `fd` (or Debian's `fdfind`) is installed — `fd --type f -0`.
3. Otherwise, an error naming `fd` as the thing to install.

`git ls-files --cached` can list a file that was deleted from disk without
`git rm`; selecting one surfaces an error when the picker tries to open it.

## Buffers

`picker-buffers` lists every open buffer, showing each one's path relative to
the editor's working directory (or its buffer name, for pathless buffers like
`*scratch*`) — bare filenames would be ambiguous whenever two open buffers
share a basename (two `mod.rs` files, say).

## Git-modified files

`picker-git-modified` runs `git status --porcelain -z --no-renames
--untracked-files=<mode>` (a fast, local, small-output command, so this is a
synchronous spawn — not the streaming source `picker-files` uses) and lists
every entry exactly as git prints it: the two-letter status code (`M `,
`A `, ` M`, `??`, …) followed by the path, relative to the repo root. `-z`
avoids git's C-quoting of paths with whitespace or non-ASCII; `--no-renames`
guarantees one field per entry (a rename otherwise prints as two NUL-separated
fields under `-z`, which would parse as a spurious extra row).

Because rows are repo-root-relative but `open-buffer!` resolves a relative
path against the editor's cwd (`:pwd`), the plugin resolves the selected
entry against the repo root (`git rev-parse --show-toplevel`) at accept time
— so `g m` opens the right file even when `:pwd` is a subdirectory of the
repo.

A clean tree still opens the picker, just with no rows. A cwd outside any
git repository (or a failed `git status`) surfaces as a status message
instead.

Since this is a synchronous spawn, the default `#t` walks every untracked
directory fully (`--untracked-files=all`) before `g m` opens — a large
un-ignored directory (no `.gitignore` entry) makes the editor pause for that
walk. Set `"untracked"` to `#f` to skip it.

### Config

| Key            | Value        | Effect                                    |
|----------------|--------------|--------------------------------------------|
| `"untracked"` | `#t` (default) | Untracked files are listed (`--untracked-files=all`, one row per file — never collapsed to a directory row, which a file picker couldn't usefully open). |
| `"untracked"` | `#f`          | Untracked files are excluded (`--untracked-files=no`). |

```scheme
(load-plugin "core:pickers" #:config (hash "untracked" #f))
```
