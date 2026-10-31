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
| `'untracked` | boolean | `#t` | `#t` passes `--untracked-files=all` to `git status`, which walks every untracked directory. `#f` passes `no`, which skips the walk and populates sooner. |

## Commands

| Key | Command | Effect |
|---|---|---|
| `z f` | `pickers/files` | Fuzzy-pick a file in the current directory tree and open it |
| `z b` | `pickers/buffers` | Fuzzy-pick an open buffer and switch to it |
| `z m` | `pickers/git-modified` | Fuzzy-pick a file with staged or unstaged git changes and open it |

`pickers/files-picker-with` and `pickers/git-picker-with` are internal commands that the
two public ones call; see [Design decisions](#design-decisions).

The keys sit under `z` because a picker opens a panel and waits, the layout
[`core:lsp`'s default keys](../lsp/docs/architecture.md#default-keys) explain.

## How it works

Every picker passes `(call! "stdlib/picker-keymap" handler)` as its keymap, which gives it
the default movement keys and the `Enter`/`Ctrl-o`/`Ctrl-t`/`Ctrl-v`/`Ctrl-s` placements
(current pane, new tab, vertical split, horizontal split); see `core:stdlib`'s
[Picker keymap](../stdlib/README.md#picker-keymap). Each handler switches the focused pane
to the picked buffer with `switch-to-buffer!`.

### File source

`pickers/files` picks its source per invocation, not at load time, so `:cd` re-scopes it:

1. Inside a git work tree: `git ls-files -z --cached --others --exclude-standard`. It reads
   the index with no filesystem walk and includes untracked-but-not-ignored files.
2. Otherwise, if `fd` (or Debian's `fdfind`) is installed: `fd --type f -0`.
3. Otherwise, an error naming `fd` as the thing to install.

Its handler, `pickers/open-file!`, passes the picked path to `open-buffer!`.

`git ls-files --cached` can list a file that was deleted from disk without `git rm`;
selecting one surfaces an error when the picker tries to open it.

### Buffers

`pickers/buffers` shows `buffer-display-path`, falling back to the buffer's name
(`*scratch*`, etc.) for a pathless one; see the manual's
[Picking buffers](https://cvlmtg.github.io/HUME/pickers.html#picking-buffers). `(buffers)`
hands each entry as a [pane-less pane value](../README.md#pane-values-vs-pane-less-values),
which is the picker payload. The handler, `pickers/switch-to-buffer!`, switches to that
value directly, with no `open-buffer!`.

The switch targets whichever pane is focused when the pick is made (`(focused-pane)`).
This is an exception to the index's
[capture the target](../README.md#capture-the-target-dont-re-read-focus) pattern: the
`Enter` or `Ctrl-o` binding runs before the next key is read, so the picker's origin pane
and the focused pane are the same.

### Git-modified files

`pickers/git-modified` resolves the repo root with `stdlib/git-toplevel` at dispatch and
raises an error before a picker opens when the cwd is outside any git repository.

#### Source

`git status --porcelain -z --no-renames --untracked-files=<mode>` runs in the background
through `spawn-async!`, not the line-streaming source `pickers/files` uses, since this
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
editor's cwd. The handler joins the selected row onto the root resolved at dispatch before
`open-buffer!`, so a selection opens the right file when `:pwd` is a subdirectory of the
repo.

#### Failure and cancellation

A `git status` failure logs `'error` and calls `picker-close!` with this picker's token.
That is a no-op if the picker has already closed or been replaced, so a slow failure
cannot tear down a picker the user opened since. The picker's `#:on-close` cancels the
`git status` job, so closing it before the status arrives, however it closes, leaves no
job running.

## Design decisions

- **`pickers/files` and `pickers/git-modified` each delegate to an internal
  `pickers/*-with` command, not probe and open in one body.** The internal one takes the
  git/fd probe result (or repo root) as an argument, so a test drives each branch (repo or
  no repo, `fd` present or absent) through `call!` without changing `PATH` or building a
  git sandbox.
