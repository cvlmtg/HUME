# Coming From Vim / Neovim

HUME differs from Vim and Neovim in many ways. This page covers the key differences to help you reorient quickly.

## The first ten minutes with HUME

### Opening a file

`hume file.txt` opens a file, and several names open several buffers. To start on a given line, append it to the name: `hume file.txt:42` (or `file.txt:42:5` for a column) instead of `vim +42 file.txt`. With no file, you get an empty `*scratch*` buffer.

Inside the editor, `:e path` works as in Vim, even on a path that doesn't exist yet: the first `:w` creates the file. Bare `:e` reloads the file from disk, and `:e!` reloads it, discarding your changes.

There is no file explorer, and the fuzzy finder is a plugin. Load `core:pickers` (the [starter config](configuration.md#example-init-scm) already does) and `z f` finds a file, `z b` an open buffer. See [Fuzzy Finder](pickers.md).

### Editing

`h` `j` `k` `l` move and `i`, `a`, `I`, `A`, `o`, `O` enter Insert mode as usual. The rest of editing turns Vim's grammar around. In Vim, most operators act on a motion typed *after* them: `dw` deletes a word, `ci"` changes inside quotes. In HUME you select first, then act: `w` selects the next word, then `d` deletes the selection. This means:

- Motions always change the selection before anything else
- Operators (`d`, `c`, `y`, …) act on whatever is currently selected
- The selection is always visible: there is no invisible "cursor as a point"

<div class="key-demo">
<strong>Cursor on the first character, press <code>w</code></strong><br>
Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
<strong>Press <code>d</code></strong><br>
Lorem<span class="head">&nbsp;</span>dolor sit
</div>

You see what `w` selected (the word plus its leading whitespace) before `d` runs, instead of typing `dw` blind and finding out what it did afterwards.

Type `:tutor` and press `Enter`. The tutor is your own copy, so edit it freely. Make the same edits you would in Vim on its opening paragraphs, finding each line with `/`:

| Edit | Vim | HUME |
|------|-----|------|
| Delete `Welcome. ` from `Welcome. This buffer is your practice space.` | `/Welcome` `Enter` `dtT` | `/Welcome` `Enter` `e` `t` `T` `d` |
| Change `practice` to `play` on the same line, then undo it with `u` | `/practice` `Enter` `ciw` `play` `Esc` | `/practice` `Enter` `c` `play` `Esc` |
| Copy that line to another application | `"+yy` | `x` `y` |
| Delete the two lines of `When you are ready, … move down.` | `/When you` `Enter` `2dd` | `/When you` `Enter` `x` `Ctrl-x` `d` |

What each step shows:

- **`/` selects what it finds.** The whole match becomes the selection: after `/Welcome` it already covers `Welcome`, and after `/practice` a plain `c` changes the word with no `iw`.
- **`t` only moves; Extend mode makes it select.** `f`, `F`, `t` and `T` move the cursor and select nothing. `e` toggles Extend mode, Vim's Visual mode, where every motion extends until you act or press `Esc`. So `t T` grows `Welcome` to `Welcome. `, and `d` deletes it and leaves the mode. On a terminal with the [kitty keyboard protocol](installation.md#terminal-compatibility), holding `Ctrl` does the same for one motion: `Ctrl-t T` grows the selection without entering Extend mode.
- **`x` selects a line.** It replaces Vim's `dd`/`yy`/`cc` doubling: `x d` deletes a line, `x c` rewrites its content and keeps the line. `Ctrl-x` adds the line below to the selection.
- **`Esc` keeps what you typed selected.** After the change, `play` is still highlighted, rather than the cursor parking on its last character. Pressing `i` now inserts *before* it; use `a` to carry on after it. Turn this off with the `insert-select-on-exit` option (see [Buffer options](settings.md#buffer-options)).
- **`y` reaches the system clipboard.** It copies to the clipboard as well as HUME's own kill ring, with no `"+` prefix. `p` pastes your last yank or delete while you haven't edited since, and the clipboard once you have. See [Copy & Paste](copy-and-paste.md).

A few more keys you'll want early. `d` on its own deletes the character under the cursor (Vim's `x`), since the cursor is already a one-character selection. `g h` and `g l` go to the start and end of the line, `g g` and `g e` to the first and last line. `/`, `n` and `N` search as in Vim, selecting each match.

### Saving and quitting

`:w`, `:w path`, `:qa` and `:qa!` do what you expect. There is no `:x` and no `ZZ`. On `*scratch*`, `:w` needs a path: `:w notes.txt`.

`:q` closes the focused pane when the tab has several, and the tab when other tabs are open. On the last pane it closes the current buffer, not the editor: if another file is still open you land on it, and HUME quits only when no other file or unsaved buffer is left. `:q!` and `:wq` close the same way, discarding your changes or saving them first. `:qa` leaves in one step. See [Quitting](files-and-buffers.md#quitting).

HUME writes no swap files, and undo history doesn't survive a restart yet. See [Persistence and safety](files-and-buffers.md#persistence-and-safety).

The tutor you started is the hands-on tour of everything else: carry on from its Introduction. To start over with a clean copy, close it with `:bd!` and run `:tutor` again.

## Bringing your config over

HUME has no `vimrc`. Every setting and keybinding is a call to a Scheme function in [`init.scm`](configuration.md): `(set-option! "name" value)` for options, `(bind-key! 'normal "keys" "command")` for keys. `init.scm` is a Scheme program, so it can also use conditionals, loops and functions, closer to what you would use Vimscript or Lua for.

| Setting | Vim `vimrc` | HUME `init.scm` |
|---|---|---|
| Theme | `colorscheme onedark` | `(set-option! "theme" "onedark")` |
| Line numbers | `set number relativenumber` | `(set-option! "gutter-line-numbers" "hybrid")`, the default. `"relative"` shows 0 on the cursor line, like `relativenumber` without `number` |
| Tab width | `set tabstop=4 shiftwidth=4` | `(set-option! "indent-width" 4)` |
| Spaces vs tabs | `set expandtab` | `(set-option! "indent-style" "soft")` |
| Scroll padding | `set scrolloff=5` | `(set-option! "scroll-margin" 5)` |
| Mouse | `set mouse=a` | `(set-option! "mouse" #t)` |
| Line wrapping | `set wrap linebreak breakindent` | `(set-option! "wrap-mode" "indent")`, the default |
| Whitespace indicators | `set list listchars=tab:>-,trail:-` | `(set-option! "whitespace-tab" "all")`<br>`(set-option! "whitespace-space" "trailing")` |
| Reload on external change | `set autoread` | *(none)*: HUME never reloads on its own. It asks first (`file-reload-prompt`, on by default) |
| Word characters | `set iskeyword+=-` | `(set-option! "word-chars" "-")` |
| Keybinding | `nnoremap <C-j> ...` | `(bind-key! 'normal "ctrl-j" "move-down")` |

`word-chars` is a buffer option listing extra characters that count as part of a word (see [Buffer options](settings.md#buffer-options)). `iskeyword`'s range syntax (`48-57`, `@`, `192-255`) doesn't carry over: list the characters directly.

Vim's per-filetype `autocmd FileType python set …` becomes an `on-language-set` hook:

```scheme
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (equal? lang "python")
      (set-buffer-option! pane "indent-width" 4)
      (set-buffer-option! pane "indent-style" "soft"))))
```

`bind-key!` takes a command *name* (listed in [Editor Commands](editor-commands.md)), not a key sequence: there's no key-to-key remapping.

## What's the same

- `h` `j` `k` `l`, `i` `a` `I` `A` `o` `O`, `u` undo, `.` repeat
- `f`, `F`, `t`, `T` find a character; `{` / `}` move by paragraph
- `/`, `n`, `N` search
- `Ctrl-o` / `Ctrl-i` walk the jump list
- Tab pages, with Vim's own tab commands (see [Tabs](#tabs))

Most `:` commands carry over:

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
| `Ctrl-w r` / `Ctrl-w R` | `Ctrl-p r` / `Ctrl-p R`. Panes split the other way inside the row or column move together instead of being refused |
| `Ctrl-^` | `:b #`, or `Ctrl-6` with `core:vim-keybind` loaded (kitty only) |
| `:set` | `:set`, with different syntax: `:set global\|buffer\|pane key=value` |

## Key differences

### Mode map

| Vim mode | HUME equivalent |
|----------|----------------|
| Normal | Normal |
| Insert | Insert |
| Visual | Extend mode (`e`) or any motion that grows or shrinks the selection |
| Visual Line | Extend mode + line motions |
| Visual Block | HUME has no rectangular selection. Use `C` to spawn column-aligned cursors (one per line below), then edit |
| Command&nbsp;line | Command mode (`:`) |

### Extend mode vs Visual mode

Vim's Visual mode is entered once and stays until you act. HUME's Extend mode is similar: press `e` to enter it, and every motion extends the selection until you act or press `Esc`. Moving back toward where you started shrinks the selection, as moving back does in Vim's Visual mode.

### Muscle-memory traps

These keys exist in both editors but do different things, so they are the most likely to bite.

| Key | In Vim | In HUME | For Vim's behaviour, use |
|-----|--------|---------|--------------------------|
| `s` | Substitute character | Filter each selection by a regex | `c` |
| `e` | Move to end of word | Toggle Extend mode | `w`, which selects the next word through its end |
| `%` | Jump to matching bracket | Select the whole buffer | `#` jumps to the matching bracket or tag; `m s` + delimiter selects the surrounding pair |
| `#` | Search word under cursor backward | Jump to the matching bracket or tag | `*` searches the word under the cursor forward; HUME has no backward variant |
| `;` | Repeat last `f`/`t` | Collapse the selection | `=` |
| `,` | Repeat last `f`/`t` backward | Keep only the primary selection | `-` |
| `m` | Set a mark | Text-object prefix | *(none)*: HUME has no marks; `Ctrl-o` / `Ctrl-i` walk the jump list |
| `[`&nbsp;/&nbsp;`]` | Bracket-motion prefix | Cycle the kill ring after a paste | *(none)* |
| `S` | Change whole line | Split selections on newlines | `x` then `c` |
| `C` | Change to end of line | Copy the selection to the line below | `e g l c`, or `C` with `core:vim-keybind` |
| `D` | Delete to end of line | *(unbound)* | `e g l d`, or `D` with `core:vim-keybind` |
| `K` | Look up keyword (external `keywordprg`, e.g. `man`) | Show hover docs for the symbol under the cursor (with `core:lsp`) | *(none)*: this is the closest equivalent; no external program |
| `gt`&nbsp;/&nbsp;`gT` | Next / previous tab | Next / previous class or type | `Ctrl-p t` / `Ctrl-p T` |

`f`, `F`, `t`, `T` behave as in Vim, and `{` / `}` are paragraph motions in both. Only the *repeat* keys moved: `;` and `,` are taken, so use `=` and `-`.

`>` and `<` indent and unindent directly, select-then-act style, with no operator-pending step and no doubled key: `>` alone does what Vim's `>>` does. A count indents by that many levels (`3>`), not that many lines: select the lines first to indent several. See [Lines and alignment](editing.md#lines-and-alignment) for how the new indent is written.

### Text objects

Vim's `iw` / `aw` family lives behind the `m` prefix, and, as everywhere else, you select first and act second:

| Vim | HUME |
|-----|------|
| `diw` | `m i w` then `d` |
| `ciw` | `m i w` then `c` |
| `ci"` | `m i "` then `c` |
| `da(` | `m a (` then `d` |
| `dap` | `m a p` then `d` |
| `dat` | `m a x` then `d` |

The available objects are word (`w`), WORD (`W`), the bracket pairs (`(`, `[`, `{`, `<`), the quote pairs (`"`, `'`, `` ` ``), argument (`a`), tag pair (`x`), line (`l`), and paragraph (`p`), each with an `i` (inner) and `a` (around) form. HUME has no sentence object. `m i i` selects the text you typed during your last insert. With a grammar that supports them, HUME also has function, class, comment, unit test and value objects, with no plugin to install, and a `g` jump to the next or previous of each kind. See [Text objects](selections.md#text-objects) and [Structural navigation](moving-around.md#structural-navigation).

### Search and replace

There is no `:s`. Substitution is a selection built up and then changed:

```
%          select the whole buffer
s          filter the selection by a regex
old        type the pattern: one selection per match appears as you type
<Enter>    keep those selections
c          change them all at once
new<Esc>   type the replacement
```

That replaces Vim's `:%s/old/new/g`. To scope it to a region, select the region first and skip the `%`. To replace only some matches, drop the unwanted ones before pressing `c`: `(` / `)` makes a selection primary, `Ctrl-,` removes the primary, and `,` keeps only the primary. `Ctrl-,` needs kitty; on other terminals, bind `remove-primary-selection` to a key of your own, e.g. `(bind-key! 'normal "alt-," "remove-primary-selection")`.

`m /` is the other route: search with `/pattern` first, then `m /` turns every match in the buffer into a selection.

::: warning
`s` needs something wider than a cursor to filter: on a bare one-character selection it does nothing. Press `%` or select a region first.
:::

Vim's confirm-each-match flag (`:%s/…/gc`) has no equivalent, but you see every match selected before you change it.

### Registers

HUME has no letter registers `a`–`z`. `"0`–`"9` hold text or macros, written only when you name them (`"0y`); `"c` is the system clipboard (Vim's `"+`), `"b` the black hole (Vim's `"_`), and `"k` the kill-ring head. Yanks and deletes go to the kill ring. See [Register prefix](copy-and-paste.md#register-prefix).

The default paste (`p`) reads from the kill ring until you edit something after your last `d`/`c`/`y`, then from the system clipboard. After `y` it still pastes the text you yanked, because `y` writes the clipboard as well as the kill ring. Yanking to a named register (`"0y`) leaves both untouched, so `p` behaves as if the `"0y` never happened; use `"0p` to paste from that register.

Two more registers exist but can't be typed after `"`: the search register (the last pattern, Vim's `"/`), written by `/`, `?`, `*` and `Ctrl-/`; and the macro register `q`, written by `Q` recording. You reach both through the commands that use them.

`[` and `]` work only right after a paste: they swap the pasted text for an older or newer kill-ring entry.

### Macros

Vim records with `q` and plays with `@`; HUME starts and stops recording with `Q` and plays with `q`:

| Vim | HUME |
|-----|------|
| `qa` … `q` | `Q` + register … `Q` |
| `@a` | `q` + register |
| `@@` | *(none)*: use `q` + register again |

Valid macro registers are `q` and `0`–`9`. The default is `q`: record with `Q Q`, play with `q q`. See [Macros](editing.md#macros).

### Dot-repeat

As in Vim, `.` repeats the last change: the last editing command or Insert session.

### Count prefix

Vim puts a count before a command (`3dw`). HUME also takes a count prefix (`1`–`9`) before the command:

| Vim | HUME |
|-----|------|
| `3w` | `3w` |
| `5j` | `5j` |
| `d3w` | `3w` then `d` (select first, then act) |

### Line motion

Beyond the `g` keys above, `g s` goes to the first non-blank character (Vim's `^`). Vim's `0` / `$` / `^` are not bound by default. To get them back with their Vim meaning, along with `C` / `D` (change / delete to end of line) and `Ctrl-6`, load `(load-plugin! "core:stdlib")` then `(load-plugin! "core:vim-keybind")` in your [`init.scm`](configuration.md). `G` is the edit prefix (`G L` / `G U` / `G C`, `G G` to toggle comments, plus `G R` with `core:lsp`), and `core:vim-keybind` does not restore Vim's meaning for it. `g e` reaches the last line either way.

| Vim | HUME (native) | HUME (`core:vim-keybind`) |
|-----|----------------|---------------------------|
| `0` / `$` / `^` | `g h` / `g l` / `g s` | `0` / `$` / `^` |
| `C` / `D` | `e g l c` / `e g l d`, or `Ctrl-g l c` / `Ctrl-g l d` (kitty only) | `C` / `D` (see [`core:vim-keybind`](core-plugins.md#core-vim-keybind)) |
| `o` (Visual mode) | `Ctrl-e` (flips anchor and head, in Normal and Extend mode) | `o` (in Extend mode) |

### Tabs

Tab pages carry over. `:tabnew`, `:tabclose`, `:tabnext`, `:tabprev` (and Vim's own abbreviations `:tabe`, `:tabc`, `:tabn`, `:tabp`) are spelled and behave the same, including Vim's placement rules: `:tabnew` opens right after the current tab, and `:tabclose` lands on the neighbor to the right (or the left, if there is none). `tabline` is HUME's `showtabline`, with `dynamic` / `always` / `never` standing in for `1` / `2` / `0`. See [Tabs](files-and-buffers.md#tabs).

The keys moved, though. `gt` / `gT` are not bound: `g` is HUME's goto prefix, and `g t` / `g T` jump between classes/types (see the muscle-memory table above). Cycle tabs with `Ctrl-p t` / `Ctrl-p T`, under the `Ctrl-p` prefix that stands in for Vim's `Ctrl-w`. The commands to bind are `goto-next-tab` / `goto-prev-tab`.

## What Vim has that HUME doesn't

- `:s` substitution and its confirm flag (`:%s/…/gc`); see [Search and replace](#search-and-replace) for the selection-based way
- Marks
- A sentence text object
- Rectangular Visual Block selection
- Letter registers `a`–`z`, and `@@` to repeat the last macro
- Searching the word under the cursor backward (`#`)
- `:x` and `ZZ`
- `:help`: `:tutor` opens the tutorial, and `:messages` shows the message log
- A file explorer
- Swap files and persistent undo
- `:tabonly`, `:tabmove`, `:tabfirst` / `:tablast`, `:tabs`, `{count}gt` to jump straight to tab *N*, and the `:tab` command modifier (`:tab split`)

## What HUME has that Vim doesn't

- Multiple selections as the normal way to edit: `C` adds a cursor on the next line, `S` splits a selection into lines, `s` narrows it to regex matches, and `m /` selects every search match
- A selection you see before every edit
- A [kill ring and smart paste](copy-and-paste.md), with `y` reaching the system clipboard and deletes leaving it alone
- One-shot extends: `Ctrl` + a motion extends for one keystroke (kitty only, except `Ctrl-x`)
- Grammar-backed [structural text objects](selections.md#text-objects) and [navigation](moving-around.md#structural-navigation) (functions, classes, comments, tests, values) with no plugin to install
- A browsable graph of the undo history ([`core:undotree`](core-plugins.md#core-undotree))
