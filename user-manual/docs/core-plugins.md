# Core Plugins

HUME ships a some plugins under the `core:` namespace — a plugin and grammar manager, language server support, and a few keymap alternatives. **None of them load automatically.** Nothing runs until you ask for it in your `init.scm`, so a default HUME is exactly what you see.

There are two ways to bring a plugin in:

```scheme
(declare-plugin "core:plum")    ; lazy — loads the first time you use it
(load-plugin "core:plum")       ; eager — loads at startup
```

Plugins that rebind keys have to be loaded eagerly, since their bindings must exist before you press anything. Those are marked below. See [Plugins](plugins.md#how-plugins-are-loaded) for the difference in detail.

## core:plum

**PLUM** — the HUME **PLU**gin **M**anager — installs and updates third-party plugins from GitHub, and installs the tree-sitter grammars that power syntax highlighting.

```scheme
(declare-plugin "core:plum")
```

PLUM never installs anything on its own: the commands below do the work when you run them.

| Command | Effect |
|---------|--------|
| `:plum-install` | Install all declared plugins not yet on disk |
| `:plum-cleanup` | Remove on-disk plugins no longer declared |
| `:plum-update` | Pull the latest version of every installed third-party plugin |
| `:plum-list` | Show declared / installed / orphan / missing plugins |
| `:plum-install-grammar <lang>` | Install and compile one grammar |
| `:plum-ensure-grammars` | Install every grammar you have a language for |
| `:plum-list-grammars` | Show the grammar catalog and what's installed |
| `:plum-cleanup-grammars` | Remove compiled grammars you no longer need |

Leaving PLUM out removes these commands — and, since PLUM is also what registers installed grammars at startup, syntax highlighting along with them. See [Syntax Highlighting](syntax-highlighting.md) for the grammar workflow.

## core:lsp

Language server support: hover, go-to-definition, references, diagnostics, rename, formatting, code actions, signature help, completions, and inlay hints. It also downloads and manages the servers themselves (`:lsp-install`, `:lsp-uninstall`, `:lsp-servers`), and the running processes (`:lsp-status`, `:lsp-stop`, `:lsp-restart`).

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:lsp")
```

Requires `core:stdlib`. Declared lazily like this, it wakes up on the first buffer with a detected language, or the first `:lsp-*` command you type.

See [Language Servers](lsp.md) for setup, the full command and key tables, and settings.

## core:steel-server

Registers a language server for Scheme buffers (`.ss`/`.scm`/`.sld`) — which includes your
own `init.scm` and plugin files, so you get hover, diagnostics, and completion while editing
your HUME config. Requires `core:lsp`, which provides the editor-side features that make a
registered server useful.

```scheme
(declare-plugin "core:stdlib")
(declare-plugin "core:lsp")
(declare-plugin "core:steel-server")
```

Declared lazily like this, it activates on the first Scheme buffer or the first time you run
`:steel-server-install`.

**This is a temporary plugin.** The underlying server isn't in HUME's regular server catalog
yet, so it can't be installed through `:lsp-install` like other servers. Once it lands
upstream, HUME's catalog will pick it up automatically and this plugin will be retired.

| Command | Effect |
|---------|--------|
| `:steel-server-install` | Install the Scheme language server and register it for Scheme buffers |

Installing requires `cargo` — install Rust from [rustup.rs](https://rustup.rs) first. See
[Language Servers](lsp.md) for the general LSP workflow.

## core:pickers

Fuzzy file and buffer finders: `g f` opens a file picker (git-index-backed inside a
repo, `fd`-backed otherwise), `g b` opens a buffer switcher.

```scheme
(load-plugin "core:pickers")
```

Must be loaded eagerly. See [Fuzzy Finder](pickers.md) for the file-source chain, keys,
and buffer display details.

## core:vim-keybind

Vim muscle memory: `$`, `^`, `0`, `G` (last line), `C` and `D` (change/delete to end of line), `Ctrl+6` (alternate file, kitty only), and `o` in Extend mode to swap the selection's ends.

```scheme
(load-plugin "core:stdlib")
(load-plugin "core:vim-keybind")
```

Must be loaded eagerly, and `core:stdlib` must be loaded eagerly before it.

By default (`'smart`), `C` is context-sensitive: on a bare cursor it changes to end of line as in vim, but with a real selection it runs HUME's own `copy-selection-on-next-line`, so that command stays reachable. Change this with `#:config`:

```scheme
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'on))
```

`'on` always changes to end of line; `'off` leaves `C` alone. Both of those skip the selection check, so with them `core:stdlib` isn't needed at all.

## core:helix-surround

Helix-style surround keys: `m s` wraps the selection, `m d` deletes a surrounding pair, `m r` replaces one.

```scheme
(load-plugin "core:helix-surround")
```

Must be loaded eagerly. Note that it takes over `m s` — which by default *selects* a surrounding pair — and removes `m w`, so wrapping lives on `m s` alone once it's loaded.

## core:classic-paste

GUI-style paste, if you'd rather not have `p` choose a source for you: `p` / `P` paste the kill ring, `Ctrl+V` / `Ctrl+Shift+V` paste the system clipboard (`Ctrl+Shift+V` needs the kitty protocol).

```scheme
(load-plugin "core:classic-paste")
```

Must be loaded eagerly.

## core:stdlib

A toolkit of small helpers that other plugins build on, rather than something you use directly. Several bundled plugins depend on it, so load it before them:

```scheme
(load-plugin "core:stdlib")
```

If you're writing a plugin yourself, see [Plugins](plugins.md) for what it offers.
