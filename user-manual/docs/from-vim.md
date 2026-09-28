# Coming From Vim / Neovim

If you know Vim or Neovim, HUME will feel different in many ways. This page covers the key differences to help you reorient quickly.

## The first ten minutes with HUME

### Opening a file

`hume file.txt` opens a file, and several names open several buffers. To start on a given line, append it to the name: `hume file.txt:42` (or `file.txt:42:5` for a column) instead of `vim +42 file.txt`. With no file at all you get an empty `*scratch*` buffer.

Inside the editor, `:e path` works as in Vim, including on a path that doesn't exist yet: the file is created on the first `:w`. Bare `:e` reloads the file from disk, and `:e!` throws away your changes while doing it.

There is no file explorer, and the fuzzy finder is a plugin. Load `core:pickers` (the [starter config](configuration.md#example-init-scm) already does) and `z f` finds a file, `z b` an open buffer. See [Fuzzy Finder](pickers.md).

### Editing

`h` `j` `k` `l` move and `i`, `a`, `I`, `A`, `o`, `O` enter Insert mode as usual. The rest of editing turns Vim's grammar around. In Vim, most operators work on a motion you specify *after* the operator: `dw` deletes a word, `ci"` changes inside quotes. In HUME you **select first, then act**. `w` selects the next word, then `d` deletes the selection. This means:

- Motions always change the selection before anything else
- Operators (`d`, `c`, `y`, …) act on whatever is currently selected
- The selection is always visible: there is no invisible "cursor as a point"

<div class="key-demo">
<strong>Cursor on the first character, press <code>w</code></strong><br>
Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
<strong>Press <code>d</code></strong><br>
Lorem<span class="head">&nbsp;</span>dolor sit
</div>

You see the selection `w` built (word plus its leading whitespace) before `d` ever runs, instead of composing `dw` blind and finding out what it did after the fact.

#### Try it on your config

Copy the bundled starter config to `~/.config/hume/init.scm` ([where to find it](configuration.md#example-init-scm)) and open it on line 20 with `hume ~/.config/hume/init.scm:20`. Then make the same three edits you would in Vim:

| Edit | Vim | HUME |
|------|-----|------|
| Uncomment line 20 (`core:buffer-words`) | `dt(` | `Ctrl-t` `(` `d` |
| Delete lines 17 and 18 (`core:plum`, `core:git-diff`) | `:17` `2dd` | `:17` `2` `Ctrl-x` `d` |
| Turn `declare-plugin!` into `load-plugin!` on line 16 | `:16` `l` `ciw` `load` `Esc` | `:16` `w` `c` `load` `Esc` |

What each step shows:

- **`t` only moves; `Ctrl` makes it select.** `f`, `F`, `t` and `T` move the cursor and select nothing. Holding `Ctrl` turns a motion into a one-shot extend, so `Ctrl-t (` selects `;; ` and `d` deletes it. For longer selections, `e` toggles Extend mode, where every motion extends until you act or press `Esc`. The `Ctrl` forms need the [kitty keyboard protocol](installation.md#terminal-compatibility).
- **`x` selects a line.** It replaces Vim's `dd`/`yy`/`cc` doubling: `x d` deletes a line, `x c` rewrites its content and keeps the line. `Ctrl-x` adds the line below to the selection.
- **`Esc` keeps what you typed selected.** After the last edit `load` is still highlighted, rather than the cursor parking on its last character. Pressing `i` now inserts *before* it; use `a` to carry on after it. Turn this off with the `select-inserted-text` option (see [Configuration](configuration.md)).

A few more keys you'll want early: `d` on its own deletes the character under the cursor (Vim's `x`), since the cursor is already a one-character selection. `g h` and `g l` go to the start and end of the line, `g g` and `g e` to the first and last line. `/`, `n` and `N` search as in Vim.

### Saving and quitting

`:w`, `:w path`, `:wq`, `:q!`, `:qa` and `:qa!` do what you expect. There is no `:x` and no `ZZ`. On `*scratch*`, `:w` needs a path: `:w notes.txt`.

`:q` closes the current buffer, not the editor. If other files are still open, you land on one of them, and HUME quits only when nothing is left. `:qa` leaves in one step. With split panes or tabs, `:q` closes the focused pane or tab first; see [Quitting](files-and-buffers.md#quitting).

HUME writes no swap files, and undo history doesn't survive a restart (not yet). See [Persistence and safety](files-and-buffers.md#persistence-and-safety).

For a hands-on tour of everything else, type `:tutor` and press `Enter`: it opens an interactive tutorial you can edit freely.

## Bringing your config over

HUME has no `vimrc`. Every setting and keybinding is a call to a Scheme function in [`init.scm`](configuration.md): `(set-option! "name" value)` for options, `(bind-key! 'normal "keys" "command")` for keys. Being a real programming language, `init.scm` also has conditionals, loops, and abstraction, closer to what you would reach for `.vim`/Lua scripting to do.

| Setting | `vimrc` | HUME `init.scm` |
|---|---|---|
| Theme | `colorscheme onedark` | `(set-option! "theme" "onedark")` |
| Line numbers | `set number relativenumber` | `(set-option! "line-number-style" 'relative)` |
| Tab width | `set tabstop=4 shiftwidth=4` | `(set-option! "tab-width" 4)` |
| Spaces vs tabs | `set expandtab` | `(set-option! "tab-style" 'soft)` |
| Scroll padding | `set scrolloff=5` | `(set-option! "scroll-margin" 5)` |
| Mouse | `set mouse=a` | `(set-option! "mouse" #t)` |
| Line wrapping | `set wrap linebreak breakindent` | `(set-option! "wrap-mode" "indent")` |
| Whitespace indicators | `set list listchars=tab:>-,trail:-` | `(set-option! "whitespace-tab" 'all)`<br>`(set-option! "whitespace-space" 'trailing)` |
| Reload on external change | `set autoread` | `(set-option! "auto-read" #t)` |
| Word characters | `set iskeyword+=-` | `(set-option! "word-chars" "-")` |
| Keybinding | `nnoremap <C-j> ...` | `(bind-key! 'normal "ctrl-j" "move-down")` |

`iskeyword`'s range syntax (`48-57`, `@`, `192-255`) doesn't carry over; `word-chars` takes the literal characters instead. Vim's per-filetype `autocmd FileType python set …` becomes an `on-language-set` hook:

```scheme
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "python")
      (set-buffer-option! pane "tab-width" 4)
      (set-buffer-option! pane "tab-style" 'soft))))
```

`bind-key!` takes a command *name* (the same names listed in [Builtin Commands](builtin-commands.md)), not a key sequence — there's no key-to-key remapping.

## Mode map

| Vim mode | HUME equivalent |
|----------|----------------|
| Normal | Normal |
| Insert | Insert |
| Visual | Extend mode (`e`) or any motion that grows or shrinks the selection |
| Visual Line | Extend mode + line motions |
| Visual Block | HUME has no rectangular selection. Use `C` to spawn column-aligned multi-cursors (one per line below), then edit. This approximates column editing without a true visual block. |
| Command&nbsp;line | Command line (`:`) |

## Key differences

### Extend mode vs Visual mode

Vim's Visual mode is entered once and stays until you act. HUME's Extend mode is similar: press `e` to enter it, and every motion extends the selection until you act or press `Esc`. Motions run backward too: moving back toward where you started shrinks the selection, much like shrinking a Visual selection by moving back in Vim.

### Muscle-memory traps

These keys exist in both editors and do different things. They are the ones most likely to bite.

| Key | In Vim | In HUME | Vim's behaviour instead |
|-----|--------|---------|-------------------------|
| `s` | Substitute character | Filter each selection by a regex | `c` |
| `e` | Move to end of word | Toggle Extend mode | `m m` selects the word under the cursor |
| `%` | Jump to matching bracket | Select the whole buffer | `#` jumps to the matching bracket or tag; `m s` + delimiter selects the surrounding pair |
| `#` | Search word under cursor backward | Jump to the matching bracket or tag | `*` searches the word under the cursor forward; HUME has no backward variant |
| `;` | Repeat last `f`/`t` | Collapse the selection | `=` |
| `,` | Repeat last `f`/`t` backward | Keep only the primary selection | `-` |
| `m` | Set a mark | Text-object prefix | *(none)*: HUME has no marks; `Ctrl-o` / `Ctrl-i` walk the jump list |
| `[`&nbsp;/&nbsp;`]` | Bracket-motion prefix | Cycle the kill ring after a paste | — |
| `S` | Change whole line | Split selections on newlines | `x` then `c` |
| `C` | Change to end of line | Copy the selection to the line below | `ctrl-g l c`, or `C` with `core:vim-keybind` |
| `D` | Delete to end of line | *(unbound)* | `ctrl-g l d`, or `D` with `core:vim-keybind` |
| `K` | Look up keyword (external `keywordprg`, e.g. `man`) | Show hover docs for the symbol under the cursor (with `core:lsp`) | *(none)*: this is the closest equivalent; no external program |
| `gt`&nbsp;/&nbsp;`gT` | Next / previous tab | Next / previous class or type | `Ctrl-p t` / `Ctrl-p T` |

`f`, `F`, `t`, `T` behave as they do in Vim, and `{` / `}` are still paragraph motions. Only the *repeat* keys moved: use `=` and `-`, because `;` and `,` are taken.

`>` and `<` indent/unindent directly, select-then-act style: no operator-pending step and no doubled key, so `>` alone does what Vim's `>>` does. A count still works (`3>`), and unlike a bare `>>`/`<<`, HUME re-renders each touched line's whole indent to the buffer's `tab-width`/`tab-style` rather than only prepending or trimming a fixed amount.

### Text objects

Vim's `iw` / `aw` family lives behind the `m` prefix, and, as everywhere else, you select first and act second. `diw` becomes `m i w` then `d`.

| Vim | HUME |
|-----|------|
| `diw` | `m i w` then `d` |
| `ciw` | `m i w` then `c` |
| `ci"` | `m i "` then `c` |
| `da(` | `m a (` then `d` |
| `dap` | `m a p` then `d` |
| `dat` | *(none)*: no tag or sentence objects |

The available objects are word (`w`), WORD (`W`), the bracket pairs (`(`, `[`, `{`, `<`), the quote pairs (`"`, `'`, `` ` ``), argument (`a`), line (`l`), and paragraph (`p`), each with an `i` (inner) and `a` (around) form. One extra has no Vim equivalent: `m i i` selects the text you typed during your last insert.

For a language with a tree-sitter grammar that ships a textobjects query, HUME also adds `m i f`/`m a f` (function), `m i t`/`m a t` (class/type), `m i c`/`m a c` (comment), `m i u`/`m a u` (unit test), and `m i v`/`m a v` (array/tuple/struct value). Vanilla Vim has nothing like these without a plugin. Each pairs with a `goto-next-<kind>`/`goto-prev-<kind>` command that jumps to the next/previous one as a selection, on the same letter under `g` (lowercase forward, uppercase backward, e.g. `g f`/`g F`).

Vim's `iskeyword` is `word-chars`, a buffer option listing extra characters that count as part of a word (see [Configuration](configuration.md)). HUME doesn't parse Vim's range syntax (`48-57`, `@`, `192-255`); list the characters directly, e.g. `-` for CSS.

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
`s` needs something wider than a cursor to filter. On a bare one-character selection it does nothing at all. Press `%` or select a region first.
:::

Vim's confirm-each-match flag (`:%s/…/gc`) has no equivalent, but you see every match selected before you commit to changing it.

### Registers

HUME replaces Vim's letter registers (`a`–`z`) with a small set of mnemonic single-character names and digit registers `"0`–`"9`. The default paste (`p`) is smart: it reads from the kill ring while nothing has been edited since your last `d`/`c`/`y`, and from the system clipboard once something has. After `y` this still pastes the text you just yanked, because `y` writes the clipboard as well as the kill ring. Yanking to an explicit non-default register instead (`"0y`) leaves both untouched (`p` behaves exactly as it would have without the `"0y`), so use `"0p` to paste from the named register.

| Name | HUME function | Vim equivalent |
|------|---------------|----------------|
| `"0`–`"9` | Numbered storage: text *or* macros, last write wins | `"0`–`"9` |
| `"k` | Kill-ring head (most recent yank/delete) | — |
| `"c` | System clipboard | `"+` |
| `"b` | Black hole: writes discarded | `"_` |

Two more registers exist but can't be typed after `"`: the search register (the last pattern, vim's `"/`), written by `/`, `?` and `*`; and the macro register `q`, written by `Q` recording. You reach both through the commands that use them, not through the `"` prefix.

`[` and `]` only do something immediately after a paste: they swap the pasted text for an older or newer kill-ring entry. Pressed at any other time they do nothing.

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
| `@@` | *(none)* (use `q<reg>` again or `qq` for the default register) |

In Vim, `q` starts and stops recording, then `@` plays. In HUME, recording is started and stopped with `Q`; playback uses `q`. Valid macro registers are `q` and `0`–`9` (the default register is `q`: record with `QQ`, play with `qq`). There is no equivalent to Vim's `@@` (repeat last played register).

### Dot-repeat

Vim's `.` repeats the last change. HUME's `.` works the same way: it repeats the last editing command or insert session.

### Count prefix

Vim uses `[count]` before commands (e.g. `3dw`). HUME also supports count prefixes (`1`–`9`):

| Vim | HUME |
|-----|------|
| `3w` | `3w` |
| `5j` | `5j` |
| `d3w` | `3w` then `d` (select first, then act) |

### Line motion

Beyond the `g` keys above, `g s` goes to the first non-blank character (Vim's `^`). The vim keys `0` / `$` / `^` are not bound by default. Load `(load-plugin! "core:stdlib")` then `(load-plugin! "core:vim-keybind")` in your [`init.scm`](configuration.md) to get them back with their vim meaning, alongside `C` / `D` (change / delete to end of line) and `Ctrl-6` (see below). `G` is the case and rename prefix (`G L` / `G U` / `G C`, plus `G R` with `core:lsp`), and `core:vim-keybind` does not restore Vim's meaning; `g e` reaches the last line either way.

| Vim | HUME (native) | HUME (`core:vim-keybind`) |
|-----|----------------|---------------------------|
| `0` / `$` / `^` | `g h` / `g l` / `g s` | `0` / `$` / `^` |
| `C` / `D` | `ctrl-g l c` / `ctrl-g l d` (kitty terminals only) | `C` (change to end of line on a bare cursor with no count; with a selection, or any count prefix, falls back to the default `copy-selection-on-next-line`) / `D` |
| `o` (visual mode) | `Ctrl-e` (flips anchor and head, any mode) | `o` (in Extend mode) |

### Tabs

Tab pages carry over as-is: `:tabnew`, `:tabclose`, `:tabnext`, `:tabprev` (and Vim's own abbreviations `:tabe`, `:tabc`, `:tabn`, `:tabp`) spell and behave the same, including Vim's placement rules: `:tabnew` opens right after the current tab, `:tabclose` lands on the neighbor to the right (or the left, if there is none). The tab bar appears once a second tab is open; `tabline` is HUME's `showtabline`, with `dynamic` / `always` / `never` standing in for `1` / `2` / `0`.

The keys moved, though: `gt` / `gT` are not bound: `g` is HUME's goto prefix, and `g t` / `g T` already jump between classes/types (see the muscle-memory table above). Cycle tabs with `Ctrl-p t` / `Ctrl-p T` instead, under the same `Ctrl-p` prefix that stands in for Vim's `Ctrl-w`. The bindable spellings are `goto-next-tab` / `goto-prev-tab`.

Not present at the moment: `:tabonly`, `:tabmove`, `:tabfirst` / `:tablast`, `:tabs`, `{count}gt` to jump straight to tab *N*, and the `:tab` command modifier (`:tab split`).

## Commands you already know

Most `:` commands work as expected:

| Vim | HUME |
|-----|------|
| `:ls` / `:buffers` | `:ls` (`:buffers` is not an alias) |
| `:bn` / `:bp` | `:bn` / `:bp` (or `:bnext` / `:bprev`) |
| `:bd` | `:bd` |
| `:cd` | `:cd` |
| `:pwd` | `:pwd` |
| `:42` | `:42` |
| `:sp` / `:vsp` | `:sp` / `:vsp` |
| `:tabnew` / `:tabclose` | `:tabnew` / `:tabclose` (`:tabe` / `:tabc`) |
| `:tabnext` / `:tabprevious` | `:tabnext` / `:tabprev` (`:tabn` / `:tabp`) |
| `Ctrl-w` window prefix | `Ctrl-p` pane prefix |
| `Ctrl-^` | `:b #`, or `Ctrl-6` with `core:vim-keybind` loaded (kitty only) |
| `Ctrl-o` / `Ctrl-i` | `Ctrl-o` / `Ctrl-i` |
| `:set` | `:set`, with different syntax: `:set global\|buffer\|pane key=value` |
| `:help` | *(none)*: `:tutor` opens the tutorial, `:messages` shows the message log |
