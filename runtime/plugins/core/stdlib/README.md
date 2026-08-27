# core:stdlib

General-purpose standard library for plugin authors — a growing toolkit of commands any
plugin might need, exposed via `call!` so cross-plugin code never has to re-derive them.

## Usage

```scheme
(declare-plugin "core:stdlib")
```

With no explicit `#:commands`/`#:events`/`#:languages`, this reads `core:stdlib`'s own
`manifest.scm`, which declares every command below as an activation trigger — `call!`
dispatch activates an unactivated plugin inline before retrying the call, so a
lazily-declared `core:stdlib` still activates transparently the first time another
plugin's command body calls one of them at runtime.

`(load-plugin "core:stdlib")` also works, loading it eagerly instead.

See ["Depending on another plugin"](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin) for why a bare `(declared-plugins)` check at a plugin's own load time is enough,
and [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-stdlib) for the
dependency-ordering rule every other core plugin follows.

That mechanism breaks only if `core:stdlib` is declared with an explicit
`#:commands`/`#:events`/`#:languages` that omits a helper a dependent needs — the override
leaves no activation stub, so `call!` logs an error and returns `#void` instead of raising,
silently resolving the dependent's config read to `#void`.

## Commands

Full call signatures live in the manual's [Standard Library](https://cvlmtg.github.io/HUME/stdlib.html) page. The notes below cover only what that page doesn't — internals a plugin author calling `call!` never needs, but a contributor touching this file does.

### Selections

All seven accept `#f` and return `#f` — callers only need to check `(current-selections)`
for `#f` once, at the call site, rather than re-checking inside every helper. See "How it
works" below for why the triple itself stays opaque.

### Filesystem + list search

Thin wrappers over Steel's `steel/filesystem`/`steel/ports` — `core:plum` and `core:lsp`
both call into these rather than each carrying its own copy. `delete-dir`/`delete-file` are
idempotent unlike Steel's own `delete-directory!`/`delete-file!` — a missing target is not an
error. `list-subdirs` skips stray non-directory entries that sit alongside a directory tree
(`.install-lock`, `.DS_Store`).

### Subprocess

Three ways to run a subprocess, pick by shape: `run-inline-output!` for `#:inline-output`
commands (process-group safety for Ctrl+C), `spawn-async!` for enumeration-scale output
streams, and `stdlib/run` for everything else — a small-output command run synchronously with
the TUI's raw mode still on. `core:plum` (`plum/run!`) builds its raise-on-failure policy on
top of `stdlib/run`, and the git probes below build their `#f`-on-failure policy on it. stdin
is piped and closed immediately — never inherited from HUME's own terminal, or the child's
reads would race the editor's key reads. Ports are grabbed before `wait` (a Steel gotcha
pinned by a permanent `hume-scripting` test: `child-stderr` returns `#f` afterwards even on a
piped stream) and drained stdout-then-stderr — stdout before `wait` so a large stdout stream
doesn't sit in the pipe past `wait`'s own block, stderr after since a small diagnostic tail
costs nothing extra once the child has already exited.

### Git

Both build on an internal `stdlib/run-stdout` (`stdlib/run`'s stdout, trimmed, if the command
exits 0, else `#f`) that isn't itself exposed as a command — trimming is only safe for a
single-value probe like these; a `-z`-delimited multi-entry blob (e.g. `git status`) can have a
leading space as *significant data* in its first entry, which trimming would eat. `git-repo?`
checks stdout rather than just exit code: inside a bare repo, `rev-parse` exits 0 but prints
`false`. `core:pickers` uses both — `git-repo?` to choose `picker-files`'s source,
`git-toplevel` to resolve a `picker-git-modified` selection against the repo root.

### Command arguments

`arg` is a string only when the user typed one on the `:` command line — HUME's minibuffer
dispatch hands a bare invocation or a keymap press an integer instead (see
`hume-editor/src/editor/dispatch.rs`'s `ArgSource` marshalling); with nothing typed, the
minibuffer injects the default count `1`.

### Plugin config

Every error names the calling plugin (its first argument) and the offending key, so a bad
`#:config` value fails at load time with a message pointing at exactly what to fix — the same
shape `core:git-diff`, `core:pickers`, and `core:vim-keybind` all use for their own config. All
five build on an internal `stdlib/config-value` (`cfg`'s value for `key`, or `default` if
absent) that isn't itself exposed as a command — a raw lookup with no type check has no
cross-plugin use case these five don't already cover.

## How it works

HUME's scripting surface has two layers, and `core:stdlib` is the outermost:

- **prelude** — convenience macros for `init.scm`; loaded at startup
- **core:stdlib** (this plugin) — commands useful to plugin authors; declared or loaded, like
  any other plugin, before anything that depends on it — see Usage above.

Cross-plugin access in HUME is `call!`-only — plugins never `require` each other's modules
(that would break the namespace isolation each plugin gets). That's why the public API here
is exposed as `define-command!`-registered commands rather than a `provide`d library. A
command name and the Steel binding of the same name live in separate namespaces (command
registry vs. module scope), so there's no collision between the command `"stdlib/run"` and the
function it wraps.

`stdlib/selection-anchor`, `stdlib/selection-head`, `stdlib/selection-primary?`, and
`stdlib/primary-selection` are the sanctioned way to pick apart a selection's
`(anchor head primary?)` triple — go through them instead of raw `car`/`cadr`/`caddr`, since the
triple's shape is this plugin's implementation detail, not a public contract.
