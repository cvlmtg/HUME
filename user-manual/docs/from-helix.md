# Coming From Helix

HUME shares Helix's core editing model — select-then-act, selections as first-class citizens — so the mental shift is small. The differences are mostly in depth, configurability, and tooling.

## What's the same

- Select-then-act: motions change the selection; operators act on it
- Selections are always visible and always cover at least one character
- `:` command line, `/` search
- `d`, `c`, `y`, `p` for delete/change/yank/paste
- `u` / `U` undo / redo

## Key differences

### Word motions

- `w`, `b`: Both editors re-anchor on each press (the anchor moves with the head — it does not stay pinned at the origin). The difference is **what gets selected**: Helix selects the gap traversed (from the old position to the next word start, e.g. `Basic ` including the trailing space); HUME selects the destination word itself (from its start to its end, e.g. `forward` with no surrounding whitespace). To grow a selection across multiple words in HUME, use Extend mode (`e` then `w`), or a one-shot extend (`Ctrl+w` under the kitty protocol). Unlike Helix, HUME's extend is bidirectional: pressing `b`/`Ctrl+b` after growing with `w`/`Ctrl+w` shrinks the selection back word by word (and vice versa) instead of only ever growing it.

### Line selection: `x` vs Extend mode (`e`)

Both editors bind `x` to select the current line. The difference is depth:

| Press | Helix `x` | HUME `x` | HUME `e` then `x` |
|-------|-----------|----------|-------------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

Helix's `x` is **modal** — once pressed, all subsequent `x` presses extend the selection line-wise until you cancel.

HUME's `x` is **one-shot**. Each press re-anchors to the next line. To get Helix's repeat-extend behavior, enter **Extend mode** first (`e`) — in Extend mode, `x` (and every other motion) extends rather than replaces. Use `Ctrl+x` for a one-shot extend without entering the mode.

Once extending, HUME's `x`/`X` are also bidirectional: after growing downward with `x`/`Ctrl+x`, pressing `X`/`Ctrl+X` shrinks the selection back up one line at a time (and vice versa), rather than only ever growing.

### Multiple selections

Both editors share the same foundations — multiple cursors, `;` to collapse, `S` to split into lines — but keybindings and a few operations differ:

| Operation | Helix | HUME |
|-----------|-------|------|
| Copy selection on line below | `Alt-C` | `C` (duplicates each selection to the same column on the next line, adding a multi-cursor — column-style editing via multi-cursor, not a rectangular visual block) |
| Copy selection on line above | `Alt-c` | (unbound) |
| Remove primary selection | `Alt-,` | `Ctrl+,` |
| Flip selections | `Alt-o` (extend mode) | `o` (extend mode) |
| Merge consecutive selections | `Alt-=`, `Alt-+` | automatic — adjacent selections never persist |
| Align selections | `&` | `&` |
| Trim whitespace at edges | — | `_` |
| Select all search matches | `%` (search mode) | `m /` |
| Select within (regex per selection) | `s` (select mode) | `s` |

### Configuration language

Helix uses TOML. HUME uses **Scheme** (`init.scm`). You bind keys and set options by calling Scheme functions:

```scheme
(set-option! "theme" "ember")
(bind-key! "normal" "ctrl-j" "move-down")
```

This makes HUME's config a real programming language — conditionals, loops, and abstraction are available from day one.

### Plugin system

Helix has no built-in plugin system. HUME has [PLUM](core-plugins.md#plum), a plugin manager where plugins are Steel (Scheme) scripts loaded from GitHub:

```scheme
(load-plugin "username/my-plugin")
```

### Statusline

Helix's statusline is fixed. HUME's statusline is fully configurable from Scheme:

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

- **Theme format** — Helix uses TOML with `[palette]` indirection and dot-separated UI scope names. HUME's theme loader is fully Helix-compatible, accepting the same file format, modifier names (`crossed_out`, `underlined`), and extended underline syntax. This lets the community share themes between both editors.
- **Tree-sitter grammars** — Rather than curating our own grammar repository list, HUME pins a Helix commit and syncs grammar sources, revisions, language extensions, and file-glob associations from Helix's `languages.toml` via a script. Tree-sitter highlight queries are fetched directly from Helix's repository at the pinned revision at install time.
- **Helix-style surround** — The `core:helix-surround` plugin remaps surround operations to `ms` (wrap), `md` (delete), and `mr` (replace), matching Helix's keybindings. This is opt-in; HUME's default surround follows its own select-then-act model.

### What HUME has that Helix doesn't

- Scripting and plugins (Steel/Scheme)
- An undo tree (branching history preserved)
- Smart paste with kill ring
- Fully configurable statusline
- Hook system (on-buffer-open, on-buffer-save, etc.)
