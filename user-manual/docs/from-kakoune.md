# Coming From Kakoune

HUME's editing model comes from Kakoune, so more transfers here than from any other editor. Selections come first, operators act on them, and multiple selections are the normal way to work rather than a special mode. What differs is how you *extend* a selection, what the search keys do, and how much HUME does inside the editor instead of through the shell.

## The first ten minutes with HUME

### Opening a file

`hume file.txt` opens a file, and several names open several buffers. To start on a given line, append it to the name: `hume file.txt:42` (or `file.txt:42:5` for a column) instead of `kak +42 file.txt`. Each `hume` is a standalone editor: there is no session to attach to with `-c`. See [Splits, windows, and tabs](#splits-windows-and-tabs).

`:e path` works as in Kakoune, even on a path that doesn't exist yet: the first `:w` creates the file. Bare `:e` reloads the file from disk, and `:e!` reloads it, discarding your changes.

The fuzzy finder is a plugin. Load `core:pickers` (the [starter config](configuration.md#example-init-scm) already does) and `z f` finds a file, `z b` an open buffer. See [Fuzzy Finder](pickers.md).

### Editing

Type `:tutor` and press `Enter`. The tutor is your own copy, so edit it freely. Make the same edits you would in Kakoune on its opening paragraphs, finding each line with `/` as you would there:

| Edit | Kakoune | HUME |
|------|---------|------|
| Delete `Welcome. ` from `Welcome. This buffer is your practice space.` | `/Welcome` `Enter` `T` `T` `d` | `/Welcome` `Enter` `e` `t` `T` `d` |
| Change `practice` to `play` on the same line, then undo it with `u` | `/practice` `Enter` `c` `play` `Esc` | `/practice` `Enter` `c` `play` `Esc` |
| Copy that line to another application | `x` `<a-\|>` + your clipboard tool | `x` `y` |
| Delete the two lines of `When you are ready, … move down.` | `/When you` `Enter` `x` `x` `d` | `/When you` `Enter` `x` `Ctrl-x` `d` |

What each step shows:

- **`t` only moves; Extend mode makes it select.** `f`, `F`, `t` and `T` move the cursor and select nothing, where Kakoune selects up to the target. `e` toggles Extend mode, where every motion grows the selection. So `t T` grows `Welcome` to `Welcome. `, and `d` deletes it and leaves the mode. On a terminal with the [kitty keyboard protocol](installation.md#terminal-compatibility), holding `Ctrl` extends for one motion, the job Shift does in Kakoune: `Ctrl-t T` grows the selection without entering Extend mode.
- **A second `x` moves on.** Each `x` selects the next line on its own, dropping the previous one. `Ctrl-x` extends instead. See [Line selection](#line-selection).
- **`Esc` keeps what you typed selected.** After the change, `play` is still selected instead of reduced to a cursor, ready to act on again. One consequence: `i` re-enters *before* that selection, so use `a` to keep typing after it. Disable this with the `insert-select-on-exit` option (see [Buffer options](settings.md#buffer-options)).
- **`y` reaches the system clipboard.** It copies to the clipboard as well as HUME's own kill ring, with no wiring. `p` pastes your last yank or delete while you haven't edited since, and the clipboard once you have. See [Copy & Paste](copy-and-paste.md).

Shift does not extend a selection: `W`, `H`, `J`, `K`, `L` do something else here or nothing, so read [Extending selections](#extending-selections) before reaching for them.

### Saving and quitting

`:w` and `:w!` work as in Kakoune. Kakoune's `:q` leaves the client; HUME's closes the focused pane when the tab has several, and the tab when other tabs are open. On the last pane it closes the current buffer: if another file is still open you land on it, and HUME quits only when no other file or unsaved buffer is left. `:q!` and `:wq` close the same way, discarding your changes or saving them first. `:qa` leaves in one step. Kakoune's `:db` is `:bd`. See [Quitting](files-and-buffers.md#quitting).

The tutor you started is the hands-on tour of everything else: carry on from its Introduction. To start over with a clean copy, close it with `:bd!` and run `:tutor` again.

## Bringing your config over

`kakrc` is kakscript: `set-option` for options, `map` for keys, `hook` for events, `define-command` for new commands. HUME's [`init.scm`](configuration.md) is Scheme, with a function for each: `(set-option! "name" value)` for options, `(bind-key! 'normal "keys" "command")` for keys, `register-hook!` for events and `define-command!` for new commands. `init.scm` is a Scheme program, so it can also use conditionals, loops and functions, the work you would hand to `%sh{}` in kakrc, without leaving the editor.

| Setting | Kakoune `kakrc` | HUME `init.scm` |
|---|---|---|
| Theme | `colorscheme onedark` | `(set-option! "theme" "onedark")` |
| Line numbers | `add-highlighter global/numbers number-lines -relative` | `(set-option! "gutter-line-numbers" "relative")` for the distance from the cursor line (0 on it), or `"hybrid"`, the default, for the cursor line's own number |
| Tab width | `set-option global tabstop 4` | `(set-option! "indent-width" 4)` |
| Indent width | `set-option global indentwidth 4` | same as tab width: HUME has one `indent-width` option, not two |
| Spaces vs tabs | `set-option global indentwidth 0` (tabs) | `(set-option! "indent-style" "soft")` (spaces) or `"hard"` (tabs) |
| Scroll padding | `set-option global scrolloff 5,0` | `(set-option! "scroll-margin" 5)` (lines only; HUME has no horizontal scroll margin) |
| Extra word characters | `set-option global extra_word_chars '_'` | `(set-option! "word-chars" "_")` |
| Reload on external change | `set-option global autoreload yes` | *(none)*: HUME never reloads on its own. It asks first (`file-reload-prompt`, on by default) |
| Whitespace indicators | `add-highlighter global/whitespace show-whitespaces` | `(set-option! "whitespace-space" "all")`<br>`(set-option! "whitespace-tab" "all")`<br>`(set-option! "whitespace-newline" "all")` |
| Keybinding | `map global normal <c-j> ...` | `(bind-key! 'normal "ctrl-j" "move-down")` |

Kakoune's `extra_word_chars` is usually set per filetype from a `hook`. HUME's equivalent is `on-language-set`, setting the `word-chars` buffer option (see [Buffer options](settings.md#buffer-options)):

```scheme
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (member lang '("css" "scss" "less"))
      (set-buffer-option! pane "word-chars" "-"))))
```

`bind-key!` takes a command *name* (listed in [Editor Commands](editor-commands.md)), not a key sequence: there's no key-to-key remapping.

## What's the same

These work the way you expect, on the same keys:

- Select-then-act, selections always covering at least one character, multiple selections as a first-class tool
- `;` reduce to cursor, `,` keep only the main selection, `(` / `)` rotate the main selection
- `%` select the whole buffer, `s` narrow each selection to its regex matches
- `&` align selections, `_` trim surrounding whitespace
- `C` duplicate the selection onto the line below
- `>` / `<` indent / unindent the lines a selection touches. HUME rewrites each line's whole indent in the buffer's `indent-style`; see [Lines and alignment](editing.md#lines-and-alignment)
- `i` / `a` / `I` / `A` / `o` / `O` to enter Insert mode, `c` / `d` / `y` / `p` to change / delete / yank / paste
- `Q` to record a macro and `q` to play it
- `u` undo, `U` redo
- `g h` line start, `g l` line end, `g g` first line
- Registers hold one entry per selection, and pasting into as many selections gives each its own entry

## Key differences

### Less shell integration

Kakoune is built to hand text to other programs: `|` pipes selections through a command, `!` inserts a command's output, and `%sh{ … }` expansions let the config shell out for anything the editor doesn't do itself.

HUME keeps `|`, with the same newline rule as Kakoune's. Instead of `!`, `Ctrl-|` (kitty only) in Insert mode inserts a command's output (see [Shell commands](editing.md#shell-commands)). Anything you would reach for `%sh{}` to do is written in Scheme instead, running inside the editor with direct access to buffers, selections and commands.

### Extending selections

This is the one that will trip you up most often. In Kakoune, holding Shift extends: `w` replaces the selection, `W` extends it, and the same goes for `H`, `J`, `K`, `L`. There is no extend *mode*: `v` opens view mode.

HUME's uppercase keys are not extends. `W` and `B` are the WORD variants, the job Kakoune gives to `<a-w>` and `<a-b>`. Extending happens two other ways. Extend mode, toggled with `e`, extends with every motion until you act or press `Esc`. A one-shot `Ctrl`+motion extends for a single keystroke without changing mode.

| Kakoune | HUME |
|---------|------|
| `W` (extend by word) | `e` then `w`, or `Ctrl-w` |
| `<a-w>` (WORD motion) | `W` |
| `H` / `J` / `K` / `L` | Extend mode, or `Ctrl-h` / `Ctrl-j` / `Ctrl-k` / `Ctrl-l` |
| `<a-;>` (flip direction) | `Ctrl-e` |
| `<a-,>` (remove main selection) | `Ctrl-,` |
| *(no equivalent)* | `e`, a persistent Extend mode |

The `Ctrl` one-shots and `Ctrl-,` need the kitty keyboard protocol, except `Ctrl-x`, which extends on any terminal. Extend mode works on any terminal. See [Terminal compatibility](installation.md#terminal-compatibility).

Bare `K` (no modifier) is not an extend in HUME: it shows LSP hover docs for the symbol under the cursor (with `core:lsp` loaded).

### Line selection

Kakoune's `x` expands the selection to cover full lines and keeps growing it on every press, and `<a-x>` trims a selection back to whole-line boundaries.

HUME's `x` re-anchors instead: each press selects one line and moves on, rather than accumulating. Growing is the job of the extend keys.

| Press | Kakoune `x` | HUME `x` | HUME `Ctrl-x` |
|-------|-------------|----------|---------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

`X` and `Ctrl-Shift-x` are the backward forms: a line selection grown downward with `Ctrl-x` shrinks back up with `X` in Extend mode, or `Ctrl-Shift-x` (kitty only). There is no equivalent of `<a-x>`; `_` trims whitespace, not to line bounds.

### Splitting, copying and merging selections

`S` is bound in both editors and means different things. Kakoune splits on a regex you type; HUME splits on newlines, which is Kakoune's `<a-s>`.

| Kakoune | HUME |
|---------|------|
| `<a-s>` (split on line boundaries) | `S` |
| `S` (split on a regex) | *(none)*: `s` narrows to regex matches instead |
| `<a-C>` (copy selection onto the line above) | no default key: bind `copy-selection-on-prev-line` (see below) |
| `<a-_>` (merge contiguous selections) | automatic: selections that share a character merge; touching ones stay separate |

To get `<a-C>` back, bind the command in your `init.scm`:

```scheme
(bind-key! 'normal "alt-C" "copy-selection-on-prev-line")
```

### Search

The search keys overlap heavily in spelling and barely at all in meaning.

| Kakoune key | Does in Kakoune | HUME equivalent |
|-------------|-----------------|-----------------|
| `/` | Select the next match | `/`: search forward, selecting the match, with a live preview as you type |
| `?` | Extend to the next match | `Ctrl-n` (kitty only), or `n` in Extend mode |
| `n` | Move the main selection to the next match | `n` |
| `N` | Add a selection at the next match | *(none)*: `m /` selects every match at once |
| `*` | Set the search pattern from the selection, adding word boundaries | *(none)*: HUME's `*` searches the word under the cursor and selects it |
| `<a-*>` | Set the search pattern from the selection, verbatim | `Ctrl-/` (kitty only) |

::: warning
Two traps. `?` opens a backward search in HUME, not an extend like Kakoune's. And `N` has no equivalent: use `m /` to select every match at once instead, then narrow with `(`/`)` and `Ctrl-,` (`,` keeps only the primary). `Ctrl-,` needs kitty; on other terminals, bind `remove-primary-selection` to a key of your own, e.g. `(bind-key! 'normal "alt-," "remove-primary-selection")` for Kakoune's own `<a-,>`.
:::

### Text objects

Kakoune reaches objects with `<a-i>` and `<a-a>`; HUME puts them behind an `m` prefix:

| Kakoune | HUME |
|---------|------|
| `<a-i>w` / `<a-a>w` | `m i w` / `m a w` |
| `<a-i>(` / `<a-a>(` | `m i (` / `m a (` |
| `m` (select to the matching bracket) | `#` jumps to the matching bracket or tag; `m s` + delimiter selects the pair |
| `[` / `]` / `{` / `}` (to object start/end) | *(none)* |

`[`/`]`/`{`/`}` are all bound elsewhere in HUME: kill-ring cycling and paragraph motions.

The objects are word (`w`), WORD (`W`), the bracket and quote pairs, argument (`a`), line (`l`) and paragraph (`p`). HUME adds `m i i`, which selects the text you typed during your last insert, and `m w` + a delimiter, which wraps each selection in a pair. With a grammar that supports them, HUME also has function, class, comment, unit test and value objects, and a `g` jump to the next or previous of each kind. See [Text objects](selections.md#text-objects) and [Structural navigation](moving-around.md#structural-navigation).

### Registers, paste, and the clipboard

Kakoune names any register with `"` plus a character. HUME's set is small and fixed: `"0`–`"9` for your own storage (text or macros), `"k` for the kill-ring head, `"c` for the system clipboard and `"b` for the black hole. There are no other single-character registers, and none like Kakoune's `%`, `.` and `#`. See [Register prefix](copy-and-paste.md#register-prefix).

`[`/`]` cycle through older/newer kill-ring entries after a paste.

### Macros

Same keys (`Q` starts and stops recording, `q` replays), but the storage differs. Kakoune records into `@` by default and takes any other register through the `"` prefix. HUME records into `q` by default (`Q Q` to record, `q q` to play) and otherwise only accepts digits: `Q 3` records into register 3. Those are the same digit slots `"3y` writes to, so keep macros and yanked text on separate numbers. See [Macros](editing.md#macros).

### Repeat

Kakoune's `.` repeats the last Insert-mode change, and `<a-.>` repeats the last object or `f`/`t` selection. HUME's `.` repeats the last editing command or Insert session: deletes, changes and pastes included. Give it a count to override the original. Repeating a find is `=` forward and `-` backward.

### Splits, windows, and tabs

Kakoune leaves window management to tmux or your window manager: you run multiple clients against one session and arrange them outside the editor.

HUME has panes built in. `Ctrl-p` is the prefix: `Ctrl-p s` and `Ctrl-p v` split, `Ctrl-p h/j/k/l` move focus, `Ctrl-p c` closes. `:sp` and `:vsp` do the same from the `:` prompt. There is no client/server model, so no attaching a second client to a running session.

A tab holds a whole pane layout. `:tabnew` opens one, `:tabclose` drops it, and `Ctrl-p t` / `Ctrl-p T` cycle between them; buffers are shared across every tab. The nearest Kakoune analogue is a second tmux window with another client attached to the same session. See [Tabs](files-and-buffers.md#tabs).

### User modes

Kakoune's named user modes (`declare-user-mode`) have no equivalent. A key sequence under any prefix, including a `Space` leader, binds like any other: `(bind-key! 'normal "space d" "delete")`.

### Plugins and language servers

Kakoune has neither a package manager nor LSP support in the core; both come from outside, via plug.kak and kakoune-lsp.

Both ship with HUME. [PLUM](core-plugins.md#core-plum) is the plugin manager: declare a plugin in `init.scm`, run `:plum-install-plugins`, and it is fetched from GitHub. [Installing a plugin](plugins.md#installing-a-plugin) walks through it with [grep.hume](https://github.com/cvlmtg/grep.hume), a live-grep picker and HUME's first official third-party plugin.

Language server support is a core plugin rather than a separate process you configure by hand. See [Language Servers](lsp.md). Syntax highlighting is tree-sitter based and built in.

## What Kakoune has that HUME doesn't

- `!` and `%sh{}` expansions: Normal-mode command output, and shelling out from config
- The client/server model, and multiple clients on one session
- Marks (Kakoune's `^` register)
- Selection undo and redo (`<c-h>` / `<c-k>`)
- Adding a selection at the next match (`N`)
- Splitting selections on a regex (`S`)
- Rotating selection *contents* (`<a-)>`), as opposed to rotating which selection is primary
- Named user modes (`declare-user-mode`)
- Arbitrary single-character registers
- Trimming a selection to whole-line bounds (`<a-x>`)

## What HUME has that Kakoune doesn't

- A built-in system clipboard, a [kill ring, and a paste](copy-and-paste.md) that picks the right source
- [Extend mode](#extending-selections), plus one-shot extends that need no mode switch
- Built-in [panes and splits](files-and-buffers.md#splits-and-panes), and [tab pages](files-and-buffers.md#tabs)
- A [plugin manager](core-plugins.md#core-plum), [Scheme scripting](configuration.md), and [hooks](writing-plugins.md#hooks)
- [Language-server](lsp.md) and [tree-sitter](syntax-highlighting.md) support that ships with the editor
- [Dot-repeat](editing.md#repeat) covering every edit, not only Insert-mode changes
- A browsable graph of the undo history ([`core:undotree`](core-plugins.md#core-undotree)), and undo by time with [`:earlier 5m`](command-mode.md#editing)
- [Selecting every search match at once](moving-around.md#search-navigation) (`m /`)
- Grammar-backed [structural text objects](#text-objects) and [navigation](moving-around.md#structural-navigation) (functions, classes, comments, tests, values)
