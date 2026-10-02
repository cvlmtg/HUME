# core:stdlib

General-purpose standard library for plugin authors: helpers any plugin might need,
exposed through `call!` so cross-plugin code never has to re-derive them.

Cross-plugin access in HUME is `call!`-only: plugins never `require` each other's
modules, since that would break the namespace isolation each plugin gets. So this plugin's
public API is a set of `define-command!`-registered commands rather than a `provide`d
library. A command name and a Steel binding of the same name live in separate namespaces,
so the command `"stdlib/run!"` does not collide with the function it wraps. The prelude
(convenience macros for `init.scm`, loaded at startup) is a separate layer and not part of
this plugin.

## Usage

```scheme
(load-plugin! "core:stdlib")
```

- **Depends on:** nothing.
- **Activates on:** the first call to any command below. Its `manifest.scm` has one
  `declare-plugin!` entry for `plugin.scm` whose `#:commands` list names every one of them.
  `load-plugin!` registers those stubs and does not run `plugin.scm` until one is called.
- **Overrides:** declaring it before `load-plugin!` with an explicit
  `#:commands`/`#:events`/`#:languages` list
  that omits a helper a dependent needs leaves that helper with no activation stub; see
  the [core plugins index](../README.md#depending-on-corestdlib).
- **User docs:** [Standard Library](https://cvlmtg.github.io/HUME/standard-library.html)
  for call signatures, and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-stdlib) for the
  dependency-ordering rule every other core plugin follows.

## Commands

The manual page above has the signatures. This table groups the commands; the sections
under [How it works](#how-it-works) cover what a contributor needs beyond that.

| Group | Commands |
|---|---|
| [Selections](#selections) | `stdlib/selection-anchor`, `-head`, `-start`, `-end`, `-primary?`, `stdlib/primary-selection`, `stdlib/all-single-char?`, `stdlib/single-selection?`, `stdlib/cursor-char-index` |
| [Filesystem and list search](#filesystem-and-list-search) | `stdlib/find`, `stdlib/write-file!`, `stdlib/delete-dir!`, `stdlib/delete-file!`, `stdlib/list-subdirs` |
| [Path safety](#path-safety) | `stdlib/safe-path-segment?` |
| [Subprocess](#subprocess) | `stdlib/run!` |
| [Git](#git) | `stdlib/git-repo?`, `stdlib/git-toplevel` |
| [Command arguments](#command-arguments) | `stdlib/resolve-lang-arg` |
| [Picker buffer-placement](#picker-buffer-placement) | `stdlib/with-tab`, `stdlib/with-vsplit`, `stdlib/with-split`, `stdlib/buffer-actions` |
| [Word tokenization](#word-tokenization) | `stdlib/split-words` |
| [Plugin config](#plugin-config) | `stdlib/config-boolean`, `-string`, `-enum`, `-integer`, `-list` |

## How it works

### Selections

A selection is the list `(anchor head start end primary?)` that `buffer-selections`
returns. The accessors are its only reading API, so the shape can change without breaking
callers. All nine accept `#f` and answer `#f` in turn, so a caller building on a value
that may itself be `#f` (a picker payload, an optional match) checks once, at the call
site.

`(buffer-selections pane)` is not one of the nine. It raises for a pane that isn't live or
isn't shown, since it is where a selection list is fetched, not where an already-fetched
list is picked apart.

### Filesystem and list search

Thin wrappers over Steel's `steel/filesystem` and `steel/ports`. `core:plum` and
`core:lsp-install` call them rather than each carrying a copy. `delete-dir!` and
`delete-file!` are idempotent, unlike Steel's own `delete-directory!` and `delete-file!`:
a missing target is not an error. `list-subdirs` returns the names of the directories
under a path, sorted, and skips stray files that sit alongside a directory tree
(`.install-lock`, `.DS_Store`).

### Path safety

`stdlib/safe-path-segment?` rejects the empty string, `.`, `..`, a path separator (`/` or
`\`), `:`, `"` and NUL: the set that is unsafe as one filesystem path segment. Use it for
any name that reaches `path-join` or a subprocess argument but did not come from a fixed
catalog: a user-typed slug, or a name parsed out of downloaded content.

The `:` rejection matters on Windows: a segment like `c:evil` after a single path
component makes `PathBuf::push` treat it as a drive-relative root, replacing the base path
instead of joining onto it. The rule mirrors `hume_platform::path::is_safe_segment`.

Call sites check the result with `(eq? #t (call! "stdlib/safe-path-segment?" …))`; see
[Design decisions](#design-decisions).

### Subprocess

`stdlib/run!` is `run-capture!`, the native blocking capture
(`hume_platform::process::run_capture`): it runs a command with direct argv, closes stdin,
and returns `(hash 'stdout s 'stderr s 'exit code)`. Its Rust doc explains why it exists
instead of Steel's `spawn-process`, `wait` and `child-stdout`. `GIT_TERMINAL_PROMPT=0`,
set by `hume_platform::process::base_command`, applies to it and to `run-inline-output!`
alike.

Three ways to run a subprocess exist across the codebase; pick by shape:

| Need | Use |
|---|---|
| An `#:inline-output` command | `run-inline-output!` (process-group isolation for Ctrl-c) |
| Enumeration-scale streaming output | `spawn-async!` |
| Anything else | `stdlib/run!` |

### Git

`stdlib/git-repo?` and `stdlib/git-toplevel` answer `#f` when `git` is not on `$PATH` or
the command exits non-zero. `git-repo?` compares stdout with `true` rather than checking
only the exit code: inside a bare repo, `rev-parse --is-inside-work-tree` exits 0 and
prints `false`. `core:pickers` uses both: `git-repo?` to choose `picker-files`'s source
and `git-toplevel` to resolve a `picker-git-modified` selection against the repo root.

### Command arguments

`stdlib/resolve-lang-arg` is called by `:lsp-install [lang]` and, through its grammar
argument helper, by `:plum-install-grammar [lang]`. Both are 2-arity typed commands, so
`arg` is the string the user typed after the command name, or `#f` when they typed none;
`crate::editor::dispatch`'s typed-command marshalling never puts anything else in that
slot. `resolve-lang-arg` falls back to the current buffer's language when `arg` is not a
string, and logs `'info` and answers `#f` when the buffer has none.

### Picker buffer-placement

`stdlib/with-tab`, `stdlib/with-vsplit` and `stdlib/with-split` each wrap a handler (the
one a picker already passes as `on-select`). Accepting an item then does two things:

1. Place a pane: a new tab, a side-by-side split or a stacked split. The placement command
   leaves the new pane focused.
2. Call the handler with the picker's selected payload.

Because the new pane is focused before the handler runs, the handler stays unchanged: its
`(switch-to-buffer! (focused-pane) (open-buffer! path))` or `(goto-location! …)` targets
the focused pane, which is the newly placed one. None of the three interprets `payload`, so
a handler works with any payload shape: a path, a buffer id, a `path:line:col` location.

Two guards sit in front of placement. `with-vsplit` and `with-split` share them in
`stdlib/with-pane-command`; `with-tab` has only the first, since a new tab has no
minimum-size failure:

- **A false payload** (an empty or not-yet-matching picker) skips placement but still
  calls the handler. The git-modified picker's handler cancels its in-flight async job on
  `#f`, so skipping the call would leave the job running.
- **A split refused for being too small** skips the handler, so the payload does not open
  in the pane that stayed put. This matches the typed `:split` and `:vsplit [path]`
  commands, which abort before the side effect.

`stdlib/buffer-actions` combines all three, plus a bare `ctrl-o` that runs the handler
as-is (an `Enter` synonym), into one `#:actions` alist for `picker!` and `live-picker!`.
The three `core:pickers` pickers use it. `picker!` and `live-picker!` try `#:actions` only
after every built-in picker key, so `ctrl-o`, `ctrl-t`, `ctrl-v` and `ctrl-s` never collide with a built-in; a plugin adding
its own entries picks keys the same way. A picker whose payload is not a placeable buffer
target (a theme picker, a command palette) omits `#:actions`.

### Word tokenization

`(stdlib/split-words pane str)` is `(split-words str (get-buffer-option pane
"word-chars"))`. It tokenizes `str` with that buffer's `word-chars`, the same
classification the `w` and `b` motions and text objects use, without the caller fetching
`word-chars` itself. `core:buffer-words` calls it once per line. A plugin tokenizing text
that is not `pane`'s own content calls the builtin `split-words` directly.

### Plugin config

Every config helper's error names the calling plugin (its first argument) and the offending
key, so a bad `#:config` value fails at load time with a message that says what to fix.
`core:git-diff`, `core:pickers`, `core:vim-keybind` and `core:buffer-words` use them for
their own config. All five build on an internal `stdlib/config-value`, which returns `cfg`'s
value for `key`, or `default` if absent.

## Design decisions

- **`eq? #t` at every `safe-path-segment?` call site.** A `call!` to an unknown command
  answers `#f`, the same value as a rejection. Comparing against `#t` keeps a missing
  command from reading as a valid segment.
- **`stdlib/run-stdout` is internal.** It returns `stdlib/run!`'s stdout, trimmed, when the
  command exits 0, and `#f` otherwise. Trimming is safe for a single-value probe like the
  git commands; a `-z`-delimited multi-entry blob such as `git status` can start with
  significant whitespace in its first entry.
- **`stdlib/config-value` is internal.** The five typed helpers are the cross-plugin
  surface.
