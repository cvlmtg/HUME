# core:stdlib

General-purpose standard library for plugin authors: helpers any plugin might need,
exposed through `call!` so cross-plugin code never has to re-derive them.

Cross-plugin access in HUME is `call!`-only: plugins never `require` each other's
modules, since that would break the namespace isolation each plugin gets. So this plugin's
public API is a set of `define-command!`-registered commands rather than a `provide`d
library. A command name and a Steel binding of the same name live in separate namespaces,
so the command `"stdlib/git-toplevel"` does not collide with the function it wraps. The prelude
(convenience macros for `init.scm`, loaded at startup) is a separate layer and not part of
this plugin.

## Usage

```scheme
(load-plugin! "core:stdlib")
```

- **Depends on:** nothing.
- **Activates on:** the first call to any command below. Its `manifest.scm` declares two
  entries: `files.scm` for the filesystem, download and archive commands, and `plugin.scm`
  for the rest. Each entry's `#:commands` list names its own commands. `load-plugin!`
  registers those stubs and runs neither file until one of its commands is called, so a
  plugin that never touches files does not load `files.scm`.
- **Overrides:** declaring it before `load-plugin!` with an explicit
  `#:commands`/`#:events`/`#:languages` list
  that omits a helper a dependent needs leaves that helper with no activation stub; see
  the [core plugins index](../README.md#depending-on-corestdlib).
- **User docs:** [Standard Library](https://cvlmtg.github.io/HUME/standard-library.html)
  for call signatures, and
  [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-stdlib) for the
  dependency-ordering rule every other core plugin follows.

## Commands

The manual page above has the signatures. This table groups the commands and names the
entry that defines each group; the sections under [How it works](#how-it-works) cover what
a contributor needs beyond that.

| Group | Entry | Commands |
|---|---|---|
| [Selections](#selections) | `plugin.scm` | `stdlib/primary-selection`, `stdlib/all-single-char?`, `stdlib/single-selection?`, `stdlib/cursor-char-offset` |
| [Lists](#lists) | `plugin.scm` | `stdlib/find`, `stdlib/concat` |
| [Filesystem](#filesystem) | `files.scm` | `stdlib/write-file!`, `stdlib/delete-dir!`, `stdlib/delete-file!`, `stdlib/list-entries`, `stdlib/list-subdirs`, `stdlib/path-absolute?` |
| [Downloads, digests and archives](#downloads-digests-and-archives) | `files.scm` | `stdlib/download!`, `stdlib/verify-sha256!`, `stdlib/find-extractor`, `stdlib/extract-archive!` |
| [Git](#git) | `plugin.scm` | `stdlib/git-repo?`, `stdlib/git-toplevel` |
| [Command arguments](#command-arguments) | `plugin.scm` | `stdlib/resolve-lang-arg` |
| [Reports](#reports) | `plugin.scm` | `stdlib/report-section`, `stdlib/show-report!` |
| [Picker keymap](#picker-keymap) | `plugin.scm` | `stdlib/picker-keymap`, `stdlib/with-tab`, `stdlib/with-vsplit`, `stdlib/with-split` |
| [Word tokenization](#word-tokenization) | `plugin.scm` | `stdlib/split-words` |
| [Plugin config](#plugin-config) | `plugin.scm` | `stdlib/config-boolean`, `-string`, `-enum`, `-integer`, `-list` |

## How it works

### Selections

The helpers work on the list `buffer-selections` returns and read each selection with
the core `selection-*` builtins, the only readers of its shape. All four accept `#f` and
answer `#f` in turn, so a caller building on a value that may itself be `#f` (a picker
payload, an optional match) checks once, at the call site.

`(buffer-selections pane)` is not one of the four. It raises for a pane that isn't live or
isn't shown, since it is where a selection list is fetched, not where an already-fetched
list is picked apart.

### Lists

`concat` joins a list of lists into one, in order, with one `foldl` over each list and a
single `reverse`. Steel's `append` called with many lists (`(apply append lists)`) costs
time that grows with the number of lists times their total length: on 2,000 one-entry
lists, as `core:git-diff` builds for 2,000 changed lines, it took 16 ms where `concat`
takes about 0.5 ms. `core:git-diff`, `core:lsp` and `core:plum` join their per-item lists
with it.

### Filesystem

Thin wrappers over Steel's `steel/filesystem` and `steel/ports`. `core:plum` and
`core:lsp-install` call them rather than each carrying a copy. `delete-dir!` and
`delete-file!` are idempotent, unlike `remove-dir!` and Steel's own `delete-file!`:
a missing target is not an error. `list-entries` returns the names of every entry of a
directory, sorted, and `list-subdirs` only those that are directories, skipping stray files
that sit alongside a directory tree (`.install-lock`, `.DS_Store`). Both raise when the
directory cannot be read, a missing one included: a general listing cannot tell a directory
that does not exist yet from a wrong path, so a caller that means "empty" checks
`path-exists?` first. `path-absolute?` follows Rust's `Path::is_absolute`: on
Windows a path is absolute only with both a drive and a root (`C:\x`, `C:/x`) or as a UNC
path (`\\server\share`), so `\x` (the current drive's) and `C:x` (relative to drive `C:`'s
working directory) are not. Elsewhere it is absolute when it starts with `/`.

### Downloads, digests and archives

`core:lsp-install` and `core:plum` download through `download!`, which runs `curl -fsSL`:
an HTTP error fails the command instead of saving the error page, a redirect (as GitHub's
release downloads use) is followed, and a `--` before the URL keeps it from being read as an
option. An offset and a size fetch that byte range with `-r`, for one member of a bundle.

They both verify and unpack downloads, through `verify-sha256!` and `extract-archive!`. Both shell out to the platform's tools rather than adding hashing or archive
crates; `docs/LSP-INSTALL.md` gives the reasoning. `verify-sha256!` compares the digests
case-insensitively. `extract-archive!` walks the extracted tree once, refuses a symlink that resolves outside
the target directory (`first-path-outside`), and returns the regular files, which
`core:lsp-install` marks executable.

`find-extractor` picks the program for an archive format, and `extract-archive!` runs the
one it picks, so `core:lsp-install`'s check before a download finds the program the
extraction runs. Off Windows that is `tar` for a tar archive and `unzip` for a zip archive,
on the search path. On Windows it is Windows' own bsdtar, `%SystemRoot%\System32\tar.exe`,
which extracts both; without it, `unzip` on the search path for a zip archive, then the
search path's `tar`. Under Git Bash that `tar` is GNU tar, which reads a `C:\...` archive path
as a remote host and cannot extract a `.zip`, so it comes last.

### Git

`stdlib/git-repo?` and `stdlib/git-toplevel` answer `#f` when `git` is not on `$PATH` or
the command exits non-zero. `git-repo?` compares stdout with `true` rather than checking
only the exit code: inside a bare repo, `rev-parse --is-inside-work-tree` exits 0 and
prints `false`. `core:pickers` uses both: `git-repo?` to choose `pickers/files`'s source
and `git-toplevel` to resolve a `pickers/git-modified` selection against the repo root.

### Command arguments

`stdlib/resolve-lang-arg` is called by `:lsp-install [lang]` and, through its grammar
argument helper, by `:plum-install-grammar [lang]`. Both are 2-arity typed commands, so
`arg` is the string the user typed after the command name, or `#f` when they typed none;
`crate::editor::dispatch`'s typed-command marshalling never puts anything else in that
slot. `resolve-lang-arg` falls back to the current buffer's language when `arg` is not a
string, and logs `'info` and answers `#f` when the buffer has none.

### Reports

A list command shows its output in a read-only buffer, because a list does not fit the one
status-line slot `log!` writes to. `stdlib/report-section` renders one section: a
`title (count)` header, followed by `, hint` when the section has items (`hint` may be
`#f`), then one indented line per item, or `none`. `stdlib/show-report!` joins sections
with a blank line and shows them with `show-read-only-buffer!`, in the buffer `[name]`.
`core:plum`'s list commands and `:lsp-catalog` build their output this way.

### Picker keymap

`stdlib/picker-keymap` is the one default keymap a picker takes: `picker!` and
`live-picker!` answer no key besides typing and `Backspace` unless their keymap binds it.
It is a plain hashmap built per call around the plugin's own handler, so a plugin reuses it
whole or `hash-insert`s the keys it wants different. The three `core:pickers` pickers and
`core:lsp`'s symbol pickers use it as is.

Movement keys call `picker-move!` and leave the picker open. `Esc` only closes it.
`Enter`, `Ctrl-o`, `Ctrl-t`, `Ctrl-v` and `Ctrl-s` go through `stdlib/picker-accept`, which
calls `picker-close!` before the handler: `.` runs the closing binding again in full, so
nothing may come before the close that `.` should not repeat.

`stdlib/with-tab`, `stdlib/with-vsplit` and `stdlib/with-split` each wrap a handler. Accepting
an item then does two things:

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
  calls the handler, which decides what "nothing matched" means, the same as `Enter`.
- **A split refused for being too small** skips the handler, so the payload does not open
  in the pane that stayed put. This matches the typed `:split` and `:vsplit [path]`
  commands, which abort before the side effect.

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

- **`stdlib/run-stdout` is internal.** It returns `run-capture!`'s stdout, trimmed, when the
  command exits 0, and `#f` otherwise. Trimming is safe for a single-value probe like the
  git commands; a `-z`-delimited multi-entry blob such as `git status` can start with
  significant whitespace in its first entry.
- **`stdlib/config-value` is internal.** The five typed helpers are the cross-plugin
  surface.
