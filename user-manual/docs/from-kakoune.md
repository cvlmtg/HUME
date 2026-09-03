# Coming From Kakoune

Kakoune invented the editing model HUME is built on, so more transfers here than from any other editor. Selections come first, operators act on them, and multiple selections are the normal way to work rather than a special mode. What differs is how you *extend* a selection, what the search keys do, and everything downstream of Kakoune's shell-first philosophy. For a hands-on introduction, run `:tutor` inside the editor.

## What's the same

These work the way you expect, same keys:

- Select-then-act, selections always covering at least one character, multiple selections as a first-class tool
- `;` reduce to cursor, `,` keep only the main selection, `(` / `)` rotate the main selection
- `%` select the whole buffer, `s` select regex matches within the selection
- `&` align selections, `_` trim surrounding whitespace
- `C` duplicate the selection onto the line below
- `>` / `<` indent / unindent the lines a selection touches, same keys and idea — HUME additionally re-renders each touched line's whole indent to the buffer's `tab-width`/`tab-style` rather than only prepending or trimming a fixed amount (blank and whitespace-only lines are left alone), and flattens an indent narrower than one level to the left margin instead of going negative
- `i` / `a` / `I` / `A` / `o` / `O` to enter Insert, `c` / `d` / `y` / `p` to change / delete / yank / paste
- `Q` to record a macro and `q` to play it
- `u` undo, `U` redo
- `g h` line start, `g l` line end, `g g` first line

## Key differences

### Extending selections

This is the one that will trip you up most often. In Kakoune, holding Shift extends: `w` replaces the selection, `W` extends it, and the same goes for `H`, `J`, `K`, `L`. There is no extend *mode* — `v` opens view mode.

HUME's uppercase keys are not extends. `W` and `B` are the WORD variants, the job Kakoune gives to `<a-w>` and `<a-b>`. Extending happens two other ways: **Extend mode**, toggled with `e`, where every motion extends until you act or press `Esc`; or a **one-shot** `Ctrl`+motion, which extends for a single keystroke without changing mode.

| Kakoune | HUME |
|---------|------|
| `W` (extend by word) | `Ctrl+w`, or `e` then `w` |
| `<a-w>` (WORD motion) | `W` |
| `H` / `J` / `K` / `L` | `Ctrl+h` / `Ctrl+j` / `Ctrl+k` / `Ctrl+l`, or Extend mode |
| `<a-;>` (flip direction) | `Ctrl+e` |
| `<a-,>` (remove main selection) | `Ctrl+,` |
| *(no equivalent)* | `e` — a persistent Extend mode |

The `Ctrl` one-shots and `Ctrl+,` need the kitty keyboard protocol; Extend mode works on any terminal. See [Terminal compatibility](installation.md#terminal-compatibility).

Bare `K` (no modifier) is not an extend at all in HUME — it shows LSP hover docs for the symbol under the cursor (with `core:lsp` loaded).

### Word motions

Both editors re-anchor on every press, and both pull in a run of whitespace — but from opposite sides. Kakoune's `w` takes the word and the whitespace that *follows* it. HUME's `w` takes the word and the whitespace *before* it, except on the first word of a line, where a leading run would be indentation, so it takes the trailing whitespace instead.

<div class="key-demo">
<strong>Cursor on the first character</strong><br>
Kakoune&nbsp;&nbsp;<span class="head">L</span>orem ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;<span class="head">L</span>orem ipsum dolor sit<br>
<br>
<strong>Press <code>w</code></strong><br>
Kakoune&nbsp;&nbsp;<span class="sel">Lorem<span class="head">&nbsp;</span></span>ipsum dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
<br>
<strong>Press <code>w</code> again</strong><br>
Kakoune&nbsp;&nbsp;Lorem <span class="sel">ipsum<span class="head">&nbsp;</span></span>dolor sit<br>
HUME&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;Lorem ipsum<span class="sel">&nbsp;dolo<span class="head">r</span></span> sit
</div>

With words separated by single spaces the two land on visually similar spans; they diverge around punctuation and line ends. Turn off `word-selects-whitespace` (see [Configuration](configuration.md)) if you would rather `w` selected the bare word.

Kakoune's `e` has no counterpart — `e` toggles Extend mode here. To select the word the cursor already sits on, use `m m` (word plus one whitespace run) or `m i w` (bare word, Kakoune's `<a-i>w`).

Kakoune's `extra_word_chars` option (default `_`) is HUME's `word-chars` buffer option — list the extra characters directly, e.g. `-` for CSS. HUME ships with no default set; configure it per language from an `on-language-set` hook (see [Configuration](configuration.md)).

### Line selection

Kakoune's `x` expands the selection to cover full lines and keeps growing it on every press, and `<a-x>` trims a selection back to whole-line boundaries.

HUME's `x` re-anchors instead: each press selects one line and moves on, rather than accumulating. Growing is the job of the extend keys.

| Press | Kakoune `x` | HUME `x` | HUME `Ctrl+x` |
|-------|-------------|----------|---------------|
| 1st | Select whole line | Select whole line | Select whole line |
| 2nd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |
| 3rd | Extend to next line | Jump to next line (re-anchor) | Extend to next line |

`X` and `Ctrl+X` are the backward forms, so a line selection grown downward with `Ctrl+x` shrinks back up with `Ctrl+X`. There is no equivalent of `<a-x>`; `_` trims whitespace, not to line bounds.

### Splitting and merging selections

`S` is bound in both editors and means different things. Kakoune splits on a regex you type; HUME splits on newlines, which is Kakoune's `<a-s>`.

| Kakoune | HUME |
|---------|------|
| `<a-s>` (split on line boundaries) | `S` |
| `S` (split on a regex) | *(none)* — `s` selects regex matches instead |
| `<a-_>` (merge contiguous selections) | automatic — adjacent selections never persist |

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
`?` is the trap. In Kakoune it extends to the next match; in HUME it opens a backward search, the way it does in Vim.

`N` is the other one. Kakoune's incremental "add the next match too" has no key here — HUME goes the other way and gives you `m /`, which turns *every* match in the buffer into a selection in one press. Narrow from there with `,` and `(` / `)`.
:::

Neither `*` moves the cursor; both just set the pattern for `n` to use. The difference is what they read: Kakoune uses whatever is selected, HUME expands to the whole word under the cursor and ignores the selection. `Ctrl+/` is the closer match to Kakoune's `*` family, and it needs kitty.

### Text objects

Kakoune reaches objects with `<a-i>` and `<a-a>`. HUME puts them behind an `m` prefix, so `<a-i>w` becomes `m i w` and `<a-a>(` becomes `m a (`.

| Kakoune | HUME |
|---------|------|
| `<a-i>w` / `<a-a>w` | `m i w` / `m a w` |
| `<a-i>(` / `<a-a>(` | `m i (` / `m a (` |
| `m` (jump to matching pair) | `#` |
| `[` / `]` / `{` / `}` (to object start/end) | *(none)* |

Two things to watch. Kakoune's `m` jumps to the matching bracket or tag; HUME's `m` prefix is the text-object key instead, so the jump moved to `#`. To select the surrounding pair rather than jump to it, use `m s` + the delimiter — `m s (` for parens — which needs you to name the delimiter, where Kakoune's `m` finds the enclosing pair on its own. And `[`, `]`, `{`, `}` are all taken: `[` and `]` cycle the kill ring after a paste, `{` and `}` are paragraph motions.

The objects available are word (`w`), WORD (`W`), the bracket and quote pairs, argument (`a`), and line (`l`). HUME adds `m i i`, which selects the text you typed during your last insert, and `m w` + a delimiter, which wraps each selection in a pair.

For a language with a tree-sitter grammar that ships a textobjects query, HUME also adds `m i f`/`m a f` (function), `m i t`/`m a t` (class/type), `m i c`/`m a c` (comment), `m i u`/`m a u` (unit test), and `m i v`/`m a v` (array/tuple/struct value) — Kakoune has no built-in equivalent. Each pairs with a `goto-next-<kind>`/`goto-prev-<kind>` command that jumps to the next/previous one as a selection, on the same letter under `g` (lowercase forward, uppercase backward — e.g. `g f`/`g F`).

### Registers, paste, and the clipboard

Kakoune registers are lists of text, one entry per selection, and you name any of them with `"` plus a character — `"` for the default, `/` for search, `@` for macros, `^` for marks. HUME's set is small and fixed:

| Name | Holds |
|------|-------|
| `"0`–`"9` | Numbered storage — text *or* macros, last write wins |
| `"k` | Kill-ring head |
| `"c` | System clipboard |
| `"b` | Black hole — writes discarded |

There is no arbitrary `"x`, and no register holding the buffer name or selection indices — Kakoune's `%`, `.` and `#` registers exist to feed `%sh{}`, which HUME has no use for.

The larger difference is what happens by default. Kakoune ships without clipboard integration; you wire up `xclip` or `pbcopy` through a pipe. HUME has the system clipboard built in as `"c`, plus a kill ring behind it, and `p` picks between them: it pastes from the kill ring while nothing has been edited since your last delete/change/yank, and from the clipboard once something has. Right after a paste, `[` and `]` swap in older and newer kill-ring entries.

See [Register prefix](copy-and-paste.md#register-prefix) for the full syntax.

### Macros

Same keys — `Q` starts and stops recording, `q` replays — but the storage differs. Kakoune records into `@` by default and takes any other register through the `"` prefix. HUME records into `q` by default (`Q Q` to record, `q q` to play) and otherwise only accepts digits: `Q 3` records into register 3. Those digit slots are the same ones `"3y` writes to, and the last write wins, so keep macros and yanked text on separate numbers.

### Repeat

Kakoune's `.` repeats the last insert-mode change, and `<a-.>` repeats the last object or `f`/`t` selection. HUME's `.` is broader: it repeats the last editing command *or* insert session — deletes, changes and pastes included. Give it a count to override the original. Repeating a find is `=` forward and `-` backward.

### No shell integration

Kakoune is built to hand text to other programs: `|` pipes selections through a command, `!` inserts a command's output, and `%sh{ … }` expansions let the config shell out for anything the editor doesn't do itself. None of that exists in HUME.

The philosophies genuinely differ here. Kakoune composes with UNIX; HUME embeds a language. Anything you would reach for `%sh{}` to do is written in Scheme instead, running inside the editor with direct access to buffers, selections and commands. That buys tighter integration and costs you the entire shell ecosystem.

### Splits and windows

Kakoune has no window management by design — you run multiple clients against one session and let tmux or your window manager arrange them.

HUME has panes built in: `Ctrl+p` is the prefix, `Ctrl+p s` and `Ctrl+p v` split, `Ctrl+p h/j/k/l` move focus, `Ctrl+p c` closes. `:sp` and `:vsp` do the same from the command mode prompt. There is no client/server model, so no attaching a second client to a running session.

### Configuration

Kakoune's `kakrc` is kakscript: `map` for keys, `set-option` for options, `hook` for events, `define-command` for new commands. HUME's [`init.scm`](configuration.md) is Scheme, with the same four ideas under different names:

```scheme
(set-option! "theme" "sand")
(bind-key! 'normal "ctrl-j" "move-down")
```

Being a real programming language, the config has conditionals, loops and abstraction from day one — closer to what you would reach `%sh{}` for in kakrc, without leaving the editor.

One thing that does not carry over: Kakoune's user mode (the `Space` leader) and `declare-user-mode` have no direct equivalent. HUME's prefixes — `g`, `m`, `z`, `Ctrl+p` — are fixed rather than user-declarable.

### Plugins and language servers

Kakoune has neither a package manager nor LSP support in the core; both come from outside, via plug.kak and kakoune-lsp.

Both ship with HUME. [PLUM](core-plugins.md#core-plum) is the built-in plugin manager — declare a plugin in `init.scm`, run `:plum-install-plugins`, and it is fetched from GitHub — here's [grep.hume](https://github.com/cvlmtg/grep.hume), a live-grep picker and HUME's first official third-party plugin:

```scheme
(declare-plugin "core:stdlib")
(load-plugin "cvlmtg/grep.hume")
```

Language server support is a bundled plugin rather than a separate process you configure by hand — see [Language Servers](lsp.md). Syntax highlighting is tree-sitter based and built in.

## What Kakoune has that HUME doesn't

- Shell pipes and `%sh{}` expansions — `|`, `!`, and shelling out from config
- The client/server model, and multiple clients on one session
- Marks (Kakoune's `^` register)
- Selection undo and redo (`<a-u>` / `<a-U>`)
- Adding a selection at the next match (`N`)
- Rotating selection *contents* (`<a-)>`), as opposed to rotating which selection is primary
- User modes and a `Space` leader
- Arbitrary single-character registers, and registers holding lists rather than single values
- Duplicating a selection onto the line *above* (`<a-C>`) — HUME only binds the downward form
- Trimming a selection to whole-line bounds (`<a-x>`)

## What HUME has that Kakoune doesn't

- A built-in system clipboard, a kill ring, and a paste that picks the right source
- Extend mode, plus one-shot extends that need no mode switch
- Built-in panes and splits
- A built-in plugin manager, Scheme scripting, and a hook system
- Bundled language-server and tree-sitter support
- Dot-repeat covering arbitrary edits, not just insert-mode changes
- An undo tree
- Selecting every search match at once (`m /`)
- Tree-sitter-backed structural text objects and navigation (functions, classes, arguments, comments, tests)
