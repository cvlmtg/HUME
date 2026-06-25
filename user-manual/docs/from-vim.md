# Coming From Vim / Neovim

If you know Vim or Neovim, HUME will feel different in many ways. This page covers the key differences to help you reorient quickly. For a hands-on introduction, run `:tutor` inside the editor.

## The biggest difference: select-then-act

In Vim, most operators work on a motion you specify *after* the operator: `dw` deletes a word, `ci"` changes inside quotes.

In HUME, the order is reversed: you **select first, then act**. `w` selects the next word, then `d` deletes the selection. This means:

- Motions always change the selection before anything else
- Operators (`d`, `c`, `y`, …) act on whatever is currently selected
- The selection is always visible — there is no invisible "cursor as a point"

## Mode map

| Vim mode | HUME equivalent |
|----------|----------------|
| Normal | Normal |
| Insert | Insert |
| Visual | Extend mode (`e`) or any motion that grows the selection |
| Visual Line | Extend mode + line motions |
| Visual Block | HUME has no rectangular selection. Use `C` to spawn column-aligned multi-cursors (one per line below), then edit — this approximates column editing without a true visual block. |
| Command-line | Command line (`:`) |

## Key differences

### Extend mode vs Visual mode

Vim's Visual mode is entered once and stays until you act. HUME's Extend mode is similar — press `e` to enter it, and every motion extends the selection until you act or press `Esc`.

### Registers

HUME replaces Vim's letter registers (`a`–`z`) with a small set of mnemonic single-character names and digit registers `"0`–`"9`. The default paste (`p`) is smart: it reads from the kill ring when the last operation was a `d` or `c`, and from the system clipboard otherwise. After `y` this still pastes the text you just yanked, because `y` writes the clipboard as well as the kill ring — but if you yanked to an explicit non-default register (`"0y`), `p` reads the clipboard (which was not touched), so use `"0p` to paste from the named register. Cycle kill-ring entries with `[` and `]`.

| Name | HUME function | Vim equivalent |
|------|---------------|----------------|
| `"0`–`"9` | Named storage — text or macros | `"0`–`"9` |
| `"k` | Kill-ring head (most recent yank/delete) | — |
| `"c` | System clipboard | `"+` |
| `"b` | Black hole — writes discarded | `"_` |
| `"s` | Search register — last search pattern | `"/` |
| `"q` | Macro register (`QQ`/`qq`) | — |

Note: letter registers `a`–`z` other than the special names above do not exist. All yanks and deletes go to the kill ring (`k`) and digit registers (`0`–`9`). Use `[`/`]` to cycle through kill-ring history.

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

Vim's `G` (last line) has no single-key equivalent; use `g e`. Otherwise the `0` / `$` / `^` keys keep their vim meaning (start / end / first-non-blank), though HUME's idiom is the `g` prefix: `g h`, `g l`, `g s`.

| Vim | HUME |
|-----|------|
| `0` / `$` / `^` | `0` / `$` / `^` (or `g h` / `g l` / `g s`) |
| `G` | `g e` |
| `gg` | `g g` |

## Commands you already know

Most `:` commands work as expected:

| Vim | HUME |
|-----|------|
| `:w` | `:w` |
| `:q` / `:q!` | `:q` / `:q!` |
| `:wq` | `:wq` |
| `:e file` | `:e file` |
| `:ls` / `:buffers` | `:ls` |
| `:bn` / `:bp` | `:bnext` / `:bprev` |
| `:bd` | `:bd` |
| `:cd` | `:cd` |
| `:pwd` | `:pwd` |
| `Ctrl+^` | `Ctrl+6` |
| `Ctrl+o` / `Ctrl+i` | `Ctrl+o` / `Ctrl+i` |
