# Core Plugins

HUME ships some plugins under the `core:` namespace: a plugin and grammar manager, language server support, live git diff, and a few keymap alternatives. **None of them load automatically.** Nothing runs until you ask for it in your [`init.scm`](configuration.md), so a default HUME is exactly what you see.

Bring a plugin in with `load-plugin!`:

```scheme
(load-plugin! "core:plum")
```

A plugin that ships a manifest (stdlib, lsp, lsp-install, plum, git-diff and steel-server) loads lazily: its code runs the first time a command, event or language it lists comes up. The rest (buffer-words, classic-paste, helix-surround, pickers and vim-keybind) have no manifest and load at startup, because their key bindings only exist once their code has run. See [Plugins](plugins.md#how-plugins-are-loaded) for the difference in detail.

## core:stdlib

A toolkit of small helpers that other plugins build on, rather than something you use directly. `core:git-diff`, `core:pickers`, `core:vim-keybind`, `core:lsp`, and `core:lsp-install` all depend on it, so load it before them:

```scheme
(load-plugin! "core:stdlib")
```

::: warning Keep its own triggers
Don't declare custom `#:commands`/`#:events`/`#:languages` for `core:stdlib`; load it as shown above. Every plugin that depends on `core:stdlib` relies on the triggers its manifest lists; a custom list can leave out a helper a dependent plugin needs, and that dependent plugin will then misbehave instead of failing with a clear error.
:::

If you're writing a plugin yourself, see [Plugin API → Standard Library](plugin-api.md#standard-library) for every command it offers.

## core:plum

**PLUM** (the HUME **PLU**gin **M**anager) installs and updates third-party plugins and themes from GitHub, and installs the tree-sitter grammars that power syntax highlighting. Its install and cleanup commands depend on `core:stdlib`.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

PLUM never installs anything on its own: the commands below do the work when you run them.

| Command | Effect |
|---------|--------|
| `:plum-install-plugins` | Install every plugin named in `init.scm` that is not yet on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins that `init.scm` does not name |
| `:plum-update-plugins` | Pull the latest version of every installed third-party plugin |
| `:plum-list-plugins` | Show named / installed / orphan / missing plugins |
| `:plum-install-grammar <lang>` | Install and compile one grammar; Tab-completes declared grammar names |
| `:plum-list-grammars` | Show the grammar catalog and what's installed |
| `:plum-cleanup-grammars` | Remove compiled grammars you no longer need |
| `:plum-install-theme <user/repo>` | Install (or reinstall) a theme repo's themes |
| `:plum-update-themes` | Pull the latest version of every installed theme repo |
| `:plum-list-themes` | Show installed theme repos and the themes each provides |
| `:plum-remove-theme <user/repo>` | Remove an installed theme repo; Tab-completes installed slugs |

`plum-ensure-grammars` (install a list of grammars not yet compiled) is for `init.scm`, not the command mode prompt; it takes a list argument.

```scheme
(call! "plum-ensure-grammars" '("rust" "toml"))
```

Leaving PLUM out only removes these commands. Already-installed plugins, grammars, and themes keep working without it. PLUM is only needed to install new ones. See [Syntax Highlighting](syntax-highlighting.md) for the grammar workflow and [Configuration](configuration.md#themes) for the theme workflow.

## core:lsp

Language server support: hover, go-to-definition, references, diagnostics, rename, formatting, code actions, signature help, completions, and inlay hints, and manages the running server processes (`:lsp-status`, `:lsp-stop`, `:lsp-restart`). Downloading the servers is `core:lsp-install`'s job.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
```

Requires `core:stdlib` loaded first. `core:lsp` loads lazily: it wakes up on the first buffer with a detected language, or the first `:lsp-*` command you type, and its key bindings go live at that same moment, before there's a buffer they'd need to act on.

See [Language Servers](lsp.md) for setup, the full command and key tables, and settings.

## core:lsp-install

Downloads, verifies and registers language servers: `:lsp-install`, `:lsp-uninstall`, `:lsp-servers` and `:lsp-rescan-servers`. It registers what it installed through `register-lsp-server!`, so `core:lsp` works the same with or without it, and a different installer can take its place by declaring that one instead.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp-install")
```

Requires `core:stdlib` loaded first. Loads lazily in two parts: `:lsp-rescan-servers` and the registration of installed servers wake on the first buffer with a detected language, and `:lsp-install`, `:lsp-uninstall` and `:lsp-servers` load on first use. See [Language Servers](lsp.md#installing-servers) for the prerequisites and the commands.

## core:steel-server

Registers a language server for Scheme buffers (`.ss`/`.scm`/`.sld`), which includes your
own `init.scm` and plugin files, so you get hover, diagnostics, and completion while editing
your HUME config. Requires `core:lsp`, which provides the editor-side features that make a
registered server useful.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
(load-plugin! "core:steel-server")
```

It activates on the first Scheme buffer or the first time you run
`:steel-server-install`. It's registered so HUME's own commands and configuration functions
are recognized while you edit `init.scm` or a plugin file, so you won't see unknown-identifier
warnings for anything HUME itself provides.

**This is a temporary plugin.** The underlying server isn't in HUME's regular server catalog
yet, so it can't be installed through `:lsp-install` like other servers. Once it lands
upstream, HUME's catalog will pick it up automatically and this plugin will be retired.

| Command | Effect |
|---------|--------|
| `:steel-server-install` | Install the Scheme language server and register it for Scheme buffers |

Installing requires `cargo`. Install Rust from [rustup.rs](https://rustup.rs) first. See
[Language Servers](lsp.md) for the general LSP workflow.

## core:pickers

Fuzzy file, buffer, and modified-file finders: `z f` opens a file picker (git-index-backed
inside a repo, `fd`-backed otherwise), `z b` opens a buffer switcher, `z m` opens a picker
over files with staged or unstaged git changes.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers")
```

It has no manifest, so it loads at startup (`core:stdlib` only needs to be loaded before it): its keys are the only way to reach its commands, and nothing else could wake it. By
default the modified-files picker includes untracked files; turn them off with `#:config`:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers" #:config (hash "untracked" #f))
```

See [Fuzzy Finder](pickers.md) for the file-source chain, keys, buffer display, and
modified-files details.

## core:git-diff

Live, VSCode-style inline git diff. As you type, compares the buffer against a git ref
(default `HEAD`) and renders gutter `+`/`~` signs plus a boundary mark (`▁`/`▔`) for
deletions, deleted lines as virtual lines, added/changed lines with a background tint, and
word-level highlights inside changed lines.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff")
```

Requires `core:stdlib` loaded before it. It wakes on the
first buffer opened (signs default on) or the first `:toggle-git-signs`/`:toggle-inline-diff`
you type.

| Command | Effect |
|---------|--------|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer |

Both take an optional git ref, e.g. `:toggle-inline-diff HEAD~2`, Tab-completing branches and
tags from the current buffer's repo. Giving a ref always turns that rendering on and points it
at that ref; it's sticky across a later bare toggle off/on. The ref is shared between the two
commands. A file git doesn't know about yet (untracked, brand-new, or outside a repo) shows no
diff.

Also keeps a `"steel:git-branch"` statusline element fresh for the focused buffer, e.g.
`(main)`. No config needed, just add it to your own `configure-statusline!` call (see
[Statusline → Custom elements](configuration.md#custom-elements)). Updates when you switch to
a buffer and when you save it; empty for a buffer outside any repo.

Both commands are typed commands, run from the `:` prompt; they have no key bindings.

Configure with `#:config`:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff"
  #:config (hash "signs" #t "inline" #f "ref" "HEAD"))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `"signs"` | bool | `#t` | Whether gutter signs start on for a newly opened buffer |
| `"inline"` | bool | `#f` | Whether inline rendering starts on for a newly opened buffer |
| `"ref"` | string | `"HEAD"` | The default git ref a buffer diffs against, until overridden per-buffer via the toggle commands |

Inline rendering's background tint and word highlights depend on your theme defining colors
for them; HUME's bundled themes do.

## core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion. Works in any
buffer, including a scratch buffer or a `.txt` file where `core:lsp` has no server to ask.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:buffer-words")
```

It has no manifest, so it loads at startup: `Ctrl-Space` (or a completion source's own trigger char, if one
applies) is the only thing that can ever invoke the completion source it registers, and nothing else could wake it. Requires `core:stdlib` loaded
first.

Keeps a per-buffer index of identifiers, refreshed as you type; `Ctrl-Space` reads it, it
never scans the buffer itself. Ranks alongside `core:lsp`'s own completions in the same menu
when both are loaded, with `core:lsp`'s answers preferred on a tie.

A word written capitalized (`Apply`) is also offered lowercase (`apply`), and the reverse,
following the case you type, so a word capitalized only because it started a sentence is
still found when you type it lowercase mid-sentence. A word with an inner capital (`HashMap`)
or written in all caps (`MAX_LEN`) is offered only as written.

Configure with `#:config`:

```scheme
(load-plugin! "core:buffer-words"
  #:config (hash "match" 'string "lines" 100))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `"match"` | `'string` \| `'fuzzy` | `'string` | `'string` narrows by prefix as you type (the vim `i_CTRL-N` feel); `'fuzzy` scores subsequence matches like `core:lsp`'s own candidates |
| `"lines"` | integer (≥ 1) | `100` | Lines fetched and scanned per side of the cursor on each background indexing tick. Lower to trim a pause on a huge buffer, raise to index a large buffer in fewer ticks |

## core:vim-keybind

Vim muscle memory: `$`, `^`, `0`, `C` and `D` (change/delete to end of line), `Ctrl-6` (alternate buffer, kitty only), and `o` in Extend mode to swap the selection's ends. It does not bind `G`: that key is HUME's own prefix (`G L`/`G U`/`G C`, plus `G R` with `core:lsp`), and `g e` already goes to the last line.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind")
```

It has no manifest, so it loads at startup (`core:stdlib` only needs to be loaded before it): it replaces keys HUME already binds, and most of what it rebinds (`goto-line-start`, `goto-line-end`, and the rest) are built-in commands, not plugin commands, so there's no first dispatch to trigger loading.

By default (`'smart`), `C` is context-sensitive: on a bare cursor with no count it changes to end of line as in vim, but with a real selection, or any count prefix (e.g. `3C`), it runs HUME's own `copy-selection-on-next-line`, so that command stays fully reachable. Change this with `#:config`:

```scheme
(load-plugin! "core:vim-keybind" #:config (hash "change-to-eol" 'on))
```

`'on` always changes to end of line; `'off` leaves `C` alone. `core:stdlib` is required for
every mode, not just `'smart`, since config validation itself goes through it.

## core:helix-surround

Helix-style surround keys: `m s` wraps the selection, `m d` deletes a surrounding pair, `m r` replaces one.

```scheme
(load-plugin! "core:helix-surround")
```

It has no manifest, so it loads at startup: it takes over `m s` (which by default *selects* a surrounding pair) and removes `m w` outright, so wrapping lives on `m s` alone once it's loaded.

## core:classic-paste

GUI-style paste, if you'd rather not have `p` choose a source for you: `p` / `P` paste the kill ring, `Ctrl-v` / `Ctrl-Shift-v` paste the system clipboard (`Ctrl-Shift-v` needs the kitty protocol).

```scheme
(load-plugin! "core:classic-paste")
```

It has no manifest, so it loads at startup: it replaces `p`/`P`/`Ctrl-v`/`Ctrl-Shift-v`'s default behavior.
