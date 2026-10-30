# Coming From Helix

HUME shares Helix's core editing model (select-then-act, selections as first-class citizens), so the mental shift is small. The differences are mostly in depth, configurability, and tooling.

## The first ten minutes with HUME

### Opening a file

`hume file.txt:42:5` and `:e path` work as in Helix, including `:e` on a path that doesn't exist yet: the first `:w` creates the file. `:o` / `:open` are not aliases; use `:e`.

The fuzzy finder is a plugin. Helix's `Space f` and `Space b` become `z f` and `z b` once `core:pickers` is loaded. The [starter config](configuration.md#example-init-scm) loads it, along with language server support and other plugins, so copy that first. See [Fuzzy Finder](pickers.md).

### Editing

Type `:tutor` and press `Enter`. The tutor is your own copy, so edit it freely. Make the same edits you would in Helix on its opening paragraphs, finding each line with `/` as you would there:

| Edit | Helix | HUME |
|------|-------|------|
| Delete `Welcome. ` from `Welcome. This buffer is your practice space.` | `/Welcome` `Enter` `v` `t` `T` `d` | `/Welcome` `Enter` `e` `t` `T` `d` |
| Change `practice` to `play` on the same line, then undo it with `u` | `/practice` `Enter` `c` `play` `Esc` | `/practice` `Enter` `c` `play` `Esc` |
| Copy that line to another application | `x` `Space y` | `x` `y` |
| Delete the two lines of `When you are ready, … move down.` | `/When you` `Enter` `x` `x` `d` | `/When you` `Enter` `x` `Ctrl-x` `d` |

What each step shows:

- **`t` only moves; Extend mode makes it select.** `f`, `F`, `t` and `T` move the cursor and select nothing, where Helix selects up to the target. `e` toggles Extend mode (Helix's `v`), where every motion grows the selection. So `t T` grows `Welcome` to `Welcome. `, and `d` deletes it and leaves the mode. On a terminal with the [kitty keyboard protocol](installation.md#terminal-compatibility), holding `Ctrl` does the same for one motion: `Ctrl-t T` grows the selection without entering Extend mode.
- **A second `x` moves on.** Each `x` selects the next line on its own, dropping the previous one. `Ctrl-x` extends instead. See [Line selection](#line-selection-x-vs-extend-mode-e).
- **`Esc` keeps what you typed selected.** After the change, `play` is still selected instead of a plain cursor, ready to act on again: delete it, surround it, search for it. One consequence: `i` re-enters *before* that selection, so use `a` to keep typing after it. Disable this with the `insert-select-on-exit` option (see [Buffer options](settings.md#buffer-options)).
- **`y` reaches the system clipboard.** It copies to the clipboard as well as HUME's own kill ring. `p` pastes your last yank or delete while you haven't edited since, and the system clipboard once you have. See [Copy & Paste](copy-and-paste.md).

### Saving and quitting

`:w`, `:wq`, `:q!`, `:qa` and `:qa!` work as in Helix. There is no `:x`; `:write-quit` is the long form of `:wq`.

`:q` on the last pane closes the current buffer, not the editor. If other files are still open, you land on one of them; HUME quits only when nothing is left. `:qa` leaves in one step. See [Quitting](files-and-buffers.md#quitting).

The tutor you just started is the hands-on tour of everything else: carry on from its Introduction. To start over with a clean copy, close it with `:bd!` and run `:tutor` again.

## Bringing your config over

HUME has no TOML config file. Every setting and keybinding is a call to a Scheme function in [`init.scm`](configuration.md): `(set-option! "name" value)` for options, `(bind-key! 'normal "keys" "command")` for keys. As a real programming language, `init.scm` also has conditionals, loops, and abstraction, which a static TOML file lacks.

| Setting | Helix `config.toml` | HUME `init.scm` |
|---|---|---|
| Theme | `theme = "onedark"` | `(set-option! "theme" "onedark")` |
| Line numbers | `[editor]`<br>`line-number = "relative"` | `(set-option! "gutter-line-numbers" "relative")` |
| Scroll padding | `scrolloff = 5` | `(set-option! "scroll-margin" 5)` |
| Mouse | `mouse = false` | `(set-option! "mouse" #f)` |
| Soft wrap | `[editor.soft-wrap]`<br>`enable = true` | `(set-option! "wrap-mode" "soft")` |
| Whitespace indicators | `[editor.whitespace]`<br>`render = "all"` | `(set-option! "whitespace-space" "all")`<br>`(set-option! "whitespace-tab" "all")`<br>`(set-option! "whitespace-newline" "all")` |
| Insert-mode cursor | `[editor.cursor-shape]`<br>`insert = "bar"` | `(set-option! "insert-cursor-shape" "bar")` |
| Auto-pairs | `auto-pairs = false` | `(set-option! "insert-auto-pairs" #f)` |
| Inlay hints | `[editor.lsp]`<br>`display-inlay-hints = true` | `(load-plugin! "core:lsp" #:config (hash 'inlay-hints #t))` |
| Statusline | `[editor.statusline]`<br>`left = [...]` | `(configure-statusline! '(...) '(...) '(...))`, see [Statusline](#statusline) |
| Keybinding | `[keys.normal]`<br>`"C-j" = "move_line_down"` | `(bind-key! 'normal "ctrl-j" "move-down")` |

Indentation (Helix's per-language `indent.tab-width`/`indent.unit` in `languages.toml`) is a buffer option in HUME, so you set it per language from a hook instead of a separate file:

```scheme
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "python")
      (set-buffer-option! pane "indent-width" 4)
      (set-buffer-option! pane "indent-style" "soft"))))
```

`bind-key!` takes a command *name* (listed in [Editor Commands](editor-commands.md)), not a key sequence: there's no key-to-key remapping.

## Bringing your themes over

HUME reads Helix's theme format: the same TOML with `[palette]` indirection, the same color forms (hex, a palette name, or one of the sixteen bare terminal color names like `red`), the same modifier names, and the same underline styles. Most Helix themes work unchanged.

- **Your own themes:** copy them from `~/.config/helix/themes/` to `~/.config/hume/themes/` (`%APPDATA%\hume\themes\` on Windows). `inherits` works too. HUME bundles many of Helix's popular themes (`onedark`, `catppuccin_mocha`, `gruvbox`, `nord`, …), so a theme built on one of them still finds its parent.
- **Published themes:** run `:plum-install-theme <user/repo>` and [PLUM](core-plugins.md#core-plum) fetches the theme repo from GitHub, the same way it installs plugins.

Type `:theme ` and press Tab to try one for the session, then set it as your default in your [`init.scm`](configuration.md#example-init-scm). Here's [Everforest](https://github.com/cvlmtg/everforest.hume), the first third-party theme for HUME, ported as-is from Helix:

```scheme
(set-option! "theme" "everforest_dark")
```

HUME's tab bar reads `ui.tabline`, and falls back to Helix's `ui.bufferline` when a theme doesn't set it. [Themes](themes.md) covers the few other places the formats differ and links a visual theme editor.

## What's the same

- Select-then-act: motions change the selection; operators act on it
- Selections are always visible and always cover at least one character
- `:` command mode prompt, `/` search
- `d`, `c`, `y`, `p` for delete/change/yank/paste
- `u` / `U` undo / redo
- `>` / `<` indent / unindent the lines a selection touches, with count support (`3>`). HUME rewrites each line's whole indent in the buffer's `indent-style`; see [Lines and alignment](editing.md#lines-and-alignment)
- `m i f`/`m a f`, `m i t`/`m a t`, `m i a`/`m a a`, `m i c`/`m a c`, `m i u`/`m a u`, `m i x`/`m a x` select the enclosing function, class/type, argument, comment, unit test, or tag pair, for any language with a tree-sitter grammar that ships a textobjects query. The letters match Helix's own match-mode textobjects, except unit test (`T`→`u`; see below)
- Enter, `o` and `O` indent new lines with Helix's indentation queries (see [Indentation](editing.md#indentation)), written in the buffer's `indent-style`. There is no `indent-heuristic` setting: HUME always uses Helix's default, `hybrid`. `indent-smart` turns automatic indentation off. HUME also moves a closing bracket or keyword back as you type it

## Key differences

### Plugin system

Helix's Steel plugin system is still an unmerged branch; HUME's ships in every release. HUME's plugins use the same Steel Scheme dialect, but plugins written for Helix's Steel branch won't run in HUME: the two editors expose different functions to scripts. [PLUM](core-plugins.md#core-plum) is HUME's plugin manager, installing plugins from GitHub. [Installing a plugin](plugins.md#installing-a-plugin) walks through it with [grep.hume](https://github.com/cvlmtg/grep.hume), a live-grep picker and HUME's first official third-party plugin.

### Growing selections

Helix grows a selection in select mode (`v`). HUME's equivalent is Extend mode (`e`), where every motion extends until you act or press `Esc`. HUME also has one-shot extends (`Ctrl-w`, `Ctrl-b`, …) that extend for a single keystroke without changing mode. These need the kitty keyboard protocol.

### Line selection: `x` vs Extend mode (`e`)

Both editors bind `x` to select the current line. The difference is depth:

| Press | Helix `x` | HUME `x` | HUME `e` then `x` |
|-------|-----------|----------|-------------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

Helix's `x` is **modal**: once pressed, every further `x` extends the selection line-wise until you cancel.

HUME's `x` is **one-shot**: each press re-anchors to the next line. For Helix's repeat-extend behavior, enter **Extend mode** first (`e`), where `x`, like every other motion, extends rather than replaces. `Ctrl-x` is a one-shot extend that doesn't enter Extend mode.

`X` is the backward form. In Normal mode, on a whole-line selection, it jumps to the line above. To shrink a line selection grown downward, press `X` in Extend mode, or `Ctrl-Shift-x` (kitty only); each press removes one line. Helix has no key for that.

### Multiple selections

Both editors share the same foundations (multiple cursors, `;` to collapse, `S` to split into lines), but keybindings and a few operations differ:

| Operation | Helix | HUME |
|-----------|-------|------|
| Copy selection on line below | `C` | `C` (duplicates each selection to the same column on the next line, adding a multi-cursor: column editing through multi-cursor, not a rectangular visual block) |
| Copy selection on line above | `Alt-C` | (unbound) |
| Remove primary selection | `Alt-,` | `Ctrl-,` (kitty only) |
| Flip selections | `Alt-;` (Normal and Select mode) | `Ctrl-e` (Normal and Extend mode) |
| Merge consecutive selections | `Alt-_` (touching selections only); `Alt--` merges all into one span | automatic: selections that share a character merge; touching ones stay separate |
| Align selections | `&` | `&` |
| Trim whitespace at edges | `_` | `_` |
| Sort | `:sort` | `:sort` (different semantics; see below) |
| Sift within (regex per selection) | `s` | `s` |
| Select all search matches | no dedicated key; `%` (select whole buffer) then `s` (sift to regex matches) | `m /` |
| Search selection, auto word-boundary anchors | `*` | *(none)* |
| Search word under cursor (Vim-style) | *(unbound)* | `*` |
| Search selection literally, no anchors | `Alt-*` | `Ctrl-/` (kitty only) |

::: warning
HUME's `*` is Vim-style: it expands to the word (or punctuation run) under the cursor and ignores any existing selection. Helix's `*` searches the literal current selection. HUME's `Ctrl-/` (kitty only) is the closer match to Helix's `Alt-*`.

To give `*` that behavior instead, rebind it in your `init.scm`:

```scheme
(bind-key! 'normal "*" "search-selection")
```
:::

::: warning
HUME's `:sort` permutes whole rows, keyed by whatever text you select on each one. Select whole lines and it behaves like a plain line sort. Select a column across several lines and it sorts by that column, moving the entire rows along with it. `%` then `:sort` sorts the whole file directly, with no splitting step.
:::

### Statusline

Helix configures its statusline in TOML (`[editor.statusline]`); HUME configures it from Scheme. Both work the same way in practice: you reorder and toggle a fixed set of built-in elements across left/center/right zones:

```scheme
(configure-statusline! '("Mode" "FileName") '("SearchMatches") '("Position"))
```

You can also add custom elements from Scheme; see [Statusline](settings.md#custom-elements).

### Bufferline vs tab bar

Helix's `bufferline` (`never` / `always` / `multiple`) is a strip of open **buffers**: pick one and it swaps into the focused split. HUME's bar lists **tab pages** instead. A tab is a saved window layout (its own splits and focused pane), so switching tabs swaps the whole pane arrangement, not a single buffer. Buffers stay global across tabs; reach them with `:ls` / `:b` / `:bn` / `:bp`, not from the bar. See [Tabs](files-and-buffers.md#tabs).

`:set global tabline=never/always/dynamic` controls visibility. `dynamic` (the default) is the analogue of Helix's `multiple`: the bar shows only once a second tab is open. Click a tab to switch to it; the bar scrolls when tabs overflow the width. A Helix theme colors it through `ui.bufferline`; see [Bringing your themes over](#bringing-your-themes-over).

### Surround

Helix uses `ms`, `md`, `mr` for surround. HUME has its own default bindings and a Helix-compatible mode:

| Action | HUME (default) | HUME (helix-surround plugin) |
|--------|---------------|------------------------------|
| Wrap | `mw` + char | `ms` + char |
| Delete | `ms` + char, then `d` | `md` + char |
| Replace | `ms` + char, then `r` | `mr` + char |

By default there's no dedicated delete or replace key, because you don't need one: `ms` selects the surrounding pair, and the ordinary `d` and `r` act on it. The plugin takes `ms` over for wrapping and removes `mw`.

Enable the Helix-style bindings by loading the built-in plugin:

```scheme
(load-plugin! "core:helix-surround")
```

### Matching brackets and structural navigation

Helix's match mode binds `m m` to jump to the matching bracket. HUME binds the same idea directly to `#`, with no mode step, and also matches HTML/XML/JSX tag pairs. `%` (select whole buffer) is untouched in either editor.

Helix's bracket mode also binds `]f`/`[f`, `]t`/`[t`, `]a`/`[a`, `]c`/`[c`, `]T`/`[T`, and `]x`/`[x` by default, jumping straight to the next/previous function, class, argument, comment, unit test, or tag pair. HUME puts the same six kinds, plus `value` for array/tuple/struct entries (which Helix's bracket mode doesn't have), on the `g` prefix instead of a separate bracket mode. Each uses the same letter as its `m i`/`m a` text object, lowercase forward and uppercase backward: `g f`/`g F`, `g t`/`g T`, `g a`/`g A`, `g c`/`g C`, `g u`/`g U` (unit test), `g v`/`g V` (value), `g x`/`g X` (tag pair).

## What we took from Helix

HUME adopts several features from Helix rather than reinventing them:

- **Tree-sitter grammars**: instead of curating its own grammar repository list, HUME follows a Helix commit. Its prebuilt grammars come from the grammar sources and revisions Helix's `languages.toml` names at that commit, with Helix's tree-sitter queries (highlighting, injections, text objects and indentation), and HUME's language extensions and file-glob associations come from the same commit.
- **Language server registrations**: the same script and pinned Helix commit also sync which servers each language uses and in what order, how each server is started, its project root markers, its default settings, and any per-language feature limits (`only-features`/`except-features`, with Helix's own feature names). [`:lsp-install`](lsp.md#installing-servers) reads this list; `set-language-servers!` overrides it per language (see [Choosing which server a language uses](lsp.md#choosing-which-server-a-language-uses)).
- **Helix-style surround**: the opt-in `core:helix-surround` plugin remaps surround operations to `ms` (wrap), `md` (delete), and `mr` (replace), matching Helix's keybindings. HUME's default surround follows its own select-then-act model.
- **Kitty keyboard protocol support**: HUME detects and uses the protocol the same way on Unix and Windows terminals, falling back to legacy key encoding where the protocol isn't available.
- **Cursor shape**: Insert mode defaults to a thin bar, as Helix does with `insert = "bar"`; Normal and Extend mode are always a block. See `insert-cursor-shape` in [Global options](settings.md#global-options) for the other shapes.

## What Helix has that HUME doesn't

- Duplicating a selection onto the line *above* (`Alt-C`); HUME only binds the downward form (`C`)
- `*` searching the selection with automatic word-boundary anchors. HUME's `Ctrl-/` matches a selection literally (Helix's `Alt-*`), and HUME's own `*` searches the word under the cursor instead of the selection. See [Search](#multiple-selections)
- Rainbow bracket highlighting: HUME reads the rest of a theme that defines a top-level `rainbow` array but has no feature to act on it
- Inserting or appending a command's output (`!`, `Alt-!`), piping selections to a command and ignoring its output (`Alt-|`), and filtering selections by a command's exit status (`$`). HUME's `|` pipes each selection through a command, as Helix's does, and in Insert mode `Ctrl-|` (kitty only) inserts a command's output at each cursor. See [Shell commands](editing.md#shell-commands)
- Merging every selection into one span across gaps (`Alt--`). HUME merges only selections that share a character, automatically; touching selections stay separate
- `:x` and `:o`/`:open` as command aliases; use `:wq`/`:write-quit` and `:e` instead

## What HUME has that Helix doesn't

- A built-in plugin manager ([PLUM](core-plugins.md#core-plum)) shipping in every release, installing plugins from GitHub; Helix's Steel plugin system is still an unmerged branch
- [Tab pages](files-and-buffers.md#tabs): a saved whole pane layout, distinct from a strip of buffers
- [Smart paste](copy-and-paste.md#smart-p-paste) with kill ring
- [Hook system](writing-plugins.md#hooks) (on-buffer-open, on-buffer-save, etc.)
- [One-shot extends](#growing-selections) that extend a single motion without switching to a select/extend mode
- Backward line-selection shrinking (`X` in Extend mode, or `Ctrl-Shift-x` with kitty; see [Line selection](#line-selection-x-vs-extend-mode-e))
- A structural text object and navigation kind for array/tuple/struct values ([`m i v`/`m a v`](selections.md#text-objects), [`g v`/`g V`](moving-around.md#structural-navigation))
- Tag pair objects and navigation ([`m i x`/`m a x`](selections.md#text-objects), [`g x`/`g X`](moving-around.md#structural-navigation)) that also work in a buffer with no markup grammar
- [Custom statusline elements](settings.md#custom-elements) written in Scheme
- Per-cursor shape and theme coloring for every multi-cursor, not just the primary one (see [Global options](settings.md#global-options))
- A real programming language for configuration ([`init.scm`](configuration.md)): conditionals, loops, and custom commands, not static TOML
