# core:pickers

Fuzzy file, buffer, and git-modified-file pickers, built on HUME's generic picker widget
(see ["Custom pickers"](https://cvlmtg.github.io/HUME/plugins.html#custom-pickers)).

## Usage

```scheme
(declare-plugin "core:stdlib")
(load-plugin "core:pickers" #:config (hash "untracked" #f))
```

Requires `core:stdlib` declared or loaded first — config validation calls
`stdlib/config-boolean` via `call!` while this plugin's body evaluates; `picker-files`/
`picker-git-modified` also call `stdlib/git-repo?`/`stdlib/git-toplevel` via `call!`, at
dispatch time (see ["Depending on another
plugin"](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)). Loads
eagerly: its keys are the only way to reach its commands, so declared lazily it would have
no trigger to ever wake it up. See [Fuzzy Finder](https://cvlmtg.github.io/HUME/pickers.html)
and [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-pickers) for keys and
config semantics.

## Commands

| Command | Effect |
|---|---|
| `picker-files` | Fuzzy-pick a file in the current directory tree and open it |
| `picker-buffers` | Fuzzy-pick an open buffer and switch to it |
| `picker-git-modified` | Fuzzy-pick a file with staged or unstaged git changes and open it |

## How it works

### Design

Built entirely from the public plugin API (`picker!`, `picker-push!`, `picker-source-spawn!`,
`spawn-async!`) — deliberately no native (Rust) picker definitions, since a fixed native set
would need a Rust PR for every new finder. Accepting a row switches the focused pane, not
just opens the buffer (`switch-to-buffer!` wrapping `open-buffer!`, never `open-buffer!`
alone).

`picker-files` and `picker-git-modified` are each split into a public command and an internal
`pickers/*-with` command that takes the git/fd probe result (or repo root) as an explicit
argument, rather than probing inline — a test seam that lets a test drive each branch (repo /
no-repo, fd present / absent) via `call!` instead of manipulating `PATH` or a real git
sandbox.

### File source

`picker-files` picks its source per invocation (not at load time, so `:cd` re-scopes it):

1. Inside a git work tree — `git ls-files -z --cached --others --exclude-standard` (reads the
   index, no filesystem walk; includes untracked-but-not-ignored files).
2. Otherwise, if `fd` (or Debian's `fdfind`) is installed — `fd --type f -0`.
3. Otherwise, an error naming `fd` as the thing to install.

`git ls-files --cached` can list a file that was deleted from disk without `git rm`;
selecting one surfaces an error when the picker tries to open it.

### Buffers

`picker-buffers` lists every open buffer, showing each one's path relative to the editor's
working directory (or its buffer name, for pathless buffers like `*scratch*`) — bare
filenames would be ambiguous whenever two open buffers share a basename.

### Git-modified files

`picker-git-modified` runs `git status --porcelain -z --no-renames --untracked-files=<mode>`
in the background — not the line-streaming source `picker-files` uses, since the picker
needs the whole, parsed output at once, not individual rows as they arrive. It opens empty
immediately, marked pending until `git status` completes, then populates in one batch,
listing every entry exactly as git prints it: the two-letter status code (`M `, `A `, ` M`,
`??`, …) followed by the path, relative to the repo root. `-z` avoids git's C-quoting of
paths with whitespace or non-ASCII; `--no-renames` guarantees one field per entry (a rename
otherwise prints as two NUL-separated fields under `-z`, parsing as a spurious extra row). A
clean tree (exit 0, empty stdout) parses to an empty item list and pushes as a no-op — no
special-casing needed.

Because rows are repo-root-relative but `open-buffer!` resolves a relative path against the
editor's cwd, the plugin resolves the selected entry against the repo root (`git rev-parse
--show-toplevel`) at accept time — so a selection opens the right file even when `:pwd` is a
subdirectory of the repo.

A cwd outside any git repository raises an error before a picker ever opens. A `git status`
failure logs `'error` and calls `picker-close! #:token token` rather than a bare
`picker-close!` — the scoped form is a no-op if this picker has already closed or been
replaced by the time a slow `git status` call fails, so it can't tear down a different
picker the user has since opened. Dismissing without selecting cancels the outstanding
`git status` job.

`"untracked"` (default `#t`) walks every untracked directory fully (`--untracked-files=all`);
a large un-ignored directory makes that walk slower, though the editor never blocks on it —
the picker opens immediately with an empty list and populates once the walk finishes. `#f`
skips the walk (`--untracked-files=no`) and populates sooner.
