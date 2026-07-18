# Inner vs Around: The Text Object Convention

## Why inner and around exist

When you want to change what's inside parentheses, you don't want to select the
parentheses themselves — just their contents. When you want to *delete* the
parentheses along with their contents, you need the parens included. This
distinction comes up constantly in code editing, and Vim named it first:
"inner" for the contents, "around" for contents plus delimiters.

The convention is productive because it composes: every text object that has a
well-defined "inside" and "outside" gets both variants for free, and you learn
one rule that applies everywhere.

## The two flavours

Every text object in HUME comes in two flavours:

- **inner (`i` prefix)**: the content *without* the delimiters. `mi(` selects
  the text inside parentheses; `miw` selects the word without surrounding
  space.
- **around (`a` prefix)**: the content *including* the delimiters. `ma(` selects
  the parentheses and their contents; `maw` selects the word plus one adjacent
  whitespace run.

## HUME's prefix: `mi` and `ma`

Vim uses `i` and `a` directly as the prefix key: `iw`, `a(`. HUME prefixes
with `m` (mnemonic: **m**atch or **m**ore): `miw`, `ma(`. This frees `i` and
`a` for their primary use as "enter insert before/after cursor" commands. The
`m` prefix also signals "I'm about to name a text region" — a consistent entry
point for text objects, surround operations, and other structural commands.

The `m` root grows a small family beyond `mi`/`ma`. `ms` followed by a
delimiter character (`ms(`, `ms[`, `ms{`, `ms<`, `ms"`, `ms'`, ``ms` ``) selects
the two delimiter characters as cursors — the building block for surround
editing, where you then act on both delimiters at once (replace them, delete
them, change them as a pair). `mw` plus a delimiter wraps the current selection
in that pair. `mm` and `MM` select the word/WORD under the cursor directly —
by default they cover surrounding whitespace too, matching how the plain word
motions `w`/`W`/`b`/`B` do (see [Word Motions](word-motions.md) for that
rule); an option restores the `miw`/`miW` (inner) behavior for all of them at
once. The same prefix keeps every "name a structural region" command under
one key.

## The trailing-whitespace rule for `maw`/`maW`

For bracket text objects, inner/around is straightforward: include or exclude
the delimiter characters. For word text objects, "around" requires a choice:
which whitespace runs to include when neither a leading nor trailing run is
obvious?

The rule `maw`/`maW` (following Vim) use: prefer to include trailing
whitespace, fall back to leading whitespace if there is no trailing space —
unconditionally, regardless of where the word falls on its line.

The reason: deleting "a word" should leave the surrounding text tidy. If you
have `one two three` and delete `aw` with the cursor on `two`, you want
`one three` with one space — not `one  three` (double-space) or `onetwo` (no
space). Including the trailing space produces the clean result in the common
case; the leading fallback handles the last word in a line.

This is a different rule from the one `w`/`b`/`mm` use by default — see
[Word Motions](word-motions.md). Both exist for the same reason (tidy
deletes), but `maw`/`maW` stay fixed at Vim's rule so they're always
available as an escape hatch, independent of the `word-selects-whitespace`
setting.

## In practice

The implementations share the code that locates the extent of the text object
(finding the bracket pair, scanning the word boundary) and differ only in what
range they return: inner stops just inside the delimiters, around includes them.
Both use the same underlying position logic — the `i`/`a` distinction is a
one-line change at the end.
