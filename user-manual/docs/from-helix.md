# Coming From Helix

HUME shares Helix's core editing model (select-then-act, selections as first-class citizens), so the mental shift is small. The differences are mostly in depth, configurability, and tooling.

## The first ten minutes with HUME

### Opening a file

`hume file.txt:42:5` and `:e path` work as in Helix, including `:e` on a path that doesn't exist yet: the file is created on the first `:w`.

Note: `:o` / `:open` are not aliases; use `:e`.

The fuzzy finder is a plugin, not built in. Helix's `Space f` and `Space b` become `z f` and `z b` once `core:pickers` is loaded. The [starter config](configuration.md#example-init-scm) loads it, along with language server support and other plugins, so copy that first. See [Fuzzy Finder](pickers.md).

### Editing

Copy the bundled starter config to `~/.config/hume/init.scm` ([where to find it](configuration.md#example-init-scm)) and open it on line 20 with `hume ~/.config/hume/init.scm:20`. Then make the same edits you would in Helix:

| Edit | Helix | HUME |
|------|-------|------|
| Uncomment line 20 (`core:buffer-words`) | `t(` `d` | `Ctrl-t` `(` `d` |
| Delete lines 17 and 18 (`core:plum`, `core:git-diff`) | `:17` `x` `x` `d` | `:17` `Ctrl-x` `Ctrl-x` `d` |
| Turn `declare-plugin` into `load-plugin` on line 16 | `:16` `l` `m i w` `c` `load` `Esc` | `:16` `l` `m m` `c` `load` `Esc` |
| Copy line 16 to another application | `x` `Space y` | `x` `y` |

What each step shows:

- **`t` only moves; `Ctrl` makes it select.** `f`, `F`, `t` and `T` move the cursor and select nothing, where Helix selects up to the target. Holding `Ctrl` turns a motion into a one-shot extend, so `Ctrl-t (` selects `;; ` and `d` deletes it. For longer selections, `e` toggles Extend mode (Helix's `v`). The `Ctrl` forms need the [kitty keyboard protocol](installation.md#terminal-compatibility).
- **A second `x` moves on.** Each `x` selects the next line on its own, dropping the previous one. `Ctrl-x` extends instead. See [Line selection](#line-selection-x-vs-extend-mode-e).
- **`Esc` keeps what you typed selected.** After the change, `load` is still selected instead of a plain cursor, ready to act on again: delete it, surround it, search for it. One consequence: `i` re-enters *before* that selection, so `a`, not `i`, is the key to keep typing after it. Disable this with the `select-inserted-text` option (see [Configuration](configuration.md)).
- **`y` reaches the system clipboard.** It copies to the clipboard as well as HUME's own kill ring. `p` pastes your last yank or delete while you haven't edited since, and the system clipboard once you have. See [Copy & Paste](copy-and-paste.md).

### Saving and quitting

`:w`, `:wq`, `:q!`, `:qa` and `:qa!` work as in Helix. There is no `:x`; `:write-quit` is the long form of `:wq`.

`:q` on the last pane closes the current buffer, not the editor. If other files are still open, you land on one of them, and HUME quits only when nothing is left. `:qa` leaves in one step. See [Quitting](files-and-buffers.md#quitting).

For a hands-on tour of everything else, type `:tutor` and press `Enter`: it opens an interactive tutorial you can edit freely.

## What's the same

- Select-then-act: motions change the selection; operators act on it
- Selections are always visible and always cover at least one character
- `:` command mode prompt, `/` search
- `d`, `c`, `y`, `p` for delete/change/yank/paste
- `u` / `U` undo / redo
- `>` / `<` indent / unindent the lines a selection touches, with count support (`3>`). HUME additionally re-renders each touched line's whole indent to the buffer's `tab-width`/`tab-style` rather than only prepending or trimming a fixed amount, so a mixed-tabs-and-spaces indent gets normalized as a side effect
- `m i f`/`m a f`, `m i t`/`m a t`, `m i a`/`m a a`, `m i c`/`m a c`, `m i u`/`m a u`: the same letters as Helix's own match-mode textobjects except unit test (`T`→`u`; see below), selecting the enclosing function, class/type, argument, comment, or unit test for any language with a tree-sitter grammar that ships a textobjects query

### Themes

Helix uses TOML with `[palette]` indirection and dot-separated UI scope names. HUME's theme loader reads the same file format, the same color forms (hex, a palette name, or one of the sixteen bare terminal color names like `red`), the same modifier names, and the same underline styles, so most Helix themes work in HUME unchanged.

::: details Where HUME's theme format differs
A terminal color name resolves to a fixed value from the standard terminal palette rather than to your own terminal's configured color, so a theme reads the same everywhere.

One thing a theme can contain isn't supported, though it doesn't stop the rest of the theme from loading: the top-level `rainbow` array, which HUME has no rainbow-bracket feature to read. A theme fails to load outright only when the problem is with the document rather than one entry in it: invalid TOML, an `inherits` parent that doesn't exist or forms a cycle or nests more than eight deep, or an `inherits`/`palette` key of the wrong type. Any other malformed entry is left unstyled and named in `:messages` instead. See [Theme scopes](configuration.md#theme-scopes) for the scopes HUME doesn't read.

HUME also extends the format in one direction: a scope can be written as a TOML section header (`[ui.cursor]`) where Helix only reads flat dotted keys. A theme hand-authored in HUME using section headers needs them flattened to plain dotted keys before Helix will take it. The theme editor's own exports are already flat, so this only matters for a theme you write by hand.
:::

Themes install the same way plugins do: run `:plum-install-theme <user/repo>` and [PLUM](core-plugins.md#core-plum) fetches the theme repo from GitHub.

Then type `:theme ` and press Tab to try it for the session, or set it as your default theme in your [`init.scm`](configuration.md#example-init-scm): Here's [Everforest](https://github.com/cvlmtg/everforest.hume), the first third-party theme for HUME, ported as-is from Helix.

```scheme
(set-option! "theme" "everforest_dark")
```


A theme editor is also available online: a single-file HTML tool you download and open in a browser to edit themes visually and export them as TOML: https://raw.githubusercontent.com/cvlmtg/HUME/main/tools/theme-editor/index.html

## Key differences

### Plugin system

Helix's Steel plugin system is still an unmerged branch; HUME's ships in every release. HUME's plugins are Steel too, the same Scheme dialect, but plugins written for Helix's Steel branch won't run in HUME: the two editors expose different functions to scripts. [PLUM](core-plugins.md#core-plum) is HUME's plugin manager, installing plugins from GitHub. Declare the plugin in your [`init.scm`](configuration.md#example-init-scm), then run `:plum-install-plugins` to fetch it. Here's [grep.hume](https://github.com/cvlmtg/grep.hume), a live-grep picker and HUME's first official third-party plugin:

```scheme
(declare-plugin "core:stdlib")
(load-plugin "cvlmtg/grep.hume")
```

### Growing selections

Helix grows a selection in select mode (`v`). HUME's equivalent is Extend mode (`e`), where every motion extends until you act or press `Esc`. HUME also has one-shot extends (`Ctrl-w`, `Ctrl-b`, …) that extend for a single keystroke without changing mode; these need the kitty keyboard protocol.

### Line selection: `x` vs Extend mode (`e`)

Both editors bind `x` to select the current line. The difference is depth:

| Press | Helix `x` | HUME `x` | HUME `e` then `x` |
|-------|-----------|----------|-------------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

Helix's `x` is **modal**: once pressed, all subsequent `x` presses extend the selection line-wise until you cancel.

HUME's `x` is **one-shot**. Each press re-anchors to the next line. To get Helix's repeat-extend behavior, enter **Extend mode** first (`e`). In Extend mode, `x` (and every other motion) extends rather than replaces. Use `Ctrl-x` for a one-shot extend without entering the mode.

`X` is the backward form: after growing a line selection downward, `X` (or `Ctrl-Shift-x`) shrinks it back up one line at a time. Helix has no key for that.

### Multiple selections

Both editors share the same foundations (multiple cursors, `;` to collapse, `S` to split into lines), but keybindings and a few operations differ:

| Operation | Helix | HUME |
|-----------|-------|------|
| Copy selection on line below | `C` | `C` (duplicates each selection to the same column on the next line, adding a multi-cursor; column-style editing via multi-cursor, not a rectangular visual block) |
| Copy selection on line above | `Alt-C` | (unbound) |
| Remove primary selection | `Alt-,` | `Ctrl-,` (kitty only) |
| Flip selections | `Alt-;` (Normal and Select mode) | `Ctrl-e` (Normal and Extend mode) |
| Merge consecutive selections | `Alt-_` (touching selections only); `Alt--` merges all into one span | automatic: adjacent selections never persist |
| Align selections | `&` | `&` |
| Trim whitespace at edges | `_` | `_` |
| Sort | `:sort` | `:sort` (different semantics; see below) |
| Sift within (regex per selection) | `s` | `s` |
| Select all search matches | no dedicated key; `%` (select whole buffer) then `s` (sift to regex matches) | `m /` |
| Search selection, auto word-boundary anchors | `*` | *(none)* |
| Search word under cursor (Vim-style) | *(unbound)* | `*` |
| Search selection literally, no anchors | `Alt-*` | `Ctrl-/` (kitty only) |

::: warning
HUME's `*` is Vim-style: it expands to the word (or punctuation run) under the cursor and ignores any existing selection, unlike Helix's `*`, which searches the literal current selection. HUME's `Ctrl-/` (kitty only) is the closer match to Helix's `Alt-*`.

To bind that behavior to `*` instead, rebind it in your `init.scm`:

```scheme
(bind-key! 'normal "*" "search-selection")
```
:::

::: warning
HUME's `:sort` permutes whole rows, keyed by whatever text you select on each one. Select whole lines and it behaves like a plain line sort; select a column across several lines and it sorts by that column, moving the entire rows along with it. `%` followed by `:sort` sorts the whole file directly, with no splitting step needed.
:::

### Configuration language

Helix uses TOML. HUME uses **Scheme** ([`init.scm`](configuration.md)). You bind keys and set options by calling Scheme functions:

```scheme
(set-option! "theme" "sand")
(bind-key! 'normal "ctrl-j" "move-down")
```

This makes HUME's config a real programming language: conditionals, loops, and abstraction are available from day one.

### Statusline

Helix's statusline is configurable via TOML (`[editor.statusline]`); HUME's is configured from Scheme. Both work the same way in practice: you reorder and toggle a fixed set of built-in elements across left/center/right zones:

```scheme
(configure-statusline! '("Mode" "FileName") '("SearchMatches") '("Position"))
```

You can also add your own custom elements from Scheme; see [Statusline](configuration.md#custom-elements).

### Bufferline vs tab bar

Helix's `bufferline` (`never` / `always` / `multiple`) is a strip of open **buffers**: pick one and it swaps into the focused split. HUME's bar lists **tab pages** instead: a tab is a saved window layout (its own splits and focused pane), so switching tabs swaps the whole pane arrangement, not a single buffer. Buffers stay global across tabs and are reached with `:ls` / `:b` / `:bn` / `:bp`, not from the bar. See [Tabs](files-and-buffers.md#tabs).

`:set global tabline=never/always/dynamic` controls visibility; `dynamic` (the default) is the analogue of Helix's `multiple`, showing the bar only once a second tab is open. Click a tab to switch to it; the bar scrolls when tabs overflow the width.

A Helix theme needs no changes to look right: HUME's `ui.tabline` falls back to Helix's `ui.bufferline` when unset; see [Theme scopes](configuration.md#theme-scopes).

### Surround

Helix uses `ms`, `md`, `mr` for surround. HUME supports both defaults and a Helix-compatible mode:

| Action | HUME (default) | HUME (helix-surround plugin) |
|--------|---------------|------------------------------|
| Wrap | `mw` + char | `ms` + char |
| Delete | `ms` + char, then `d` | `md` + char |
| Replace | `ms` + char, then `r` | `mr` + char |

By default there's no dedicated delete or replace key because you don't need one: `ms` selects the surrounding pair, and then the ordinary `d` and `r` act on it. Loading the plugin swaps that trade: it takes `ms` over for wrapping and removes `mw`.

Enable the Helix-style bindings by loading the built-in plugin:

```scheme
(load-plugin "core:helix-surround")
```

### Matching brackets and structural navigation

Helix's match mode binds `m m` to jump to the matching bracket. HUME binds the same idea directly to `#`, with no mode step, and also matches HTML/XML/JSX tag pairs, without disturbing `%` (select whole buffer) in either editor.

Helix's bracket mode also binds `]f`/`[f`, `]t`/`[t`, `]a`/`[a`, `]c`/`[c`, and `]T`/`[T` by default, jumping straight to the next/previous function, class, argument, comment, or unit test. HUME puts the same six kinds (plus `value`, for array/tuple/struct entries, which Helix's bracket mode doesn't have) on the `g` prefix instead of a separate bracket mode: lowercase jumps forward, uppercase jumps backward, on the same letter as the `m i`/`m a` text object: `g f`/`g F`, `g t`/`g T`, `g a`/`g A`, `g c`/`g C`, `g u`/`g U` (unit test), `g v`/`g V` (value).

## What we took from Helix

Several features were intentionally adopted from Helix rather than reinvented:

- **Tree-sitter grammars**: rather than curating our own grammar repository list, HUME pins a Helix commit and syncs grammar sources, revisions, language extensions, and file-glob associations from Helix's `languages.toml` via a script. Tree-sitter highlight queries are fetched directly from Helix's repository at the pinned revision at install time.
- **Helix-style surround**: the `core:helix-surround` plugin remaps surround operations to `ms` (wrap), `md` (delete), and `mr` (replace), matching Helix's keybindings. This is opt-in; HUME's default surround follows its own select-then-act model.
- **Kitty keyboard protocol support**: HUME uses the `termina` crate so the same detection and encoding work consistently on Unix and Windows terminals alike, falling back to legacy key encoding where the protocol isn't available.
- **Cursor shape**: HUME's Insert-mode cursor defaults to a thin bar, matching the look Helix gives you once you set `insert = "bar"` in `[editor.cursor-shape]`; set `cursor-shape-insert` to `block` or `underline` for the other two shapes (see [Global options](configuration.md#global-options)). Normal and Extend mode have no shape setting of their own and are always a block, same as Helix's own default for every mode it doesn't override. HUME departs from Helix for multi-cursor editing: Helix always shows every extra cursor as a colored block regardless of shape, since only one of them can ever be the real terminal cursor. HUME instead applies `cursor-shape-insert` to every cursor alike. With `block`, each one is painted from the theme's own cursor colors (`ui.cursor.insert`/`ui.cursor.primary.insert`, and their Normal/Extend equivalents); with `bar` or `underline`, only the real terminal cursor marks the primary one, and the rest are visible only where they sit inside a highlighted selection.

## What Helix has that HUME doesn't

- Duplicating a selection onto the line *above* (`Alt-C`); HUME only binds the downward form (`C`)
- `*` searching the selection with automatic word-boundary anchors: HUME's `Ctrl-/` matches a selection literally (Helix's `Alt-*`), and HUME's own `*` searches the word under the cursor instead of the selection. See [Search](#multiple-selections)
- Rainbow bracket highlighting: HUME reads the rest of a theme that defines a top-level `rainbow` array but has no feature to act on it
- Shell pipe integration: piping selections through a command (`|`), inserting a command's output (`!`), or filtering selections by a shell command's exit status (`$`). Anything you'd reach for these for is written in Scheme instead, running inside the editor with direct access to buffers, selections and commands
- Merging every selection into one span across gaps (`Alt--`); HUME only merges selections that already touch, automatically
- `:x` and `:o`/`:open` as command aliases; use `:wq`/`:write-quit` and `:e` instead

## What HUME has that Helix doesn't

- A built-in plugin manager ([PLUM](core-plugins.md#core-plum)) shipping in every release, installing plugins from GitHub; Helix's Steel plugin system is still an unmerged branch
- [Tab pages](files-and-buffers.md#tabs): a saved whole pane layout, distinct from a strip of buffers
- [Smart paste](copy-and-paste.md#smart-p-paste) with kill ring
- [Hook system](plugins.md#hooks) (on-buffer-open, on-buffer-save, etc.)
- [One-shot extends](#growing-selections) that extend a single motion without switching to a select/extend mode
- Backward line-selection shrinking (`X`/`Ctrl-Shift-x`; see [Line selection](#line-selection-x-vs-extend-mode-e))
- A structural text object and navigation kind for array/tuple/struct values ([`m i v`/`m a v`](selections.md#text-objects), [`g v`/`g V`](moving-around.md#structural-navigation))
- [Custom statusline elements](configuration.md#custom-elements) written in Scheme
- Per-cursor shape and theme coloring for every multi-cursor, not just the primary one (see [Global options](configuration.md#global-options))
- A real programming language for configuration ([`init.scm`](configuration.md)): conditionals, loops, and custom commands, not static TOML
