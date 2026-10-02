# core:pickers

Fuzzy file, buffer, and git-modified-file pickers, built on HUME's generic picker widget
with no native (Rust) picker definitions.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers" #:config (hash "untracked" #f))
```

- **Depends on:** `core:stdlib`: config validation calls `stdlib/config-boolean` at load
  time; `picker-files`/`picker-git-modified` call `stdlib/git-repo?`/`stdlib/git-toplevel`
  at dispatch time; all three pickers call `stdlib/buffer-actions`.
- **Activates on:** its own key bindings only. It has no `manifest.scm`, so it must be
  loaded eagerly (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Fuzzy Finder](https://cvlmtg.github.io/HUME/pickers.html) and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-pickers) for keys
  and config semantics.

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `"untracked"` | boolean | `#t` | `#t` passes `--untracked-files=all` to `git status`, which walks every untracked directory. `#f` passes `no`, which skips the walk and populates sooner. |

## Commands

| Key | Command | Effect |
|---|---|---|
| `z f` | `picker-files` | Fuzzy-pick a file in the current directory tree and open it |
| `z b` | `picker-buffers` | Fuzzy-pick an open buffer and switch to it |
| `z m` | `picker-git-modified` | Fuzzy-pick a file with staged or unstaged git changes and open it |

`pickers/files-picker-with` and `pickers/git-picker-with` are internal commands that the
two public ones call; see Design decisions.

The three keys sit under `z`. A picker opens a panel over the buffer and waits, the same
shape as `core:lsp`'s references list and code-action menu, also on `z`; `g` is left to
commands that name a place.

Every picker also binds `Ctrl-o`, `Ctrl-t`, `Ctrl-v` and `Ctrl-s` to open the selection in
the current pane, a new tab, a vertical split or a horizontal split. They come from
`core:stdlib`'s [picker buffer-placement](../stdlib/README.md#picker-buffer-placement).

## How it works

### File source

`picker-files` picks its source per invocation, not at load time, so `:cd` re-scopes it:

1. Inside a git work tree: `git ls-files -z --cached --others --exclude-standard`. It reads
   the index with no filesystem walk and includes untracked-but-not-ignored files.
2. Otherwise, if `fd` (or Debian's `fdfind`) is installed: `fd --type f -0`.
3. Otherwise, an error naming `fd` as the thing to install.

`git ls-files --cached` can list a file that was deleted from disk without `git rm`;
selecting one surfaces an error when the picker tries to open it.

### Buffers

`picker-buffers` shows `buffer-display-path`, falling back to the buffer's name
(`*scratch*`, etc.) for a pathless one; see the manual's
[Picking buffers](https://cvlmtg.github.io/HUME/pickers.html#picking-buffers). `(buffers)`
hands each entry as a [pane-less pane value](../README.md#pane-values-vs-pane-less-values),
which is used directly as the picker payload.

The switch targets whichever pane is focused when the pick is made (`(focused-pane)`).
This is an exception to the index's
[capture the target](../README.md#capture-the-target-dont-re-read-focus) pattern: the
selection callback runs synchronously with Enter or `Ctrl-o`, so the picker's origin pane
and the focused pane are the same.

### Git-modified files

`picker-git-modified` resolves the repo root with `stdlib/git-toplevel` at dispatch and
raises an error before a picker opens when the cwd is outside any git repository.

#### Source

`git status --porcelain -z --no-renames --untracked-files=<mode>` runs in the background
through `spawn-async!`, not the line-streaming source `picker-files` uses, since this
picker needs the whole output parsed at once. The picker opens empty and marked pending,
then populates in one batch when `git status` completes. Each row is the two-letter status
code (`M `, `A `, ` M`, `??`, …) followed by the path, relative to the repo root.

| Flag | Why |
|---|---|
| `-z` | Avoids git's C-quoting of paths with whitespace or non-ASCII |
| `--no-renames` | Gives one field per entry; a rename otherwise prints as two NUL-separated fields under `-z` and parses as an extra row |
| `--untracked-files=<mode>` | `all` (default) walks every untracked directory fully and can be slow on a large un-ignored directory; `no` skips the walk. The editor does not block either way |

#### Parsing

`-z` terminates every entry including the last, so splitting on NUL leaves a trailing `""`
fragment, the whole output for a clean tree. The parser filters empty fragments, so a clean
tree (exit 0, empty stdout) yields an empty item list and pushes as a no-op.

#### Opening

Rows are repo-root-relative but `open-buffer!` resolves a relative path against the
editor's cwd. The handler joins the selected row onto the root resolved at dispatch, so a
selection opens the right file when `:pwd` is a subdirectory of the repo.

#### Failure and cancellation

A `git status` failure logs `'error` and calls `picker-close!` with this picker's token.
That is a no-op if the picker has already closed or been replaced, so a slow failure
cannot tear down a picker the user opened since. Dismissing without selecting cancels the
outstanding `git status` job.

## Design decisions

- **Accepting a row switches the focused pane.** Every picker's handler wraps `open-buffer!`
  in `switch-to-buffer!`.
- **All three pickers pass `#:actions (call! "stdlib/buffer-actions" handler)`.** That gives
  each the `Ctrl-o`/`Ctrl-t`/`Ctrl-v`/`Ctrl-s` placements. `buffer-actions` needs the same
  proc twice, once as `on-select` and once wrapped for each placement, so each picker's
  handler is a named `define` (`pickers/open-file!`, `pickers/switch-to-buffer!`, and the
  git picker's own `handler`) and not an inline lambda.
- **`picker-files` and `picker-git-modified` each split into a public command and an
  internal `pickers/*-with` command.** The internal one takes the git/fd probe result (or
  repo root) as an argument. A test drives each branch (repo or no repo, `fd` present or
  absent) through `call!` without changing `PATH` or building a git sandbox.
