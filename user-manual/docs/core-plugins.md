# Core Plugins

HUME ships four plugins under the `core:` namespace. None load automatically — add `(load-plugin "core:...")` to your `init.scm` for any you want.

## PLUM

**PLUM** — the HUME **PLU**gin **M**anager — installs and updates third-party plugins from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

PLUM is not different from any other plugin, so you must load it too:

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
