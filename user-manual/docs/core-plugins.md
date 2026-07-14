# Core Plugins

HUME ships a few plugins under the `core:` namespace. None load automatically — add a `declare-plugin` or `load-plugin` call to your `init.scm` for any you want. See [Plugins](plugins.md#how-plugins-are-loaded) for the difference.

## PLUM

**PLUM** — the HUME **PLU**gin **M**anager — installs and updates third-party plugins from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

PLUM is not different from any other plugin, so you must bring it in too, in your `init.scm`:

```scheme
(declare-plugin "core:plum")
```

`(load-plugin "core:plum")` also works, loading it eagerly instead.

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

## core:lsp

Language server support: hover, go-to-definition, references, diagnostics, rename,
formatting, code actions, signature help, completions, and inlay hints — and downloads
and manages the language servers themselves (`:lsp-install` / `:lsp-uninstall` /
`:lsp-servers`), independently of PLUM. Also owns runtime server management
(`:lsp-status` / `:lsp-stop` / `:lsp-restart`) — these require `core:lsp` loaded even for
a manually `register-lsp-server!`-registered server. Requires `core:stdlib` declared (or
loaded) first.
See [Language Servers](lsp.md) for setup, the full commands/keys table, and settings.

## core:helix-surround

Helix-style surround shortcuts: `ms` wraps the selection in a surrounding pair, `md` deletes a surrounding pair, `mr` replaces one.

## core:classic-paste

Opt-in GUI-style paste split: `p` / `P` paste the kill-ring head, `Ctrl+V` / `Ctrl+Shift+V` paste from the OS clipboard.

## core:vim-keybind

Vim muscle-memory keys: `$`, `^`, `0`, and the alternate-file toggle, plus `C`, `D` (delete to end of line), and `G` (go to last line). Requires `core:stdlib` loaded eagerly first — not just declared; this plugin checks for it at its own load time.

By default (`'smart`), `C` is context-sensitive: on a bare cursor it's vim's change to end of line; with an active (multi-char) selection it instead runs the default command, `copy-selection-on-next-line`, so that command stays reachable without giving up vim muscle memory for the common case. Pass `#:config` to change this:

```scheme
(load-plugin "core:stdlib")
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'off))
```

`'on` makes `C` always change to end of line regardless of selection; `'off` leaves `C` untouched, keeping the default `copy-selection-on-next-line` binding in place.

## core:stdlib

General-purpose standard library for plugin authors — a growing toolkit of commands any plugin might need, exposed via `call!` so cross-plugin code never has to re-derive them. Bring it in before any plugin that calls them:

```scheme
(declare-plugin "core:stdlib")
```

`(load-plugin "core:stdlib")` also works, loading it eagerly instead — some dependents (like `core:vim-keybind`, above) require this.

`(current-selections)` returns a list of selection records, one per cursor — treat each record as opaque; query it only through the commands below, never by picking it apart directly. Character positions count from 0; line numbers count from 1, matching what the statusline shows.

- `(call! "stdlib/single-selection?" sels)` — true when there's exactly one cursor.
- `(call! "stdlib/all-single-char?" sels)` — true when every selection covers exactly one character (no text is selected — just cursors).
- `(call! "stdlib/cursor-char-index" sels)` — the character position of the primary cursor.
- `(char-index->line idx)` — converts a character position (as returned above) to its line number.

All three `stdlib/*` commands accept `#f` and return `#f` — you only need to check `(current-selections)` for `#f` once, wherever you call it.
