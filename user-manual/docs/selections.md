# Selections

Every editing command acts on the current selection. There is no cursor without a selection: even a single-character "cursor position" is a one-character selection.

## How selections work

A selection has two ends: the **anchor** and the **head**. The head is the moving end; the anchor stays fixed until you reset the selection. The selection always covers at least one character.

## Building a selection

### Whole lines

| Key | Effect |
|-----|--------|
| `x` | Select the current line. On a line that is already selected, select the next line instead |
| `X` | Select the current line. On a line that is already selected, select the line above instead |
| `Ctrl-x` | Grow the selection by one line downward |
| `Ctrl-Shift-x` | Grow the selection by one line upward, or shrink one grown downward (kitty only) |

`x` then `d` deletes a line, `x` then `y` copies it. Each `x` replaces the selection rather than adding to it, so pressing `x` three times leaves only the third line selected. To select three lines, press `x` then `Ctrl-x` twice, or use [Extend mode](#extend-mode).

### Extend mode

Press `e` to enter Extend mode. Every motion then moves only the head while the anchor stays put: moving away grows the selection, and moving back toward where you started shrinks it. Press `e` again or `Esc` to return to Normal. The status bar shows `EXT` while Extend mode is active.

For a one-shot extend without entering Extend mode, hold `Ctrl` with a motion key under the kitty keyboard protocol (`Ctrl-h`/`Ctrl-j`/`Ctrl-k`/`Ctrl-l`, `Ctrl-w`/`Ctrl-b`, `Ctrl-f`/`Ctrl-t`, `Ctrl-g h`, and so on): that one keypress runs the motion with extend on. Keys with their own `Ctrl` binding keep it (`Ctrl-d`, `Ctrl-u`, `Ctrl-o`, `Ctrl-e`), and `Ctrl` with a key that isn't a motion does nothing. `Ctrl-x` extends the line selection downward on any terminal; its backward twin `Ctrl-Shift-x` needs kitty, since older terminals can't tell the two apart.

Search has the same one-shot extend: `Ctrl-n` (kitty only) moves the head to the next search match while the anchor stays put, so the selection covers everything from where you started through that match. `Ctrl-Shift-n` does the same backward, extending to the previous match.

::: tip Extending to a brand-new search
Extend mode also works with a new `/` or `?` search, not only with `Ctrl-n`/`Ctrl-Shift-n` stepping through an existing one. Press `e`, then search: the anchor stays where you were, and the head jumps to the first match as you type, and again on every `n`/`N` afterward.
:::

In Extend mode, `w`/`b` and `x`/`X` also shrink in whole units: the opposite key shrinks the selection one word or one line at a time. The word or line where you started stays fully selected whichever way you grow or shrink, and crossing back past it flips the selection's direction instead of cutting it off partway.

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

The anchor stays on `Lorem`'s trailing whitespace throughout: `w` grows the head forward one word at a time, and `b` shrinks it back the same way.

### Text objects

Text objects select structured regions in one step. They use the `m` prefix: `m i` for inner content, `m a` for around (including delimiters, or one adjacent whitespace run for words):

| Sequence | Selects |
|----------|---------|
| `m i w` / `m a w` | Inner word / word + one adjacent whitespace run (see `word-selects-whitespace`) |
| `m i W` / `m a W` | Inner WORD / WORD + one adjacent whitespace run (see `word-selects-whitespace`) |
| `m i (` / `m a (` | Inside `()` / including `()` |
| `m i [` / `m a [` | Inside `[]` / including `[]` |
| `m i {` / `m a {` | Inside `{}` / including `{}` |
| `m i <` / `m a <` | Inside `<>` / including `<>` |
| `m i "` / `m a "` | Inside `"…"` / including `"…"` |
| `m i '` / `m a '` | Inside `'…'` / including `'…'` |
| `` m i ` `` / `` m a ` `` | Inside `` `…` `` / including `` `…` `` |
| `m i a` / `m a a` | Argument (trimmed) / argument + separator comma |
| `m i l` / `m a l` | Line content (no newline) / full line (with newline) |
| `m i p` / `m a p` | Paragraph text / the paragraph plus its trailing blank gap |
| `m i f` / `m a f` | Inside a function / the function including its signature (and attributes, decorators, …) |
| `m i t` / `m a t` | Inside a class or type / the class or type including its header |
| `m i c` / `m a c` | Inside a comment / the whole comment block |
| `m i u` / `m a u` | Inside a unit test function's body / the whole unit test, including its attribute or decorator |
| `m i v` / `m a v` | Inside an array/tuple/struct value / the value plus its separator comma |
| `m i x` / `m a x` | Between a pair of tags / the tags and everything between them |

Closing brackets work as well as opening ones: `m i )` is the same as `m i (`, and likewise for `]`, `}`, and `>`.

`m i a` / `m a a` and the last six rows above (`f`, `t`, `c`, `u`, `v`, `x`) are structure-aware. For a language whose grammar ships a `textobjects.scm` (PLUM installs one alongside highlights where the upstream grammar has one), they select the actual function, class, comment, unit test, value, or tag pair. Wherever the grammar doesn't cover the cursor (a syntax error, a buffer with no grammar at all), `a` and `x` fall back to a lexical scan. Without a grammar, `f`/`t`/`c`/`u`/`v` do nothing, with no message. Where the grammar covers it, a nested list, tuple, or struct literal passed as a call argument is itself the argument; use `m i v` / `m a v` for its members.

`m i x` selects the content without its surrounding whitespace. With the cursor on a tag itself, it selects the content of the enclosing pair.

Under the `g` prefix, each of `f`/`t`/`a`/`c`/`u`/`v`/`x` also jumps to the next or previous instance of its kind (lowercase forward, uppercase backward, e.g. `g f`/`g F`). See [Moving Around](moving-around.md#structural-navigation).

Two shortcuts select the word under the cursor directly:

| Key | Effect |
|-----|--------|
| `m m` | Word under the cursor, same as `m a w`: with one adjacent whitespace run by default, by the same rule as `w`/`b`. Set `word-selects-whitespace` to `off` to get `m i w` instead (see [Buffer options](settings.md#buffer-options)) |
| `M M` | WORD under the cursor, same as `m a W` |

How `m i` and `m a` differ on a word and on brackets:

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

`m i i` selects the text you most recently typed before leaving Insert mode, however you entered it (`i`, `a`, `o`, `O`, `A`, `I`, `c`), so you can act on what you just wrote. It follows the inserted text through later edits, and stops working only once that text is deleted or the buffer's text is replaced. There is no `m a i`: an insertion has no delimiters or surrounding structure to select "around".

## Select all

| Key | Effect |
|-----|--------|
| `%` | Select entire buffer |
| `m /` | Turn every search match in the buffer into a selection. See [Moving Around](moving-around.md#search-navigation) |

## Flipping and collapsing the selection

| Key | Effect |
|-----|--------|
| `;` | Collapse selection to head and exit Extend mode |
| `Ctrl-;` | Collapse selection to anchor and exit Extend mode (kitty only) |
| `Ctrl-e` | Swap anchor and head of each selection (any mode; works on legacy terminals too) |

## Multiple selections

HUME can hold several selections at once. Editing commands act on all of them, each independently.

| Action | Key | Effect |
|--------|-----|--------|
| Sift within selection | `s` | Enter a regex pattern; each selection is narrowed to its sub-matches |
| Split on newlines | `S` | Split multi-line selections into one selection per line |
| Copy to next line | `C` | Duplicate each selection to the same character columns on the line below, adding a cursor. No text is copied. Each copy sits one selection-height below the last: a one-line selection copies to the next line, a three-line one three lines further down. A count (e.g. `3C`) adds that many copies at once, stopping where the buffer ends. Repeating `C` also stacks cursors line by line for column-style editing. HUME has no rectangular (block) selection. |
| Trim whitespace | `_` | Shrink each selection so it starts and ends on non-whitespace; the text is unchanged |
| Keep primary | `,` | Remove all selections except the primary |
| Remove primary | `Ctrl-,` | Remove the primary selection, promote next (kitty only) |
| Cycle primary forward | `)` | Make the next selection the primary |
| Cycle primary backward | `(` | Make the previous selection the primary |

### Sift within (`s`)

Press `s` to enter Sift mode, type a regex pattern and press `Enter`. Each selection is narrowed to the parts matching the pattern, one new selection per match. This splits a line into tokens, for example:

1. Select a line (`x`)
2. Press `s` and type `\w+` to select each word individually
3. Press `d` to delete all words at once

The pattern sees each selection's text on its own, without a line break the selection ends on: the same text `c` would change. So `^` and `$` match at the start and end of the selection, not of the line or the buffer, and `\b` treats the selection's edges as the edges of the text. Add `(?m)` to make `^` and `$` also match at the start and end of each line inside the selection. On `## Title` selected with `x`, `^#+` selects `##` and `Title$` selects `Title`. A line break the selection ends on is never matched, so a selection covering only an empty line has nothing to sift.

`s` needs at least one selection wider than a single character; on a bare single-character cursor it does nothing. Pressing `Enter` on an invalid pattern, or one with no match, reports it and leaves the selections as they were. See [Regex syntax](moving-around.md#regex-syntax) for the pattern flavor and case-sensitivity rules.
