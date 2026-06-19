# HUME as a Golf Club

Vimgolf is a game: given a starting text and a target text, transform one into
the other using as few keystrokes as possible. Every character you type (and
every named key like `<esc>` or `<ret>`) costs one stroke. Lowest score wins.

It is also a surprisingly useful lens on a modal editor. The pressure to be
terse forces you to discover idioms — the vocabulary of movements, selections,
and actions that make a modal editor expressive rather than just different.

HUME is a kakoune-style editor, which means its editing model differs from Vim
in a fundamental way: **you select first, then act**. In Vim you typically say
"do *what* to *where*" (e.g. `dw` — delete, then word). In HUME you say "select
*where*, then do *what*" (e.g. `wd` — extend to next word, then delete it). This
changes which challenges HUME wins, ties, or loses.

---

## Challenge 1 — Delete the first line

**Input**

```
first line
second line
```

**Output**

```
second line
```

**HUME solution:** `xd` (2 keystrokes)

- `x` — select the current line (including its newline).
- `d` — delete the selection.

**Vim equivalent:** `dd` (2 keystrokes — a tie).

Both editors reach the same score here. The idioms are different (`xd` vs
`dd`) but the keystroke count is identical. In HUME, `x` *selects* the line
rather than implying deletion; `d` then acts on whatever was selected. In Vim,
`dd` is a single composite "delete line" command — no separate selection step.

---

## Challenge 2 — Add an exclamation mark to the end of every line

**Input**

```
foo
bar
baz
```

**Output**

```
foo!
bar!
baz!
```

**HUME solution:** `%SA!<esc>` (5 keystrokes)

- `%` — select the entire buffer (one big selection spanning all text).
- `S` — split the selection on newlines, turning one selection into one per
  line. This is HUME's multiple-cursor creation idiom.
- `A` — move all cursors to the end of their respective lines and enter insert
  mode. Every cursor is now positioned after the last character of its line.
- `!` — type the character to insert (simultaneously, at every cursor).
- `<esc>` — exit insert mode.

**Vim equivalent:** a common approach is `qaA!<esc>j0q2@a` or `:%s/$\!/g<enter>`
— the first is 12 keystrokes, the second is 11. HUME's multiple-cursor model
gives a clear advantage here: once all lines share a single distributed
selection, the insertion happens once and applies everywhere.

This is the challenge where select-then-act is not just *different* — it is
genuinely shorter.

---

## Challenge 3 — Replace a specific word

**Input**

```
foo bar baz
```

**Output**

```
foo qux baz
```

**HUME solution:** `/bar<ret>cqux<esc>` (10 keystrokes)

- `/bar<ret>` — search forward for "bar". This creates a selection on the
  match. (Named key `<ret>` confirms the search.)
- `c` — *change*: delete the selection and enter insert mode.
- `qux` — type the replacement.
- `<esc>` — exit insert mode.

**Vim equivalent:** `/bar<enter>cwqux<esc>` is also 10 keystrokes (a tie),
or `:%s/bar/qux<enter>` is 13. The surface syntax differs — HUME's `c`
(change) acts on the already-selected match, while Vim's `cw` is "change word"
from the cursor position — but the score ties.

---

## A note on cross-editor comparison

The scores above are for fun and orientation, not rigorous benchmarking.
Different editors have different primitive vocabularies; a challenge that
rewards a Vim-specific idiom will look bad for HUME, and vice versa. The
goal is to understand your own editor's idioms more deeply, not to declare a
winner.

The most transferable lesson: challenges that involve repeating the same action
across many locations are where HUME's multiple-cursor model shines. Challenges
that map cleanly to a single built-in verb (like `dd`) tend to be ties.

---

## Running the challenges yourself

See `tools/golf/README.md` for setup and usage.
