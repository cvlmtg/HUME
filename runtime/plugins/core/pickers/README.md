# core:pickers

Fuzzy file and buffer pickers — `git`/`fd`-backed file finder, plus a buffer
switcher, built on HUME's generic picker widget (see
[docs/FUZZY-FINDERS.md](../../../../docs/FUZZY-FINDERS.md)).

## Usage

Loads eagerly — it binds keys, which must be live from the first keystroke,
not deferred to a first-use trigger:

```scheme
(load-plugin "core:pickers")
```

## Keys

| Key   | Command          | Effect                                       |
|-------|------------------|-----------------------------------------------|
| `g f` | `picker-files`   | Fuzzy-pick a file in the current tree and open it |
| `g b` | `picker-buffers` | Fuzzy-pick an open buffer and switch to it    |

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
