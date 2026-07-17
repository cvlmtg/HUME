# Coming From Helix

HUME shares Helix's core editing model — select-then-act, selections as first-class citizens — so the mental shift is small. The differences are mostly in depth, configurability, and tooling.

## What's the same

- Select-then-act: motions change the selection; operators act on it
- Selections are always visible and always cover at least one character
- `:` command line, `/` search
- `d`, `c`, `y`, `p` for delete/change/yank/paste
- `u` / `U` undo / redo

::: tip
Unlike Helix, HUME's `c` keeps the selection on the text you changed: select a word, change it, and once you leave Insert mode the new text is still selected, ready to act on again — delete it, surround it, search for it. Disable this with the `select-changed-text` option (see [Configuration](configuration.md)).
:::

## Key differences

### Word motions

`w`, `b`: Both editors re-anchor on each press (the anchor moves with the head — it does not stay pinned at the origin). Helix selects the gap traversed — from the old position to the next word start, including the trailing whitespace. HUME selects the destination word itself and, by default, its surrounding whitespace too (trailing if there is any, otherwise the whitespace before it) — the same span `maw`/`maW` would select on that word. In the common case of words separated by single spaces the two editors land on visually the same span; they diverge at line ends and around punctuation, where Helix's "gap traversed" and HUME's "around the destination word" compute different things. Turn off `word-selects-whitespace` (see [Configuration](configuration.md)) for HUME's older bare-word behavior instead.

<div style="font-family:var(--vp-font-family-mono);line-height:2;overflow-x:auto">
<strong>Cursor on the first character</strong><br>
Helix&nbsp;&nbsp;<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">L</span>orem ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">L</span>orem ipsum dolor sit<br>
<br>
<strong>Press <code>w</code></strong><br>
Helix&nbsp;&nbsp;<span style="background:var(--vp-c-brand-soft);border-radius:3px">Lorem<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">&nbsp;</span></span>ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;Lorem <span style="background:var(--vp-c-brand-soft);border-radius:3px">ipsum<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">&nbsp;</span></span>dolor sit<br>
<br>
<strong>Press <code>w</code> again</strong><br>
Helix&nbsp;&nbsp;Lorem <span style="background:var(--vp-c-brand-soft);border-radius:3px">ipsum<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">&nbsp;</span></span>dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;Lorem ipsum <span style="background:var(--vp-c-brand-soft);border-radius:3px">dolor<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">&nbsp;</span></span>sit
</div>

To select the word the cursor is already sitting on — no forward jump — HUME binds `mm`. By default it selects the whole word plus surrounding whitespace, same as `maw`, no matter where in the word the cursor sits; with `word-selects-whitespace` off it behaves like `miw` (bare word) instead. Helix has no dedicated command for this, but `e` (move to end of word) reaches a similar result *only when the cursor already sits on the word's first character* — unlike `w`, `e` always excludes the trailing whitespace and, starting from the middle of a word, selects only from that point to the word's end (`rem`), not the whole word. `mm` has neither restriction.

<div style="font-family:var(--vp-font-family-mono);line-height:2;overflow-x:auto">
<strong>Select the current word, cursor in the middle of the word</strong><br>
Helix&nbsp;&nbsp;Lo<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">r</span>em ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;Lo<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">r</span>em ipsum dolor sit<br>
<br>
<strong>Press <code>e</code></strong><br>
Helix&nbsp;&nbsp;Lo<span style="background:var(--vp-c-brand-soft);border-radius:3px">re<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">m</span></span> ipsum dolor sit<br>
<strong>Press <code>mm</code></strong><br>
HUME&nbsp;&nbsp;&nbsp;<span style="background:var(--vp-c-brand-soft);border-radius:3px">Lorem<span style="background:var(--vp-c-brand-1);color:var(--vp-c-bg);border-radius:3px">&nbsp;</span></span>ipsum dolor sit
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
| Remove primary selection | `Alt-,` | `Ctrl+,` |
| Flip selections | `Alt-;` (Normal and Select mode) | `Ctrl+e` (Normal and Extend mode) |
| Merge consecutive selections | `Alt-_` (touching selections only); `Alt--` merges all into one span | automatic — adjacent selections never persist |
| Align selections | `&` | `&` |
| Trim whitespace at edges | `_` | `_` |
| Select within (regex per selection) | `s` | `s` |
| Select all search matches | no dedicated key — `%` (select whole buffer) then `s` (sub-select regex matches) | `m /` |
| Use selection as search pattern | `*` (adds word-boundary anchors; `Alt-*` for the literal selection) | `Ctrl+/` (kitty only) |

::: warning
HUME's `*` is not the same operation as Helix's — it's Vim-style, searching the whole word under the cursor rather than the literal selection. `Ctrl+/` is HUME's equivalent of Helix's `*`.

To get Helix's exact `*` back on the `*` key, rebind it to `search-selection` in your `init.scm`:

```scheme
(bind-key! 'normal "*" "search-selection")
```
:::

### Configuration language

Helix uses TOML. HUME uses **Scheme** (`init.scm`). You bind keys and set options by calling Scheme functions:

```scheme
(set-option! "theme" "sand")
(bind-key! 'normal "ctrl-j" "move-down")
```

This makes HUME's config a real programming language — conditionals, loops, and abstraction are available from day one.

### Plugin system

Helix has no built-in plugin system. HUME has [PLUM](core-plugins.md#plum), a plugin manager where plugins are Steel (Scheme) scripts loaded from GitHub:

```scheme
(load-plugin "username/my-plugin")
```

### Statusline

Helix's statusline is configurable via TOML (`[editor.statusline]`) — you can reorder and toggle a fixed set of built-in elements (mode, file name, diagnostics, position, etc.) across left/center/right zones. HUME's statusline is scripted from Scheme, so providers can compute and display arbitrary custom content, not just rearrange a built-in list:

```scheme
(configure-statusline! '("Mode" "FileName") '("SearchMatches") '("Position"))
```

### Surround

Helix uses `ms`, `md`, `mr` for surround. HUME supports both defaults and a Helix-compatible mode:

| Action | HUME (default) | HUME (helix-surround plugin) |
|--------|---------------|------------------------------|
| Wrap | `mw` + char | `ms` + char |
| Delete | (unbound) | `md` + char |
| Replace | (unbound) | `mr` + char |

Enable the Helix-style bindings by loading the built-in plugin:

```scheme
(load-plugin "core:helix-surround")
```

### What we took from Helix

Several features were intentionally adopted from Helix rather than reinvented:

- **Tree-sitter grammars** — Rather than curating our own grammar repository list, HUME pins a Helix commit and syncs grammar sources, revisions, language extensions, and file-glob associations from Helix's `languages.toml` via a script. Tree-sitter highlight queries are fetched directly from Helix's repository at the pinned revision at install time.
- **Helix-style surround** — The `core:helix-surround` plugin remaps surround operations to `ms` (wrap), `md` (delete), and `mr` (replace), matching Helix's keybindings. This is opt-in; HUME's default surround follows its own select-then-act model.
- **Theme format** — Helix uses TOML with `[palette]` indirection and dot-separated UI scope names. HUME's theme loader is fully Helix-compatible, accepting the same file format, modifier names (`crossed_out`, `underlined`), and extended underline syntax. This lets the community share themes between both editors.

HUME also ships a theme editor — a single-file HTML tool you can open in a browser to edit themes visually and export them as TOML. You can download it from https://github.com/cvlmtg/HUME/blob/main/tools/theme-editor/index.html

### What HUME has that Helix doesn't

- Scripting and plugins (Steel/Scheme)
- An undo tree (branching history preserved)
- Smart paste with kill ring
- Fully configurable statusline
- Hook system (on-buffer-open, on-buffer-save, etc.)
