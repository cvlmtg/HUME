# Core Plugins

HUME ships some plugins under the `core:` namespace: a plugin and grammar manager, language server support, live git diff, an undo-history graph, and a few keymap alternatives. **None of them load automatically.** Nothing runs until you ask for it in your [`init.scm`](configuration.md).

Bring a plugin in with `load-plugin!`:

```scheme
(load-plugin! "core:plum")
```

Loading a plugin doesn't slow down startup: most start the first time you use them, such as when you run one of their commands or open a file in a language they handle. See [Plugins](plugins.md#how-plugins-are-loaded) for details.

## core:plum

**PLUM** (the HUME **PLU**gin **M**anager) installs and updates third-party plugins and themes from GitHub, and installs the tree-sitter grammars that power syntax highlighting. Its install and cleanup commands depend on `core:stdlib`.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

PLUM installs nothing on its own; the commands below do the work when you run them.

| Command | Effect |
|---------|--------|
| `:plum-install-plugins` | Install every plugin named in `init.scm` that is not yet on disk |
| `:plum-cleanup-plugins` | Remove on-disk plugins that `init.scm` does not name |
| `:plum-update-plugins` | Bring every installed third-party plugin to its pinned tag, branch or commit, or its latest version (see [Pinning a version](plugins.md#pinning-a-version)) |
| `:plum-list-plugins` | Show named / installed / orphan / missing plugins |
| `:plum-install-grammar <lang>` | Download and install one language's grammar; Tab-completes language names |
| `:plum-list-grammars` | Show the installed grammars, and what else the grammars folder holds: grammars installed under a name that isn't a language HUME knows, leftover files, and what grammar installs that were stopped partway left |
| `:plum-cleanup-grammars` | Remove everything in the grammars folder but the installed grammars, and what installs that were stopped partway left behind |
| `:plum-install-theme <user/repo>` | Install (or reinstall) a theme repo's themes |
| `:plum-update-themes` | Pull the latest version of every installed theme repo |
| `:plum-list-themes` | Show installed theme repos and the themes each provides |
| `:plum-remove-theme <user/repo>` | Remove an installed theme repo; Tab-completes installed slugs |

`plum/ensure-grammars` installs a list of grammars not yet installed. It takes a list argument, so it is for `init.scm`, not the `:` prompt:

```scheme
(call! "plum/ensure-grammars" '("rust" "toml"))
```

`#:config (hash 'grammar-source "/path/to/folder")` makes PLUM install the grammars a folder you built them into holds from there, and download the rest; see [Install from a local folder](syntax-highlighting.md#install-from-a-local-folder).

Leaving PLUM out only removes these commands. Already-installed plugins, grammars, and themes keep working without it; PLUM is only needed to install new ones. See [Syntax Highlighting](syntax-highlighting.md) for the grammar workflow and [Themes](themes.md#installing-themes) for the theme workflow.

## core:lsp

Language server support (hover, go-to-definition, references, symbol pickers, diagnostics, rename, formatting, code actions, signature help, completions, and inlay hints), plus management of the running server processes (`:lsp-status`, `:lsp-stop`, `:lsp-restart`). Downloading the servers is `core:lsp-install`'s job.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
```

Requires `core:stdlib` loaded first.

See [Language Servers](lsp.md) for setup, the full command and key tables, and settings.

## core:lsp-install

Downloads and verifies language servers and sets them up for `core:lsp`, with `:lsp-install`, `:lsp-uninstall` and `:lsp-catalog`. Each language uses the same servers Helix picks for it. Without this plugin, `core:lsp` works with servers you [register yourself](lsp.md#registering-a-language-server).

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp-install")
```

Requires `core:stdlib` loaded first. See [Language Servers](lsp.md#installing-servers) for the prerequisites and the commands.

## core:steel-server

Registers a language server for Scheme buffers (`.ss`/`.scm`/`.sld`), including your own
`init.scm` and plugin files, so you get hover, diagnostics, and completion while editing your
HUME config. It also makes `cog.scm` the project root marker for Scheme. It requires
`core:lsp`, which provides the editor-side features that make a registered server useful.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:lsp")
(load-plugin! "core:steel-server")
```

The server knows HUME's own commands and configuration functions, so nothing HUME provides
raises an unknown-identifier warning while you edit `init.scm` or a plugin file.

**This is a temporary plugin.** The underlying server isn't in HUME's regular server catalog
yet, so `:lsp-install` can't install it like other servers. Once it lands
upstream, HUME's catalog will pick it up automatically and this plugin will be retired.

| Command | Effect |
|---------|--------|
| `:steel-server-install` | Install the Scheme language server and register it for Scheme buffers |

Installing requires `cargo`; install Rust from [rustup.rs](https://rustup.rs) first. See
[Language Servers](lsp.md) for the general LSP workflow.

## core:pickers

Fuzzy file, buffer, and modified-file finders: `z f` opens a file picker (git-index-backed
inside a repo, `fd`-backed otherwise), `z b` a buffer switcher, and `z m` a picker over files
with staged or unstaged git changes.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:pickers")
```

Requires `core:stdlib` loaded first.

See [Fuzzy Finder](pickers.md) for the file-source chain, keys, buffer display,
modified-files details, and options.

## core:git-diff

Live, VSCode-style inline git diff. As you type, it compares the buffer against a git ref
(default `HEAD`) and renders:

- gutter `+`/`~` signs, plus a boundary mark (`▁`/`▔`) for deletions
- deleted lines as virtual lines
- added/changed lines with a background tint
- word-level highlights inside changed lines

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff")
```

Requires `core:stdlib` loaded first.

| Command | Effect |
|---------|--------|
| `:toggle-git-signs [ref]` | Toggle gutter signs for the current buffer |
| `:toggle-inline-diff [ref]` | Toggle inline rendering (virtual deleted lines, word highlights, background tint) for the current buffer |

Both take an optional git ref, e.g. `:toggle-inline-diff HEAD~2`, and Tab-complete branches
and tags from the current buffer's repo. Giving a ref always turns that rendering on and
points it at that ref, and the ref sticks across a later bare toggle off and on. The two
commands share the ref. A file git doesn't know about yet (untracked, brand-new, or outside a
repo) shows no diff.

The plugin also keeps a `"steel:git-branch"` statusline element fresh for the focused
buffer, e.g. `(main)`, or `⎇ main` with `'branch-style` set to `'icon` (see below). Add it to
your own `configure-statusline!` call (see
[Statusline → Custom elements](settings.md#custom-elements)). It updates when you switch to a
buffer and when you save it, and is empty for a buffer outside any repo.

Both commands are typed commands, run from the `:` prompt, with no keys bound by default. Each has an
editor-command twin, without the ref argument, that you can bind to a key:

```scheme
(bind-key! 'normal "space i" "git-diff/toggle-inline-diff")
(bind-key! 'normal "space s" "git-diff/toggle-git-signs")
```

A plugin or a command of your own can check and turn on the inline diff, or draw diffs of its own the same way; see [Drawing a diff](writing-plugins.md#drawing-a-diff).

Configure with `#:config`:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:git-diff"
  #:config (hash 'signs #t 'inline #f 'ref "HEAD" 'branch-style 'parens))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `'signs` | bool | `#t` | Whether gutter signs start on for a newly opened buffer |
| `'inline` | bool | `#f` | Whether inline rendering starts on for a newly opened buffer |
| `'ref` | string | `"HEAD"` | The default git ref a buffer diffs against, until overridden per-buffer via the toggle commands |
| `'branch-style` | `'parens` or `'icon` | `'parens` | How the `"steel:git-branch"` statusline element writes the branch: `(main)` or `⎇ main` |

Inline rendering's background tint and word highlights need your theme to define colors for
them; HUME's bundled themes do.

## core:undotree

A graph of the buffer's undo history in the bottom drawer, with a jump to any revision in it, including a revision on another branch.

```scheme
(load-plugin! "core:undotree")
```

It needs no other plugin, but shows what each revision changed only when [`core:git-diff`](#core-git-diff) is loaded too (see below).

| Command | Effect |
|---------|--------|
| `:undotree` | Open the focused buffer's undo tree in the drawer, or close it if it is open |
| `undotree/toggle` | The same, as a command you can bind to a key |

The plugin ships no key. Bind one in your `init.scm`:

```scheme
(bind-key! 'normal "z u" "undotree/toggle")
```

Each row is one revision: the graph, a marker column, and how long ago it was made. In the marker column, `@` is the revision the buffer is on and `S` the revision your file was last saved at. Newer revisions are on top, a branch is a column of its own, and `o-'` is a branch joining the revision it grew from:

```
o          2m
| o    @   4m
| | o      6m
| | o      7m
| o-'      9m
o-'     S 12m
```

The drawer opens with the highlight on the current revision. `Down`/`Up` move the highlight and `Ctrl-d`/`Ctrl-u` page it, as in the [language server lists](lsp.md). `Enter` moves the buffer to the highlighted revision and leaves the drawer open, so you can step through history one revision at a time; `Esc` closes it. After a jump, `U` and `Ctrl-r` redo along the branch you jumped into.

With `core:stdlib` and `core:git-diff` loaded before it, the buffer also shows what the revision you are on changed, drawn the way `core:git-diff`'s inline diff draws a diff against a git ref:

- the text of its parent revision as deleted lines above the changed lines
- the changed words highlighted
- a tint on the changed lines

The diff follows every jump and edit while the drawer is open. The root revision shows nothing, and closing the drawer clears it. The diff does not depend on whether `core:git-diff`'s own inline diff is switched on. Without `core:git-diff` the drawer works the same, and opening it shows a message saying to load the plugin.

The drawer follows the buffer: it redraws after an edit, an undo or a redo, and switches to the new buffer when you switch buffers. The ages update once a minute while it is open. Opening another drawer, such as the references list, replaces it. A read-only buffer can't be jumped, and `Enter` there reports an error. The history lives in memory only, so it starts fresh each time you open a file.

## core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion, in any buffer,
including a scratch buffer or a `.txt` file where `core:lsp` has no server to ask.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:buffer-words")
```

Requires `core:stdlib` loaded first.

It keeps a per-buffer index of identifiers, refreshed as you type, and `Ctrl-Space` reads
that index instead of scanning the buffer. When both plugins are loaded, its candidates rank
alongside `core:lsp`'s in the same menu, with `core:lsp`'s answers preferred on a tie.

A word written capitalized (`Apply`) is also offered lowercase (`apply`), and the reverse,
following the case you type. A word capitalized only because it started a sentence is still
found when you type it lowercase mid-sentence. A word with an inner capital (`HashMap`) or in
all caps (`MAX_LEN`) is offered only as written.

Configure with `#:config`:

```scheme
(load-plugin! "core:buffer-words"
  #:config (hash 'match 'string 'chars 8000))
```

| Key | Type | Default | Effect |
|---|---|---|---|
| `'match` | `'string` \| `'fuzzy` | `'string` | `'string` narrows by prefix as you type (the vim `i_CTRL-N` feel); `'fuzzy` scores subsequence matches like `core:lsp`'s own candidates |
| `'chars` | integer (≥ 1) | `8000` | Characters scanned per side of the cursor on each background indexing step, rounded down to whole lines but always at least one line. An edit changing more characters than this re-indexes in background steps instead of at once. Lower to trim a pause on a huge buffer, raise to index a large buffer in fewer steps |

## core:vim-keybind

Vim muscle memory: `$`, `^`, `0`, `C` and `D` (change/delete to end of line), `Ctrl-6` (alternate buffer, kitty only), and `o` in Extend mode to swap the selection's ends. It doesn't bind `G`, which is HUME's own prefix (`G L`/`G U`/`G C`/`G G`, plus `G R` with `core:lsp`); `g e` already goes to the last line.

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:vim-keybind")
```

Requires `core:stdlib` loaded first.

By default (`'smart`), `C` is context-sensitive. On a bare cursor with no count it changes to end of line as in vim. With a real selection or any count prefix (e.g. `3C`), it runs HUME's own `copy-selection-on-next-line`, so that command stays reachable. Change this with `#:config`:

```scheme
(load-plugin! "core:vim-keybind" #:config (hash 'change-to-eol 'on))
```

`'on` always changes to end of line; `'off` leaves `C` alone. Every mode needs `core:stdlib`,
not just `'smart`, because config validation goes through it.

## core:helix-surround

Helix-style surround keys: `m s` wraps the selection, `m d` deletes a surrounding pair, `m r` replaces one.

```scheme
(load-plugin! "core:helix-surround")
```

It takes over `m s`, which by default *selects* a surrounding pair, and removes `m w`, so wrapping lives on `m s` alone.

## core:classic-paste

GUI-style paste, if you'd rather not have `p` choose a source for you: `p` / `P` paste the kill ring, `Ctrl-v` / `Ctrl-Shift-v` paste the system clipboard (`Ctrl-Shift-v` needs the kitty protocol).

```scheme
(load-plugin! "core:classic-paste")
```

## core:stdlib

A toolkit of small helpers that other plugins build on, not something you use directly. `core:buffer-words`, `core:git-diff`, `core:lsp`, `core:lsp-install`, `core:pickers`, `core:plum` and `core:vim-keybind` all depend on it, so load it before them:

```scheme
(load-plugin! "core:stdlib")
```

::: warning Keep its own triggers
Load `core:stdlib` as shown above, without a `declare-plugin!` line of your own. The plugins that depend on it need every one of its helpers; with a narrower trigger list they misbehave instead of reporting a clear error.
:::

If you're writing a plugin, see [Standard Library](standard-library.md) for every command it offers.
