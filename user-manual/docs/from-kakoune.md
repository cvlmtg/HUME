# Coming From Kakoune

Kakoune invented the editing model HUME is built on, so more transfers here than from any other editor. Selections come first, operators act on them, and multiple selections are the normal way to work rather than a special mode. What differs is how you *extend* a selection, what the search keys do, and everything downstream of Kakoune's shell-first philosophy.

## The first ten minutes with HUME

### Opening a file

`hume file.txt` opens a file, and several names open several buffers. To start on a given line, append it to the name: `hume file.txt:42` (or `file.txt:42:5` for a column) instead of `kak +42 file.txt`. Each `hume` is a standalone editor: there is no session to attach to with `-c`. See [Splits, windows, and tabs](#splits-windows-and-tabs).

`:e path` works as in Kakoune, including on a path that doesn't exist yet: the file is created on the first `:w`. Bare `:e` reloads the file from disk, and `:e!` throws away your changes while doing it.

A fuzzy finder ships as a plugin. Load `core:pickers` (the [starter config](configuration.md#example-init-scm) already does) and `z f` finds a file, `z b` an open buffer. See [Fuzzy Finder](pickers.md).

### Editing

Type `:tutor` and press `Enter`. The tutor is your own copy, so edit it freely. Then make the same edits you would in Kakoune on its opening paragraphs, finding each line with `/` as you would there:

| Edit | Kakoune | HUME |
|------|---------|------|
| Delete `Welcome. ` from `Welcome. This buffer is your practice space.` | `/Welcome` `Enter` `T` `T` `d` | `/Welcome` `Enter` `e` `t` `T` `d` |
| Change `practice` to `play` on the same line, then undo it with `u` | `/practice` `Enter` `c` `play` `Esc` | `/practice` `Enter` `c` `play` `Esc` |
| Copy that line to another application | `x` `<a-\|>` + your clipboard tool | `x` `y` |
| Delete the two lines of `When you are ready, … move down.` | `/When you` `Enter` `x` `x` `d` | `/When you` `Enter` `x` `Ctrl-x` `d` |

What each step shows:

- **`t` only moves; Extend mode makes it select.** `f`, `F`, `t` and `T` move the cursor and select nothing, where Kakoune selects up to the target. `e` toggles Extend mode, where every motion grows the selection, so `t T` grows `Welcome` to `Welcome. ` and `d` deletes it and leaves the mode. On a terminal with the [kitty keyboard protocol](installation.md#terminal-compatibility), holding `Ctrl` extends for one motion, the job Shift does in Kakoune: `Ctrl-t T` grows the selection without entering Extend mode.
- **A second `x` moves on.** Each `x` selects the next line on its own, dropping the previous one. `Ctrl-x` extends instead. See [Line selection](#line-selection).
- **`Esc` keeps what you typed selected.** After the change, `play` is still selected instead of reduced to a cursor, ready to act on again. One consequence: `i` re-enters *before* that selection, so use `a` to keep typing after it. Disable this with the `insert-select-on-exit` option (see [Buffer options](settings.md#buffer-options)).
- **The system clipboard needs no wiring.** `y` copies to it as well as to HUME's own kill ring. `p` pastes your last yank or delete while you haven't edited since, and the clipboard once you have.

Shift does not extend a selection: `W`, `H`, `J`, `K`, `L` do something else here, so read [Extending selections](#extending-selections) before reaching for them.

### Saving and quitting

`:w`, `:wq`, `:q!` and `:w!` work as in Kakoune. Kakoune's `:q` leaves the client; HUME's closes the current buffer. If other files are still open, you land on one of them, and HUME quits only when nothing is left. `:qa` leaves in one step. Kakoune's `:db` is `:bd`. See [Quitting](files-and-buffers.md#quitting).

The tutor you just started is the hands-on tour of everything else: carry on from its Introduction. To start over with a clean copy, close it with `:bd!` and run `:tutor` again.

## Bringing your config over

`kakrc` is kakscript: `set-option` for options, `map` for keys, `hook` for events, `define-command` for new commands. HUME's [`init.scm`](configuration.md) is Scheme, with the same four ideas under different names, plus conditionals, loops, and abstraction from day one, closer to what you would reach `%sh{}` for in kakrc, without leaving the editor: `(set-option! "name" value)` for options, `(bind-key! 'normal "keys" "command")` for keys.

| Setting | Kakoune `kakrc` | HUME `init.scm` |
|---|---|---|
| Theme | `colorscheme onedark` | `(set-option! "theme" "onedark")` |
| Line numbers | `add-highlighter global/numbers number-lines -relative` | `(set-option! "gutter-line-numbers" 'relative)` |
| Tab width | `set-option global tabstop 4` | `(set-option! "indent-width" 4)` |
| Indent width | `set-option global indentwidth 4` | same as tab width: HUME has one `indent-width` option, not two |
| Spaces vs tabs | `set-option global indentwidth 0` (tabs) | `(set-option! "indent-style" 'soft)` (spaces) or `'hard` (tabs) |
| Scroll padding | `set-option global scrolloff 5,0` | `(set-option! "scroll-margin" 5)` (lines only; HUME has no horizontal scroll margin) |
| Extra word characters | `set-option global extra_word_chars '_'` | `(set-option! "word-chars" "_")` |
| Reload on external change | `set-option global autoreload yes` | `(set-option! "file-reload-prompt" #t)` |
| Whitespace indicators | `add-highlighter global/whitespace show-whitespaces` | `(set-option! "whitespace-space" 'all)`<br>`(set-option! "whitespace-tab" 'all)`<br>`(set-option! "whitespace-newline" 'all)` |
| Keybinding | `map global normal <c-j> ...` | `(bind-key! 'normal "ctrl-j" "move-down")` |

Kakoune's `extra_word_chars` is usually set per filetype from a `hook`. HUME's equivalent is `on-language-set`:

```scheme
(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (member lang '("css" "scss" "less"))
      (set-buffer-option! pane "word-chars" "-"))))
```

`bind-key!` takes a command *name* (the same names listed in [Builtin Commands](builtin-commands.md)), not a key sequence — there's no key-to-key remapping.

## What's the same

These work the way you expect, same keys:

- Select-then-act, selections always covering at least one character, multiple selections as a first-class tool
- `;` reduce to cursor, `,` keep only the main selection, `(` / `)` rotate the main selection
- `%` select the whole buffer, `s` narrow each selection to its regex matches
- `&` align selections, `_` trim surrounding whitespace
- `C` duplicate the selection onto the line below
- `>` / `<` indent / unindent the lines a selection touches, same keys and idea. HUME rewrites each line's whole indent in the buffer's `indent-style`; see [Lines and alignment](editing.md#lines-and-alignment)
- `i` / `a` / `I` / `A` / `o` / `O` to enter Insert, `c` / `d` / `y` / `p` to change / delete / yank / paste
- `Q` to record a macro and `q` to play it
- `u` undo, `U` redo
- `g h` line start, `g l` line end, `g g` first line

## Key differences

### Less shell integration

Kakoune is built to hand text to other programs: `|` pipes selections through a command, `!` inserts a command's output, and `%sh{ … }` expansions let the config shell out for anything the editor doesn't do itself.

HUME keeps `|`, with the same newline rule as Kakoune's, and inserts a command's output with `Ctrl-|` (kitty only) in Insert mode instead of `!` (see [Shell commands](editing.md#shell-commands)). The rest differs by design. Kakoune composes with UNIX; HUME embeds a language. Anything you would reach for `%sh{}` to do is written in Scheme instead, running inside the editor with direct access to buffers, selections and commands. That buys tighter integration and costs you the shell ecosystem.

### Extending selections

This is the one that will trip you up most often. In Kakoune, holding Shift extends: `w` replaces the selection, `W` extends it, and the same goes for `H`, `J`, `K`, `L`. There is no extend *mode*: `v` opens view mode.

HUME's uppercase keys are not extends. `W` and `B` are the WORD variants, the job Kakoune gives to `<a-w>` and `<a-b>`. Extending happens two other ways: **Extend mode**, toggled with `e`, where every motion extends until you act or press `Esc`; or a **one-shot** `Ctrl`+motion, which extends for a single keystroke without changing mode.

| Kakoune | HUME |
|---------|------|
| `W` (extend by word) | `e` then `w`, or `Ctrl-w` |
| `<a-w>` (WORD motion) | `W` |
| `H` / `J` / `K` / `L` | Extend mode, or `Ctrl-h` / `Ctrl-j` / `Ctrl-k` / `Ctrl-l` |
| `<a-;>` (flip direction) | `Ctrl-e` |
| `<a-,>` (remove main selection) | `Ctrl-,` |
| *(no equivalent)* | `e`, a persistent Extend mode |

The `Ctrl` one-shots and `Ctrl-,` need the kitty keyboard protocol; Extend mode works on any terminal. See [Terminal compatibility](installation.md#terminal-compatibility).

Bare `K` (no modifier) is not an extend at all in HUME: it shows LSP hover docs for the symbol under the cursor (with `core:lsp` loaded).

### Word characters

Kakoune's `extra_word_chars` is HUME's `word-chars` buffer option (see [Buffer options](settings.md#buffer-options)).

### Line selection

Kakoune's `x` expands the selection to cover full lines and keeps growing it on every press, and `<a-x>` trims a selection back to whole-line boundaries.

HUME's `x` re-anchors instead: each press selects one line and moves on, rather than accumulating. Growing is the job of the extend keys.

| Press | Kakoune `x` | HUME `x` | HUME `Ctrl-x` |
|-------|-------------|----------|---------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

`X` and `Ctrl-Shift-x` are the backward forms, so a line selection grown downward with `Ctrl-x` shrinks back up with `X` in Extend mode, or `Ctrl-Shift-x` (kitty only). There is no equivalent of `<a-x>`; `_` trims whitespace, not to line bounds.

### Splitting and merging selections

`S` is bound in both editors and means different things. Kakoune splits on a regex you type; HUME splits on newlines, which is Kakoune's `<a-s>`.

| Kakoune | HUME |
|---------|------|
| `<a-s>` (split on line boundaries) | `S` |
| `S` (split on a regex) | *(none)*: `s` narrows to regex matches instead |
| `<a-_>` (merge contiguous selections) | automatic: selections that share a character merge; touching ones stay separate |

### Search

The search keys overlap heavily in spelling and barely at all in meaning.

| Kakoune | Does | HUME |
|---------|------|------|
| `/` | Select next match | Search forward, with a live preview as you type |
| `?` | **Extend** to next match | Search *backward* |
| `<c-n>` | | **Extend** to next match |
| `n` | Move main selection to next match | same |
| `N` | **Add** a selection at the next match | Move main selection to the previous match |
| `m /` | | Select every match at once |
| `*` | Set search pattern from the selection, smart word boundaries | Set it from the word under the cursor, ignoring the selection |
| `<a-*>` | Set pattern from the selection, verbatim | |
| `<c-/>` | | Set pattern from the selection, verbatim |

::: warning
Two traps: `?` opens a backward search in HUME (Vim-style), not an extend like Kakoune's. And `N` has no equivalent — use `m /` to select every match at once instead, then narrow with `(`/`)` and `Ctrl-,` (`,` keeps only the primary). `Ctrl-,` needs kitty; on other terminals, bind `remove-primary-selection` to a key of your own, e.g. `(bind-key! 'normal "alt-," "remove-primary-selection")` for Kakoune's own `<a-,>`.
:::

Neither `*` moves the cursor; both just set the pattern for `n` to use. The difference is what they read: Kakoune uses whatever is selected, HUME expands to the whole word under the cursor and ignores the selection. `Ctrl-/` is the closer match to Kakoune's `*` family, and it needs kitty.

### Text objects

Kakoune reaches objects with `<a-i>` and `<a-a>`. HUME puts them behind an `m` prefix, so `<a-i>w` becomes `m i w` and `<a-a>(` becomes `m a (`.

| Kakoune | HUME |
|---------|------|
| `<a-i>w` / `<a-a>w` | `m i w` / `m a w` |
| `<a-i>(` / `<a-a>(` | `m i (` / `m a (` |
| `m` (jump to matching pair) | `#` |
| `[` / `]` / `{` / `}` (to object start/end) | *(none)* |

Two things to watch: Kakoune's `m` jumps to the matching bracket/tag (HUME's is `#`, since `m` is taken by text objects; select the pair instead with `m s` + delimiter, e.g. `m s (`). And `[`/`]`/`{`/`}` are all bound elsewhere: kill-ring cycling and paragraph motions.

The objects available are word (`w`), WORD (`W`), the bracket and quote pairs, argument (`a`), and line (`l`). HUME adds `m i i`, which selects the text you typed during your last insert, and `m w` + a delimiter, which wraps each selection in a pair.

For a language with a tree-sitter grammar that ships a textobjects query, HUME also adds `m i f`/`m a f` (function), `m i t`/`m a t` (class/type), `m i c`/`m a c` (comment), `m i u`/`m a u` (unit test), and `m i v`/`m a v` (array/tuple/struct value). Kakoune has no built-in equivalent. Each pairs with a `goto-next-<kind>`/`goto-prev-<kind>` command that jumps to the next/previous one as a selection, on the same letter under `g` (lowercase forward, uppercase backward, e.g. `g f`/`g F`).

### Registers, paste, and the clipboard

Kakoune registers are lists of text, one entry per selection, and you name any of them with `"` plus a character: `"` for the default, `/` for search, `@` for macros, `^` for marks. HUME's set is small and fixed:

| Name | Holds |
|------|-------|
| `"0`–`"9` | Numbered storage: text *or* macros, last write wins |
| `"k` | Kill-ring head |
| `"c` | System clipboard |
| `"b` | Black hole: writes discarded |

There is no arbitrary `"x`, and no register holding the buffer name or selection indices. Kakoune's `%`, `.` and `#` registers exist to feed `%sh{}`, which HUME has no use for.

The clipboard is also reachable directly as `"c`. `[`/`]` cycle through older/newer kill-ring entries after a paste.

See [Register prefix](copy-and-paste.md#register-prefix) for the full syntax.

### Macros

Same keys (`Q` starts and stops recording, `q` replays), but the storage differs. Kakoune records into `@` by default and takes any other register through the `"` prefix. HUME records into `q` by default (`Q Q` to record, `q q` to play) and otherwise only accepts digits: `Q 3` records into register 3. Those digit slots are the same ones `"3y` writes to, and the last write wins, so keep macros and yanked text on separate numbers.

### Repeat

Kakoune's `.` repeats the last insert-mode change, and `<a-.>` repeats the last object or `f`/`t` selection. HUME's `.` is broader: it repeats the last editing command *or* insert session: deletes, changes and pastes included. Give it a count to override the original. Repeating a find is `=` forward and `-` backward.

### Splits, windows, and tabs

Kakoune has no window management by design: you run multiple clients against one session and let tmux or your window manager arrange them.

HUME has panes built in: `Ctrl-p` is the prefix, `Ctrl-p s` and `Ctrl-p v` split, `Ctrl-p h/j/k/l` move focus, `Ctrl-p c` closes. `:sp` and `:vsp` do the same from the command mode prompt. There is no client/server model, so no attaching a second client to a running session.

On top of that, a **tab** saves a whole pane layout: its own splits and focused pane. `:tabnew` opens a second arrangement, `:tabclose` drops it, and `Ctrl-p t` / `Ctrl-p T` cycle between them. Buffers are shared across every tab; only the layout differs. The nearest Kakoune analogue is a second tmux window with another client attached to the same session. See [Tabs](files-and-buffers.md#tabs).

### User modes

Kakoune's user mode (the `Space` leader) and `declare-user-mode` have no direct equivalent. HUME's prefixes (`g`, `m`, `z`, `Ctrl-p`) can't be declared from your config.

### Plugins and language servers

Kakoune has neither a package manager nor LSP support in the core; both come from outside, via plug.kak and kakoune-lsp.

Both ship with HUME. [PLUM](core-plugins.md#core-plum) is the built-in plugin manager: declare a plugin in `init.scm`, run `:plum-install-plugins`, and it is fetched from GitHub. [Installing a plugin](plugins.md#installing-a-plugin) walks through it with [grep.hume](https://github.com/cvlmtg/grep.hume), a live-grep picker and HUME's first official third-party plugin.

Language server support is a bundled plugin rather than a separate process you configure by hand. See [Language Servers](lsp.md). Syntax highlighting is tree-sitter based and built in.

## What Kakoune has that HUME doesn't

- `!` and `%sh{}` expansions: Normal-mode command output, and shelling out from config
- The client/server model, and multiple clients on one session
- Marks (Kakoune's `^` register)
- Selection undo and redo (`<a-u>` / `<a-U>`)
- Adding a selection at the next match (`N`)
- Rotating selection *contents* (`<a-)>`), as opposed to rotating which selection is primary
- User modes and a `Space` leader
- Arbitrary single-character registers, and registers holding lists rather than single values
- Duplicating a selection onto the line *above* (`<a-C>`); HUME only binds the downward form
- Trimming a selection to whole-line bounds (`<a-x>`)

## What HUME has that Kakoune doesn't

- A built-in system clipboard, a [kill ring, and a paste](copy-and-paste.md) that picks the right source
- [Extend mode](#extending-selections), plus one-shot extends that need no mode switch
- Built-in [panes and splits](files-and-buffers.md#splits-and-panes), and [tab pages](files-and-buffers.md#tabs)
- A built-in [plugin manager](core-plugins.md#core-plum), [Scheme scripting](configuration.md), and a [hook system](writing-plugins.md#hooks)
- Bundled [language-server](lsp.md) and [tree-sitter](syntax-highlighting.md) support
- [Dot-repeat](editing.md#repeat) covering arbitrary edits, not just insert-mode changes
- A browsable graph of the undo history ([`core:undotree`](core-plugins.md#core-undotree)), and undo by time with [`:earlier 5m`](command-mode.md#editing)
- [Selecting every search match at once](moving-around.md#search-navigation) (`m /`)
- Tree-sitter-backed [structural text objects](#text-objects) and [navigation](moving-around.md#structural-navigation) (functions, classes, arguments, comments, tests)
