# Selections

Selections are central to how HUME works. Every editing command acts on the current selection — there is no cursor-without-selection. Even a single-character "cursor position" is a one-character selection.

## How selections work

A selection has two ends: the **anchor** and the **head**. The head is the moving end; the anchor stays fixed until you reset the selection. The selection always covers at least one character.

## Building a selection

### Extend mode

Press `e` to enter Extend mode. In Extend mode, every motion grows the selection instead of moving it — and moving back toward where you started shrinks it again, since only the moving end travels while the anchor stays put. Press `e` again or `Esc` to return to Normal. The status bar shows `EXT` while Extend mode is active.

You can also do a one-shot extend without entering Extend mode: under the kitty keyboard protocol, `Ctrl+h`/`Ctrl+j`/`Ctrl+k`/`Ctrl+l`/`Ctrl+w`/`Ctrl+b` run the corresponding motion with extend on for that single keypress. `Ctrl+x` extends the line selection downward on any terminal; its backward twin `Ctrl+X` needs kitty, since older terminals can't tell the two apart.

The same one-shot extend applies to search: `Ctrl+n` (kitty only) jumps the head to the next search match while the anchor stays put, growing the selection to cover everything from where you started through the new match — without entering Extend mode. `Ctrl+N` does the same backward, extending to the previous match.

`w`/`b` and `x`/`X` additionally shrink in whole units: pressing the opposite key shrinks the selection back down one word or one line at a time, rather than one character at a time. The word or line where you started stays fully selected no matter which way you shrink or grow from there — crossing back past your starting point flips the selection's direction instead of cutting it off partway.

<div class="key-demo">
<strong>Word selected with <code>w</code>, then <code>e</code> to enter Extend mode</strong><br>
Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor sit<br>
<br>
<strong>Press <code>w</code></strong><br>
Lorem<span class="sel">&nbsp;ipsum dolo<span class="head">r</span></span> sit<br>
<br>
<strong>Press <code>w</code> again</strong><br>
Lorem<span class="sel">&nbsp;ipsum dolor si<span class="head">t</span></span><br>
<br>
<strong>Press <code>b</code></strong><br>
Lorem<span class="sel">&nbsp;ipsum dolo<span class="head">r</span></span> sit
</div>

The anchor stays pinned on `Lorem`'s trailing whitespace throughout — `w` grows the head forward one word at a time, `b` shrinks it back the same way.

### Text objects

Text objects select structured regions in one step. They use the `m` prefix — `m i` for inner content, `m a` for around (including delimiters, or one adjacent whitespace run for words):

| Sequence | Selects |
|----------|---------|
| `m i w` / `m a w` | Inner word / word + one adjacent whitespace run |
| `m i W` / `m a W` | Inner WORD / WORD + one adjacent whitespace run |
| `m i (` / `m a (` | Inside `()` / including `()` |
| `m i [` / `m a [` | Inside `[]` / including `[]` |
| `m i {` / `m a {` | Inside `{}` / including `{}` |
| `m i <` / `m a <` | Inside `<>` / including `<>` |
| `m i "` / `m a "` | Inside `"…"` / including `"…"` |
| `m i '` / `m a '` | Inside `'…'` / including `'…'` |
| `` m i ` `` / `` m a ` `` | Inside `` `…` `` / including `` `…` `` |
| `m i a` / `m a a` | Argument (trimmed) / argument + separator comma |
| `m i l` / `m a l` | Line content (no newline) / full line (with newline) |
| `m i f` / `m a f` | Inside a function / the function including its signature (and attributes, decorators, …) |
| `m i t` / `m a t` | Inside a class or type / the class or type including its header |
| `m i c` / `m a c` | Inside a comment / the whole comment block |
| `m i u` / `m a u` | Inside a unit test function's body / the whole unit test, including its attribute or decorator |
| `m i v` / `m a v` | Inside an array/tuple/struct value / the value plus its separator comma |

Closing brackets work as well as opening ones: `m i )` is the same as `m i (`, and likewise for `]`, `}`, and `>`.

`m i a` / `m a a` and the last five rows above (`f`, `t`, `c`, `u`, `v`) are structure-aware: for a
language whose grammar ships a `textobjects.scm` (PLUM installs one alongside highlights where the
upstream grammar has one), they select the actual function, class, comment, unit test, or value node —
falling back to a lexical scan for `m i a` / `m a a` wherever the grammar doesn't cover the cursor
(a syntax error, a buffer with no grammar at all). Without a grammar, `f`/`t`/`c`/`u`/`v` are a
silent no-op. Because the argument object is now structure-aware, a nested list, tuple, or struct
literal passed as a call argument is itself the argument — use `m i v` / `m a v` for its members.

Each of `f`/`t`/`a`/`c`/`u`/`v` also jumps to the next/previous instance of its kind under the
`g` prefix (lowercase forward, uppercase backward, e.g. `g f`/`g F`) — see
[Moving Around](moving-around.md#structural-navigation).

Two shortcuts select the word under the cursor directly:

| Key | Effect |
|-----|--------|
| `m m` | Word under the cursor (plus one adjacent whitespace run by default, same rule as `w`/`b`; disable `word-selects-whitespace` for `m i w` instead — see [Configuration](configuration.md)) |
| `M M` | WORD under the cursor (same as `m a W` by default) |

There is no paragraph text object; use the `{` and `}` paragraph motions.

`m i` selects just the structure's content; `m a` includes what surrounds it — one adjacent whitespace run for words, the delimiters themselves for brackets:

<div class="key-demo">
<strong>Cursor mid-word, press <code>m</code> <code>i</code> <code>w</code></strong><br>
Lorem <span class="sel">ipsu<span class="head">m</span></span> dolor<br>
<strong>Press <code>m</code> <code>a</code> <code>w</code></strong><br>
Lorem<span class="sel">&nbsp;ipsu<span class="head">m</span></span> dolor<br>
<br>
<strong>Cursor inside the parens, press <code>m</code> <code>i</code> <code>(</code></strong><br>
call(<span class="sel">one, tw<span class="head">o</span></span>)<br>
<strong>Press <code>m</code> <code>a</code> <code>(</code></strong><br>
call<span class="sel">(one, two<span class="head">)</span></span>
</div>

`m i i` selects the text you most recently typed before leaving Insert mode — however you entered it (`i`, `a`, `o`, `O`, `A`, `I`, `c`). Type something, press `Esc`, then `m i i` to act on what you just wrote. It stops working as soon as you make another change to the buffer (including undo/redo). There is no `m a i` — an insertion has no delimiters or surrounding structure to select "around".

## Select all

| Key | Effect |
|-----|--------|
| `%` | Select entire buffer |
| `m /` | Turn every search match in the buffer into a selection — see [Moving Around](moving-around.md#search-navigation) |

## Flipping and collapsing the selection

| Key | Effect |
|-----|--------|
| `;` | Collapse selection to head and exit Extend mode |
| `Ctrl+;` | Collapse selection to anchor and exit Extend mode (kitty only) |
| `Ctrl+e` | Swap anchor and head of each selection (any mode; works on legacy terminals too) |

## Multiple selections

HUME supports multiple simultaneous selections. Each selection behaves independently — editing commands act on all of them at once.

| Action | Key | Effect |
|--------|-----|--------|
| Select within selection | `s` | Enter a regex pattern; each selection is filtered to its sub-matches |
| Split on newlines | `S` | Split multi-line selections into one selection per line |
| Copy to next line | `C` | Duplicate each selection to the same character column on the line below, adding a multi-cursor. No text is copied — the new selections cover the same column range on the next line. A count prefix (e.g. `3C`) copies onto that many lines below in one step; repeating `C` also stacks cursors line by line for column-style editing. HUME has no rectangular/visual-block selection primitive. |
| Trim whitespace | `_` | Remove leading/trailing whitespace from all selections |
| Keep primary | `,` | Remove all selections except the primary |
| Remove primary | `Ctrl+,` | Remove the primary selection, promote next (kitty only) |
| Cycle primary forward | `)` | Make the next selection the primary |
| Cycle primary backward | `(` | Make the previous selection the primary |

### Select within (`s`)

Press `s` to enter Select mode. Type a regex pattern and press `Enter`. Each existing selection is filtered to only the sub-ranges matching the pattern, creating one new selection per match. This is useful for splitting a line selection into individual tokens:

1. Select a line (`x`)
2. Press `s` and type `\w+` to select each word individually
3. Press `d` to delete all words at once

`s` requires at least one non-collapsed selection — on a bare single-character cursor it is a silent no-op. See [Regex syntax](moving-around.md#regex-syntax) for the pattern flavor and case-sensitivity rules.
