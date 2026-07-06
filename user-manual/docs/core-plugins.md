# Core Plugins

HUME ships a few plugins under the `core:` namespace. None load automatically — add `(load-plugin "core:...")` to your `init.scm` for any you want.

## PLUM

**PLUM** — the HUME **PLU**gin **M**anager — installs and updates third-party plugins from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

PLUM is not different from any other plugin, so you must load it too in your `init.scm`:

```scheme
(load-plugin "core:plum")
```

Disabling it just removes the management commands below — anything already installed keeps working.

### Plugin commands

| Command | Effect |
|---------|--------|
| `:plum-install` | Install all declared plugins not yet on disk |
| `:plum-cleanup` | Remove on-disk plugins no longer declared |
| `:plum-update` | Pull latest in every installed third-party plugin |
| `:plum-list` | Show declared/installed/orphan/missing plugins |

See [Plugins](plugins.md) for how to declare and write your own.

### Grammars

PLUM also installs and manages tree-sitter grammars. See [Syntax Highlighting](syntax-highlighting.md) for the full workflow and its `:plum-*-grammar` commands.

## core:helix-surround

Helix-style surround shortcuts: `ms` wraps the selection in a surrounding pair, `md` deletes a surrounding pair, `mr` replaces one.

## core:classic-paste

Opt-in GUI-style paste split: `p` / `P` paste the kill-ring head, `Ctrl+V` / `Ctrl+Shift+V` paste from the OS clipboard.

## core:vim-keybind

Vim muscle-memory keys: `$`, `^`, `0`, and the alternate-file toggle, plus `C` (change to end of line), `D` (delete to end of line), and `G` (go to last line).

`C` is the only one of these that replaces a default binding (`copy-selection-on-next-line`). If you'd rather keep that default and drop just `C`, pass `#:config`:

```scheme
(load-plugin "core:vim-keybind" #:config (hash "skip-shadows" #t))
```

## core:stdlib

A library of helper functions for plugin authors. Load it before any plugin that depends on it:

```scheme
(load-plugin "core:stdlib")
```

`(current-selections)` returns a list of selection records, one per cursor — treat each record as opaque and read it only through the accessors below, never by picking it apart directly. Character positions count from 0; line numbers count from 1, matching what the statusline shows.

- `(stdlib/selection-anchor sel)` / `(stdlib/selection-head sel)` — the two ends of a selection record. The head is the end that moves — where the cursor blinks.
- `(stdlib/selection-primary? sel)` — whether this is the primary selection (the one shown in the statusline, when there are multiple cursors).
- `(stdlib/primary-selection sels)` — pick out the primary selection from the full list.
- `(stdlib/single-selection? sels)` — true when there's exactly one cursor.
- `(stdlib/all-single-char? sels)` — true when every selection covers exactly one character (no text is selected — just cursors).
- `(stdlib/cursor-char-index sels)` — the character position of the primary cursor.
- `(char-index->line idx)` — converts a character position (as returned by the accessors above) to its line number.

All of these accept `#f` and return `#f` — you only need to check `(current-selections)` for `#f` once, wherever you call it.
