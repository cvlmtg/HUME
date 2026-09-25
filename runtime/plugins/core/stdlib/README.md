# core:stdlib

General-purpose standard library for plugin authors — a growing toolkit of helpers any
plugin might need, exposed via `call!` so cross-plugin code never has to re-derive them.

## Usage

```scheme
(declare-plugin "core:stdlib")
```

- **Depends on:** nothing.
- **Activates on:** the first call to any command below — its `manifest.scm` lists every
  one of them as an activation trigger. `(load-plugin "core:stdlib")` also works, loading
  it eagerly instead.
- **Pitfall:** this mechanism only works while `core:stdlib` is declared with no explicit
  `#:commands`/`#:events`/`#:languages` override. An override that omits a helper a
  dependent needs leaves no activation stub for it — see the
  [core plugins index](../README.md#depending-on-corestdlib).
- **User docs:** [Standard Library](https://cvlmtg.github.io/HUME/stdlib.html) for full
  call signatures, and [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-stdlib)
  for the dependency-ordering rule every other core plugin follows.

## Two layers, one reason for each

HUME's scripting surface splits into two layers:

- **The prelude** — convenience macros for `init.scm`, loaded at startup.
- **`core:stdlib`** (this plugin) — commands useful to plugin authors, declared or loaded
  like any other plugin, before anything that depends on it.

Cross-plugin access in HUME is `call!`-only: plugins never `require` each other's
modules, since that would break the namespace isolation each plugin gets. That's why
this plugin's public API is a set of `define-command!`-registered commands rather than a
`provide`d library — a command name and a Steel binding of the same name live in
separate namespaces, so there's no collision between the command `"stdlib/run"` and the
function it wraps.

## Commands

The notes below cover only what the [Standard Library](https://cvlmtg.github.io/HUME/stdlib.html)
manual page doesn't — internals a plugin author calling `call!` never needs, but a
contributor touching this file does.

### Selections

`stdlib/selection-anchor`, `stdlib/selection-head`, `stdlib/selection-primary?`,
`stdlib/primary-selection`, `stdlib/all-single-char?`, `stdlib/single-selection?`,
`stdlib/cursor-char-index`.

A selection is an opaque `(anchor head primary?)` triple — its shape is this plugin's
implementation detail, not a public contract, so callers pick it apart through these
functions instead of raw `car`/`cadr`/`caddr`. All seven accept `#f` and answer `#f` in
turn, so a caller building on a value that may itself be `#f` (a picker payload, an
optional match) only has to check once, at the call site.

`(buffer-selections pane)` itself is not one of these seven — it raises rather than
answering `#f` for a pane that isn't live or isn't shown, since it's the one place a
selection list is actually fetched, not one that picks an already-fetched list apart.

### Filesystem and list search

`stdlib/find`, `stdlib/write-file`, `stdlib/delete-dir`, `stdlib/delete-file`,
`stdlib/list-subdirs`.

Thin wrappers over Steel's `steel/filesystem`/`steel/ports` — `core:plum` and `core:lsp`
both call into these rather than each carrying its own copy. `delete-dir`/`delete-file`
are idempotent, unlike Steel's own `delete-directory!`/`delete-file!` — a missing target
is not an error. `list-subdirs` filters to actual directories, skipping stray files that
sit alongside a directory tree (`.install-lock`, `.DS_Store`).

### Path safety

`stdlib/safe-path-segment?`.

Rejects the empty string, `.`/`..`, a path separator (`/` or `\`), `:`/`"`, and NUL — the
set that's unsafe to use as one filesystem path segment. Use it for any name that reaches
`path-join` or a subprocess argument but did not come from a fixed catalog: a user-typed
slug, or a name parsed out of downloaded content.

The `:` rejection matters on Windows specifically: a segment like `c:evil` after a single
path component makes `PathBuf::push` treat it as a drive-relative root, replacing the
sandboxed base path entirely instead of joining onto it (mirrors
`hume_platform::path::is_safe_segment`'s rule on the Rust side).

> [!IMPORTANT]
> Every call site checks `(eq? #t (call! "stdlib/safe-path-segment?" …))`, not a bare
> truthiness test. A `call!` miss to an unknown command already answers `#f`, same as a
> genuine rejection — the `eq?` guard keeps the two distinguishable in case that ever
> changes.

### Subprocess

`stdlib/run`.

Three ways to run a subprocess exist across the codebase; pick by shape:

| Need | Use |
|---|---|
| An `#:inline-output` command | `run-inline-output!` (process-group safety for Ctrl-c) |
| Enumeration-scale streaming output | `spawn-async!` |
| Everything else | `stdlib/run` |

`stdlib/run` is `run-capture!` (native, `hume_platform::process::run_capture`) — see its
own Rust doc for why it exists instead of Steel's `spawn-process`/`wait`/`child-stdout`/
`child-stderr`, and `hume_platform::process::base_command`'s doc for the
`GIT_TERMINAL_PROMPT=0` policy it shares with `run-inline-output!`. The git probes below
build their `#f`-on-failure policy on it.

### Git

`stdlib/git-repo?`, `stdlib/git-toplevel`.

Both build on an internal `stdlib/run-stdout` (`stdlib/run`'s stdout, trimmed, if the
command exits 0, else `#f`) — not exposed as a command itself, since trimming is only
safe for a single-value probe like these; a `-z`-delimited multi-entry blob (e.g. `git
status`) can have a leading space as significant data in its first entry, which trimming
would eat.

`git-repo?` checks stdout rather than just the exit code: inside a bare repo, `rev-parse`
exits 0 but prints `false`. `core:pickers` uses both — `git-repo?` to choose
`picker-files`'s source, `git-toplevel` to resolve a `picker-git-modified` selection
against the repo root.

### Command arguments

`stdlib/resolve-lang-arg`.

Both of `resolve-lang-arg`'s callers (`:lsp-install [lang]`, `:plum-install-grammar
[lang]`) are 2-arity typed commands, so their `arg` parameter is always either the string
the user typed after the command name, or `#f` when they typed none — `crate::editor::
dispatch`'s typed-command marshalling never injects anything else into that slot.
`resolve-lang-arg` falls back to the current buffer's language when `arg` isn't a string.

### Picker buffer-placement

`stdlib/with-tab`, `stdlib/with-vsplit`, `stdlib/with-split`, `stdlib/buffer-actions`.

Each of the first three wraps a handler — the same one a picker already passes as
`on-select` — so accepting an item does three things in order:

1. Place a pane (new tab, side-by-side split, or stacked split).
2. Focus it.
3. Call the handler with the picker's selected payload.

That ordering is what lets the handler stay unchanged: its existing
`(switch-to-buffer! (focused-pane) (open-buffer! path))` (or `(goto-location! ...)`)
targets whichever pane is focused, so it lands in the newly placed one for free. None of
the three interprets `payload` itself, so any picker's handler works no matter what shape
its payload is — a path, a buffer id, a `path:line:col` location.

Two guards sit in front of placement, both in the shared `with-pane-command` core that
`with-vsplit`/`with-split` build on (`with-tab` keeps its own simpler payload-only check,
since a new tab has no minimum-size failure mode to guard against):

- **A false payload** (an empty or not-yet-matching picker) skips placement but still
  calls the handler — the git-modified picker's handler, for instance, cancels its
  in-flight async job on `#f`, so skipping that call too would leave it running.
- **A split refused for being too small** skips the handler entirely, rather than opening
  the payload in the pane that stayed put — matching the typed `:split`/`:vsplit [path]`
  commands' own precedent of aborting before the side effect rather than silently
  redirecting it.

`stdlib/buffer-actions` composes all three plus a bare `Ctrl-o` (the handler as-is, an
`Enter` synonym) into one `#:actions` alist for `picker!`/`live-picker!`. `core:pickers`'
three built-in pickers all opt in this way. Reserved keys always win: `picker!`/
`live-picker!` try `#:actions` only after every built-in picker key (movement,
`Backspace`, `Enter`, `Escape`, query input), so `Ctrl-o`/`t`/`v`/`s` are safe choices
precisely because none of them collides with a built-in — a plugin adding its own entries
should pick keys the same way. A picker whose payload isn't a placeable buffer target (a
theme picker, a command palette) simply doesn't pass `#:actions` — there's no flag to
turn off, only a kwarg to omit.

### Word tokenization

`stdlib/split-words`.

`(stdlib/split-words pane str)` is `(split-words str (get-buffer-option pane
"word-chars"))` — tokenizing `str` using that buffer's configured `word-chars`, the same
classification `w`/`b` motions and text objects use, without the caller fetching and
threading `word-chars` through itself.

A plugin tokenizing many of `pane`'s own lines in a loop should instead read
`(get-buffer-option pane "word-chars")` once and call the underlying `split-words`
directly per line — `core:buffer-words`' own per-tick scan does this, since re-deriving
the same setting on every line would be wasted work. A plugin tokenizing text that isn't
`pane`'s own content also calls `split-words` directly.

### Plugin config

`stdlib/config-boolean`, `stdlib/config-string`, `stdlib/config-enum`,
`stdlib/config-integer`, `stdlib/config-list`.

Every error names the calling plugin (its first argument) and the offending key, so a bad
`#:config` value fails at load time with a message pointing at exactly what to fix —
`core:git-diff`, `core:pickers`, `core:vim-keybind`, and `core:buffer-words` all use this
shape for their own config. All five build on an internal `stdlib/config-value` (`cfg`'s
value for `key`, or `default` if absent) — not exposed as a command, since a raw lookup
with no type check has no cross-plugin use case these five don't already cover.
