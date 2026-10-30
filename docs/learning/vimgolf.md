# HUME as a Golf Club

Vimgolf is a game: given a starting text and a target text, transform one
into the other in as few keystrokes as possible. Every character you type,
and every named key like `<esc>` or `<ret>`, costs one stroke. Lowest score
wins.

It is also a useful lens on a modal editor. The pressure to be terse forces
you to discover idioms: the vocabulary of movements, selections, and actions
that makes a modal editor expressive rather than just different.

HUME is a Kakoune-style editor, so its editing model differs from Vim's in
one fundamental way: **you select first, then act**. In Vim you typically say
"do *what* to *where*" (`dw`: delete, then word). In HUME you say "select
*where*, then do *what*" (`wd`: select the next word, with its adjacent
whitespace by default, then delete the selection). This changes which
challenges HUME wins, ties, or loses.

---

## Challenge 1: delete the first line

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

- `x`: select the current line, including its newline.
- `d`: delete the selection.

**Vim equivalent:** `dd` (2 keystrokes, a tie).

The idioms differ but the count is the same. In HUME, `x` *selects* the line
and `d` acts on whatever is selected. In Vim, `dd` is one composite "delete
line" command with no separate selection step.

---

## Challenge 2: add an exclamation mark to the end of every line

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

- `%`: select the entire buffer as one selection.
- `S`: split the selection on newlines, one selection per line. This is how
  HUME creates multiple cursors.
- `A`: move every cursor to the end of its line and enter insert mode.
- `!`: type the character, at every cursor at once.
- `<esc>`: exit insert mode.

**Vim equivalent:** a common approach is `qaA!<esc>j0q2@a` (11 keystrokes)
or `:%s/$/!/g<enter>` (10 keystrokes). Once every line has its own
selection, HUME types the insertion once and it applies everywhere.

This is the challenge where select-then-act is not just *different* but
shorter.

---

## Challenge 3: replace a specific word

**Input**

```
foo bar baz
```

**Output**

```
foo qux baz
```

**HUME solution:** `/bar<ret>cqux<esc>` (10 keystrokes)

- `/bar<ret>`: search forward for "bar". The match becomes the selection.
- `c`: *change*, deleting the selection and entering insert mode.
- `qux`: type the replacement.
- `<esc>`: exit insert mode.

**Vim equivalent:** `/bar<enter>cwqux<esc>` is 11 keystrokes, and
`:%s/bar/qux<enter>` is 12, so HUME wins by a stroke or two. HUME's `c` acts
on the selected match, while Vim's `cw` changes a word from the cursor. The
match doubling as a selection is what saves the stroke.

---

## A note on cross-editor comparison

These scores are for fun and orientation, not benchmarking. Editors have
different primitive vocabularies: a challenge that rewards a Vim-specific
idiom will look bad for HUME, and the reverse. The goal is to understand your
own editor's idioms, not to declare a winner.

The most transferable lesson: challenges that repeat one action across many
places are where multiple cursors shine. Challenges that map to a single
built-in verb (like `dd`) tend to be ties.

---

## Running the challenges yourself

HUME ships a small golf harness with a corpus of real vimgolf problems:
reference solutions, input and output fixtures, and a runner that compares
HUME's keystroke count against Kakoune's. The three challenges above are
teaching examples; the corpus is where to test HUME on the real field.
