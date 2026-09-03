# Coming From Helix

HUME shares Helix's core editing model — select-then-act, selections as first-class citizens — so the mental shift is small. The differences are mostly in depth, configurability, and tooling.

## What's the same

- Select-then-act: motions change the selection; operators act on it
- Selections are always visible and always cover at least one character
- `:` command mode prompt, `/` search
- `d`, `c`, `y`, `p` for delete/change/yank/paste
- `u` / `U` undo / redo
- `>` / `<` indent / unindent the lines a selection touches, with count support (`3>`) — HUME additionally re-renders each touched line's whole indent to the buffer's `tab-width`/`tab-style` rather than only prepending or trimming a fixed amount, so a mixed-tabs-and-spaces indent gets normalized as a side effect
- `m i f`/`m a f`, `m i t`/`m a t`, `m i a`/`m a a`, `m i c`/`m a c`, `m i u`/`m a u` — the same letters as Helix's own match-mode textobjects except unit test (`T`→`u`; see below), selecting the enclosing function, class/type, argument, comment, or unit test for any language with a tree-sitter grammar that ships a textobjects query

::: tip
Unlike Helix, HUME's `c` keeps the selection on the text you changed: select a word, change it, and once you leave Insert mode the new text is still selected, ready to act on again — delete it, surround it, search for it. Disable this with the `select-changed-text` option (see [Configuration](configuration.md)).
:::

## Key differences

### Word motions

`w`, `b`: Both editors re-anchor on each press (the anchor moves with the head — it does not stay pinned at the origin). Helix selects the gap traversed — from the old position to the next word start, including the trailing whitespace. HUME selects the destination word itself and, by default, the whitespace *before* it too — except the first word of a line, which takes its trailing whitespace instead, since a leading run there would be indentation. In the common case of words separated by single spaces the two editors land on visually similar spans; they diverge in exactly where the whitespace sits (leading for HUME vs. trailing for Helix's traversed gap) and around punctuation or line ends, where the two models compute different things outright. Turn off `word-selects-whitespace` (see [Configuration](configuration.md)) for HUME's bare-word behavior instead.

<div class="key-demo">
<strong>Cursor on the first character</strong><br>
Helix&nbsp;&nbsp;<span class="head">L</span>orem ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;<span class="head">L</span>orem ipsum dolor sit<br>
<br>
<strong>Press <code>w</code></strong><br>
Helix&nbsp;&nbsp;<span class="sel">Lorem<span class="head">&nbsp;</span></span>ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
<br>
<strong>Press <code>w</code> again</strong><br>
Helix&nbsp;&nbsp;Lorem <span class="sel">ipsum<span class="head">&nbsp;</span></span>dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;Lorem ipsum<span class="sel">&nbsp;dolo<span class="head">r</span></span> sit
</div>

To select the word the cursor is already sitting on — no forward jump — HUME binds `mm`. By default it selects the whole word plus one adjacent whitespace run (same rule as `w`/`b` above), no matter where in the word the cursor sits. Helix's closest equivalent is `maw`, match mode's around-word text object: it also grabs the whole word plus one whitespace run, independent of cursor position. The two pick sides differently — `maw` reaches for trailing whitespace first and only falls back to leading whitespace if the word has none, while `mm` reaches for leading whitespace first and switches to trailing only for a line's first word (where a leading run would be indentation) — so they agree at line starts and for a line's last word, but land on opposite sides mid-line. `miw` (inner word, no whitespace) matches `mm` exactly, but only once `word-selects-whitespace` is turned off. `e` (move to end of word) only approximates `mm` when the cursor already sits on the word's first character — from the middle of a word it instead selects just cursor→end (`rem`), not the whole word.

<div class="key-demo">
<strong>Select the current word, cursor in the middle of the word</strong><br>
Helix&nbsp;&nbsp;Lorem ip<span class="head">s</span>um dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;Lorem ip<span class="head">s</span>um dolor sit<br>
<br>
<strong>Press <code>maw</code></strong><br>
Helix&nbsp;&nbsp;Lorem <span class="sel">ipsum<span class="head">&nbsp;</span></span>dolor sit<br>
<strong>Press <code>mm</code></strong><br>
HUME&nbsp;&nbsp;&nbsp;Lorem<span class="sel"> ipsu<span class="head">m</span></span> dolor sit
</div>

### Growing selections

To grow a selection across multiple words in HUME, use Extend mode (`e` then `w`), or a one-shot extend (`Ctrl+w` under the kitty protocol). Both editors can shrink a grown selection back the same way: since extending keeps the anchor fixed and only moves the head, reversing direction (`b`/`Ctrl+b` after `w`/`Ctrl+w` in HUME; `b` after `w` in Helix's select mode) moves the head back toward the anchor instead of growing further. What differs is how you get into extending: Helix requires pressing `v` (select mode) first, after which every motion extends until you leave the mode; HUME's Extend mode (`e`) works the same way, but HUME also offers one-shot per-keystroke extends (`Ctrl+w`/`Ctrl+b`) that skip the mode switch entirely.

### Line selection: `x` vs Extend mode (`e`)

Both editors bind `x` to select the current line. The difference is depth:

| Press | Helix `x` | HUME `x` | HUME `e` then `x` |
|-------|-----------|----------|-------------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

Helix's `x` is **modal** — once pressed, all subsequent `x` presses extend the selection line-wise until you cancel.

HUME's `x` is **one-shot**. Each press re-anchors to the next line. To get Helix's repeat-extend behavior, enter **Extend mode** first (`e`) — in Extend mode, `x` (and every other motion) extends rather than replaces. Use `Ctrl+x` for a one-shot extend without entering the mode.

Unlike its word motions, Helix's `x` doesn't share the anchor-fixed extend mechanism — it's hardcoded to always grow downward one line per press, and the default keymap has no key that shrinks a grown line selection back up (`X` normalizes the existing selection to whole-line boundaries rather than undoing a previous `x`; `Alt-x` shrinks to line bounds from an unrelated starting point). HUME's `x`/`X` are genuinely bidirectional: after growing downward with `x`/`Ctrl+x`, pressing `X`/`Ctrl+X` shrinks the selection back up one line at a time (and vice versa).

### Multiple selections

Both editors share the same foundations — multiple cursors, `;` to collapse, `S` to split into lines — but keybindings and a few operations differ:

| Operation | Helix | HUME |
|-----------|-------|------|
| Copy selection on line below | `C` | `C` (duplicates each selection to the same column on the next line, adding a multi-cursor — column-style editing via multi-cursor, not a rectangular visual block) |
| Copy selection on line above | `Alt-C` | (unbound) |
| Remove primary selection | `Alt-,` | `Ctrl+,` (kitty only) |
| Flip selections | `Alt-;` (Normal and Select mode) | `Ctrl+e` (Normal and Extend mode) |
| Merge consecutive selections | `Alt-_` (touching selections only); `Alt--` merges all into one span | automatic — adjacent selections never persist |
| Align selections | `&` | `&` |
| Trim whitespace at edges | `_` | `_` |
| Sort | `:sort` | `:sort` (different semantics — see below) |
| Select within (regex per selection) | `s` | `s` |
| Select all search matches | no dedicated key — `%` (select whole buffer) then `s` (sub-select regex matches) | `m /` |
| Search selection, auto word-boundary anchors | `*` | *(none)* |
| Search word under cursor (Vim-style) | *(unbound)* | `*` |
| Search selection literally, no anchors | `Alt-*` | `Ctrl+/` (kitty only) |

::: warning
HUME's `*` is not the same operation as Helix's `*`. Helix's `*` searches the literal current selection (or just the character under a collapsed cursor), adding `\b` anchors only when that text looks like a word — it never expands past what's already selected. HUME's `*` is Vim-style: it expands to the whole run under the cursor, ignoring any existing selection. A word gets `\b` anchors; a run of punctuation is searched literally without them, and on whitespace `*` does nothing at all.

HUME's `Ctrl+/` is the closer match — to Helix's `Alt-*` (literal selection, no anchors). HUME has no equivalent of Helix's `*` (selection-based search with automatic boundary detection).

To put `Ctrl+/`'s behavior on the `*` key instead — matching Helix's `Alt-*`, not `*` — rebind it in your `init.scm`:

```scheme
(bind-key! 'normal "*" "search-selection")
```
:::

::: warning
HUME's `:sort` permutes whole rows, keyed by whatever text you select on each one. Select whole lines and it behaves like a plain line sort; select a column across several lines and it sorts by that column, moving the entire rows along with it. `%` followed by `:sort` sorts the whole file directly — no splitting step needed.
:::

### Configuration language

Helix uses TOML. HUME uses **Scheme** ([`init.scm`](configuration.md)). You bind keys and set options by calling Scheme functions:

```scheme
(set-option! "theme" "sand")
(bind-key! 'normal "ctrl-j" "move-down")
```

This makes HUME's config a real programming language — conditionals, loops, and abstraction are available from day one.

### Plugin system

Helix has no built-in plugin system. HUME has [PLUM](core-plugins.md#core-plum), a plugin manager where plugins are Scheme scripts installed from GitHub. Declare the plugin, then run `:plum-install-plugins` to fetch it — here's [grep.hume](https://github.com/cvlmtg/grep.hume), a live-grep picker and HUME's first official third-party plugin:

```scheme
(declare-plugin "core:stdlib")
(load-plugin "cvlmtg/grep.hume")
```

### Statusline

Helix's statusline is configurable via TOML (`[editor.statusline]`); HUME's is configured from Scheme. Both work the same way in practice — you reorder and toggle a fixed set of built-in elements across left/center/right zones:

```scheme
(configure-statusline! '("Mode" "FileName") '("SearchMatches") '("Position"))
```

You can also add your own custom elements from Scheme — see [Statusline](configuration.md#custom-elements).

### Surround

Helix uses `ms`, `md`, `mr` for surround. HUME supports both defaults and a Helix-compatible mode:

| Action | HUME (default) | HUME (helix-surround plugin) |
|--------|---------------|------------------------------|
| Wrap | `mw` + char | `ms` + char |
| Delete | `ms` + char, then `d` | `md` + char |
| Replace | `ms` + char, then `r` | `mr` + char |

By default there's no dedicated delete or replace key because you don't need one: `ms` selects the surrounding pair, and then the ordinary `d` and `r` act on it. Loading the plugin swaps that trade — it takes `ms` over for wrapping and removes `mw`.

Enable the Helix-style bindings by loading the built-in plugin:

```scheme
(load-plugin "core:helix-surround")
```

### Matching brackets and structural navigation

Helix's match mode binds `m m` to jump to the matching bracket. HUME binds the same idea directly to `#` — no mode step — and also matches HTML/XML/JSX tag pairs, without disturbing `%` (select whole buffer) in either editor.

Helix's bracket mode also binds `]f`/`[f`, `]t`/`[t`, `]a`/`[a`, `]c`/`[c`, and `]T`/`[T` by default, jumping straight to the next/previous function, class, argument, comment, or unit test. HUME puts the same six kinds (plus `value`, for array/tuple/struct entries, which Helix's bracket mode doesn't have) on the `g` prefix instead of a separate bracket mode — lowercase jumps forward, uppercase jumps backward, on the same letter as the `m i`/`m a` text object: `g f`/`g F`, `g t`/`g T`, `g a`/`g A`, `g c`/`g C`, `g u`/`g U` (unit test), `g v`/`g V` (value).

### What we took from Helix

Several features were intentionally adopted from Helix rather than reinvented:

- **Tree-sitter grammars** — Rather than curating our own grammar repository list, HUME pins a Helix commit and syncs grammar sources, revisions, language extensions, and file-glob associations from Helix's `languages.toml` via a script. Tree-sitter highlight queries are fetched directly from Helix's repository at the pinned revision at install time.
- **Helix-style surround** — The `core:helix-surround` plugin remaps surround operations to `ms` (wrap), `md` (delete), and `mr` (replace), matching Helix's keybindings. This is opt-in; HUME's default surround follows its own select-then-act model.
- **Kitty keyboard protocol support** — HUME uses the `termina` crate so the same detection and encoding work consistently on Unix and Windows terminals alike, falling back to legacy key encoding where the protocol isn't available.
- **Theme format** — Helix uses TOML with `[palette]` indirection and dot-separated UI scope names. HUME's theme loader reads the same file format, modifier names (`crossed_out`, `underlined`), and extended underline syntax, so themes can be shared between both editors. One difference: palette entries must be `#rrggbb` values — a theme using terminal color names like `red` in its palette won't pick those up.

A theme editor is also available online — a single-file HTML tool you download and open in a browser to edit themes visually and export them as TOML: https://raw.githubusercontent.com/cvlmtg/HUME/main/tools/theme-editor/index.html

### What HUME has that Helix doesn't

- Scripting and plugins (Scheme)
- Smart paste with kill ring
- Hook system (on-buffer-open, on-buffer-save, etc.)
