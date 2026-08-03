# Coming From Vim / Neovim

If you know Vim or Neovim, HUME will feel different in many ways. This page covers the key differences to help you reorient quickly. For a hands-on introduction, run `:tutor` inside the editor.

## The biggest difference: select-then-act

In Vim, most operators work on a motion you specify *after* the operator: `dw` deletes a word, `ci"` changes inside quotes.

In HUME, the order is reversed: you **select first, then act**. `w` selects the next word, then `d` deletes the selection. This means:

- Motions always change the selection before anything else
- Operators (`d`, `c`, `y`, …) act on whatever is currently selected
- The selection is always visible — there is no invisible "cursor as a point"

<div class="key-demo">
<strong>Cursor on the first character, press <code>w</code></strong><br>
Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
<strong>Press <code>d</code></strong><br>
Lorem<span class="head">&nbsp;</span>dolor sit
</div>

You see the selection `w` built — word plus its leading whitespace — before `d` ever runs, instead of composing `dw` blind and finding out what it did after the fact.

## Mode map

| Vim mode | HUME equivalent |
|----------|----------------|
| Normal | Normal |
| Insert | Insert |
| Visual | Extend mode (`e`) or any motion that grows or shrinks the selection |
| Visual Line | Extend mode + line motions |
| Visual Block | HUME has no rectangular selection. Use `C` to spawn column-aligned multi-cursors (one per line below), then edit — this approximates column editing without a true visual block. |
| Command&nbsp;line | Command line (`:`) |

## Key differences

### Extend mode vs Visual mode

Vim's Visual mode is entered once and stays until you act. HUME's Extend mode is similar — press `e` to enter it, and every motion extends the selection until you act or press `Esc`. Motions run backward too: moving back toward where you started shrinks the selection, much like shrinking a Visual selection by moving back in Vim.

### Muscle-memory traps

These keys exist in both editors and do different things. They are the ones most likely to bite.

| Key | In Vim | In HUME | Vim's behaviour instead |
|-----|--------|---------|-------------------------|
| `x` | Delete character | Select the current line | `d` — the cursor is already a one-character selection |
| `s` | Substitute character | Filter each selection by a regex | `c` |
| `e` | Move to end of word | Toggle Extend mode | `m m` selects the word under the cursor |
| `%` | Jump to matching bracket | Select the whole buffer | `m s` + delimiter selects the surrounding pair |
| `;` | Repeat last `f`/`t` | Collapse the selection | `=` |
| `,` | Repeat last `f`/`t` backward | Keep only the primary selection | `-` |
| `m` | Set a mark | Text-object prefix | — HUME has no marks; `Ctrl+o` / `Ctrl+i` walk the jump list |
| `[` / `]` | Bracket-motion prefix | Cycle the kill ring after a paste | — |
| `S` | Change whole line | Split selections on newlines | `x` then `c` |
| `C` | Change to end of line | Copy the selection to the line below | `ctrl-g l c`, or `C` with `core:vim-keybind` |
| `D` | Delete to end of line | *(unbound)* | `ctrl-g l d`, or `D` with `core:vim-keybind` |
| `U` | Undo the whole line | Redo | — `Ctrl+r` also redoes |

`f`, `F`, `t`, `T` behave as they do in Vim, and `{` / `}` are still paragraph motions. Only the *repeat* keys moved: use `=` and `-`, because `;` and `,` are taken.

### Text objects

Vim's `iw` / `aw` family lives behind the `m` prefix, and — as everywhere else — you select first and act second. `diw` becomes `m i w` then `d`.

| Vim | HUME |
|-----|------|
| `diw` | `m i w` then `d` |
| `ciw` | `m i w` then `c` |
| `ci"` | `m i "` then `c` |
| `da(` | `m a (` then `d` |
| `dat` / `dap` | — no tag, paragraph, or sentence objects |

The available objects are word (`w`), WORD (`W`), the bracket pairs (`(`, `[`, `{`, `<`), the quote pairs (`"`, `'`, `` ` ``), argument (`a`), and line (`l`) — each with an `i` (inner) and `a` (around) form. One extra has no Vim equivalent: `m i i` selects the text you typed during your last insert.

### Search and replace

There is no `:s`. Substitution is a selection built up and then changed:

```
%          select the whole buffer
s          filter the selection by a regex
old        type the pattern — one selection per match appears as you type
<Enter>    keep those selections
c          change them all at once
new<Esc>   type the replacement
```

That replaces Vim's `:%s/old/new/g`. To scope it to a region instead of the file, select the region first and skip the `%`. To replace only some matches, drop the ones you don't want with `,` and `(` / `)` before pressing `c`.

`m /` is the other route: search with `/pattern` first, then `m /` turns every match in the buffer into a selection.

::: warning
`s` needs something wider than a cursor to filter — on a bare one-character selection it does nothing at all. Press `%` or select a region first.
:::

Vim's confirm-each-match flag (`:%s/…/gc`) has no equivalent, but you see every match selected before you commit to changing it.

### Registers

HUME replaces Vim's letter registers (`a`–`z`) with a small set of mnemonic single-character names and digit registers `"0`–`"9`. The default paste (`p`) is smart: it reads from the kill ring while nothing has been edited since your last `d`/`c`/`y`, and from the system clipboard once something has. After `y` this still pastes the text you just yanked, because `y` writes the clipboard as well as the kill ring. Yanking to an explicit non-default register instead (`"0y`) leaves both untouched — `p` behaves exactly as it would have without the `"0y` — so use `"0p` to paste from the named register.

| Name | HUME function | Vim equivalent |
|------|---------------|----------------|
| `"0`–`"9` | Numbered storage — text *or* macros, last write wins | `"0`–`"9` |
| `"k` | Kill-ring head (most recent yank/delete) | — |
| `"c` | System clipboard | `"+` |
| `"b` | Black hole — writes discarded | `"_` |

Two more registers exist but can't be typed after `"`: the search register (the last pattern, vim's `"/`), written by `/`, `?` and `*`; and the macro register `q`, written by `Q` recording. You reach both through the commands that use them, not through the `"` prefix.

`[` and `]` only do something immediately after a paste — they swap the pasted text for an older or newer kill-ring entry. Pressed at any other time they do nothing.

::: warning
Letter registers `a`–`z` other than the special names above do not exist. All yanks and deletes go to the kill ring (`k`) and digit registers (`0`–`9`).
:::

See [Register prefix](copy-and-paste.md#register-prefix) for the full syntax and canonical register list.

### Macros

Macros work similarly but with different key triggers:

| Vim | HUME |
|-----|------|
| `qa` … `q` | `Q<reg>` … `Q` |
| `@a` | `q<reg>` |
| `@@` | — (use `q<reg>` again or `qq` for the default register) |

In Vim, `q` starts and stops recording, then `@` plays. In HUME, recording is started and stopped with `Q`; playback uses `q`. Valid macro registers are `q` and `0`–`9` (the default register is `q`: record with `QQ`, play with `qq`). There is no equivalent to Vim's `@@` (repeat last played register).

### Dot-repeat

Vim's `.` repeats the last change. HUME's `.` works the same way — it repeats the last editing command or insert session.

### Count prefix

Vim uses `[count]` before commands (e.g. `3dw`). HUME also supports count prefixes (`1`–`9`):

| Vim | HUME |
|-----|------|
| `3w` | `3w` |
| `5j` | `5j` |
| `d3w` | `3w` then `d` (select first, then act) |

### Line motion

HUME's idiom for line motions is the `g` prefix: `g h` (start), `g l` (end), `g s` (first non-blank), `g e` (last line). The vim keys `0` / `$` / `^` / `G` are not bound by default — load `(load-plugin "core:stdlib")` then `(load-plugin "core:vim-keybind")` in `init.scm` to get them back with their vim meaning, alongside `C` / `D` (change / delete to end of line) and `Ctrl+6` (see below).

| Vim | HUME (native) | HUME (`core:vim-keybind`) |
|-----|----------------|---------------------------|
| `0` / `$` / `^` | `g h` / `g l` / `g s` | `0` / `$` / `^` |
| `G` | `g e` | `G` |
| `gg` | `g g` | `g g` |
| `C` / `D` | `ctrl-g l c` / `ctrl-g l d` (kitty terminals only) | `C` (change to end of line on a bare cursor; with a selection, falls back to the default `copy-selection-on-next-line`) / `D` |
| `o` (visual mode) | `Ctrl+e` (flips anchor and head, any mode) | `o` (in Extend mode) |

## Commands you already know

Most `:` commands work as expected:

| Vim | HUME |
|-----|------|
| `:w` | `:w` |
| `:q` / `:q!` | `:q` / `:q!` |
| `:wq` | `:wq` |
| `:e file` | `:e file` |
| `:ls` / `:buffers` | `:ls` (`:buffers` is not an alias) |
| `:bn` / `:bp` | `:bn` / `:bp` (or `:bnext` / `:bprev`) |
| `:bd` | `:bd` |
| `:cd` | `:cd` |
| `:pwd` | `:pwd` |
| `:42` | `:42` |
| `:sp` / `:vsp` | `:sp` / `:vsp` |
| `Ctrl+w` window prefix | `Ctrl+p` pane prefix |
| `Ctrl+^` | `:b #`, or `Ctrl+6` with `core:vim-keybind` loaded (kitty only) |
| `Ctrl+o` / `Ctrl+i` | `Ctrl+o` / `Ctrl+i` |
| `:set` | `:set`, with different syntax — `:set global\|buffer\|pane key=value` |
| `:help` | — `:tutor` opens the tutorial, `:messages` shows the message log |
