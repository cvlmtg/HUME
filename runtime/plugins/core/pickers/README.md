# core:pickers

Fuzzy file, buffer, and git-modified-file pickers, built on HUME's generic picker widget
with no native (Rust) picker definitions.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers" #:config (hash 'untracked #f))
```

- **Depends on:** `core:stdlib`, at load time for config validation and when a picker
  opens.
- **Activates on:** `load-plugin!`. Its `bind-key!` calls must run at startup, so it has
  no `manifest.scm` (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Fuzzy Finder](https://cvlmtg.github.io/HUME/pickers.html) and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-pickers).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'untracked` | boolean | `#t` | `#t` passes `--untracked-files=all` to `git status`, which walks every untracked directory and can be slow on a large un-ignored one. `#f` passes `no`, which skips the walk and populates sooner. The editor does not block either way. |

## Commands

| Key | Command | Effect |
|---|---|---|
| `z f` | `pickers/files` | Fuzzy-pick a file in the current directory tree and open it |
| `z b` | `pickers/buffers` | Fuzzy-pick an open buffer and switch to it |
| `z m` | `pickers/git-modified` | Fuzzy-pick a file with staged or unstaged git changes and open it |

The keys sit under `z` because a picker opens a panel and waits, the layout
[`core:lsp`'s default keys](../lsp/docs/architecture.md#default-keys) explain.

## How it works

Every picker takes `core:stdlib`'s [Picker keymap](../stdlib/README.md#picker-keymap)
around a handler that switches `(focused-pane)` to the picked buffer. Reading focus is that
keymap's contract: a placement key focuses the pane it creates before the handler runs. The
index's [capture the target](../README.md#capture-the-target-dont-re-read-focus) pattern
does not apply, since a binding runs before the next key is read.

`z f` and `z m` fill their picker from a command's NUL-separated output through
`picker-source-spawn!`, which shows the picker as pending until the command exits, stops
the command when the picker closes, and reports a failed exit with the command's stderr.

### File source

`pickers/files` picks its source per invocation, not at load time, so `:cd` re-scopes it:

1. Inside a git work tree: `git ls-files -z --cached --others --exclude-standard`. It reads
   the index with no filesystem walk and includes untracked-but-not-ignored files.
2. Otherwise, if `fd` (or Debian's `fdfind`) is installed: `fd --type f -0`.
3. Otherwise, an error naming `fd` as the thing to install.

`git ls-files --cached` can list a file that was deleted from disk without `git rm`;
selecting one surfaces an error when the picker tries to open it.

### Buffers

`pickers/buffers` shows `buffer-display-path`, falling back to the buffer's name
(`*scratch*`, etc.) for a pathless one; see the manual's
[Picking buffers](https://cvlmtg.github.io/HUME/pickers.html#picking-buffers). Each item's
payload is the [pane-less pane value](../README.md#pane-values-vs-pane-less-values)
`(buffers)` hands, which the handler switches to directly.

Rows take `:ls`'s scopes through the picker's `#:render`: an unsaved buffer takes the
warning scopes, otherwise the buffer the picker opened from, matched by `buffer-key`, is
bold. A buffer closed while the picker is open (`buffer-live?` is `#f`) keeps a plain row,
since `buffer-dirty?` raises for it.

### Git-modified files

`pickers/git-modified` resolves the repo root with `stdlib/git-toplevel` at dispatch and
raises an error before a picker opens when the cwd is outside any git repository.

#### Source

`git status --porcelain -z --no-renames --untracked-files=<mode>` fills the picker. Each
item is one entry: the two-letter status code (`M `, `A `, ` M`, `??`, …), a space, and
the path relative to the repo root.

| Flag | Why |
|---|---|
| `-z` | Avoids git's C-quoting of paths with whitespace or non-ASCII |
| `--no-renames` | Gives one field per entry; a rename otherwise prints as two NUL-separated fields under `-z` and lists its old path as an extra row |

#### Rows

The picker's `#:render` keeps each row's status visible and colors it. Each status letter
gets its own span, so the index (staged) letter and the worktree letter are colored apart,
as `git status` shows them. On the selected row a letter takes `ui.text.focus`, the row's
own selected style, since a theme's diff colors can match that row's background.

A row wider than the panel keeps its three status columns and drops the front of the path
behind `…` with `string-fit`, which never splits a character. Below four cells there is no
room for both, and the picker's own head cut takes over.

#### Opening

Rows are repo-root-relative but `open-buffer!` resolves a relative path against the
editor's cwd. The handler joins the selected row's path onto the root resolved at dispatch
before `open-buffer!`, so a selection opens the right file when `:pwd` is a subdirectory
of the repo.
