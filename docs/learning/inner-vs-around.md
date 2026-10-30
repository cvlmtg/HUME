# Inner vs Around: The Text Object Convention

## Why inner and around exist

When you want to change what is inside parentheses, you do not want to
select the parentheses themselves, only their contents. When you want to
*delete* the parentheses along with their contents, you need the parens
included. This distinction comes up constantly in code editing, and modal
editors name it "inner" for the contents and "around" for contents plus
delimiters.

The convention is productive because it composes: every text object that has
a well-defined "inside" and "outside" gets both variants, and you learn one
rule that applies everywhere.

## The two flavours

Nearly every text object in HUME comes in two flavours (the rare object with
no meaningful "outside", like last-insertion, has only an inner form):

- **inner (`i`)**: the content *without* the delimiters. `mi(` selects the
  text inside parentheses; `miw` selects the word without surrounding space.
- **around (`a`)**: the content *including* the delimiters. `ma(` selects the
  parentheses and their contents; `maw` selects the word plus one adjacent
  whitespace run.

## HUME's prefix: `mi` and `ma`

HUME reaches text objects through the `m` prefix (mnemonic: **m**atch), as
Helix does: `miw`, `ma(`. This leaves `i` and `a` free for their primary use,
entering insert before or after the selection, and gives every "name a text
region" command one entry point.

The `m` root holds a small family beyond `mi`/`ma`. `ms` followed by a
delimiter character (`ms(`, `ms[`, `ms{`, `ms<`, `ms"`, `ms'`, ``ms` ``)
selects the two delimiter characters as cursors, the building block for
surround editing, where you then act on both delimiters at once (replace
them, delete them, change them as a pair). `mw` plus a delimiter wraps the
current selection in that pair. `mm` and `MM` select the word/WORD under the
cursor directly; they are `maw`/`maW` under a shorter name (see below).

## The whitespace rule for `maw`/`maW`

For bracket text objects, inner/around is straightforward: include or
exclude the delimiter characters. For word text objects, "around" requires a
choice: which whitespace run to include?

The `word-selects-whitespace` setting picks the side. It applies to
`maw`/`maW`, `mm`/`MM`, and the word motions `w`/`W`/`b`/`B`, which is why
`mm`, `maw` and `w` land on identical spans.

- **`before`** (the default): take the whitespace *before* the word. When
  there is none, because the word starts its line or punctuation sits
  directly before it, take the whitespace *after* it instead.
- **`after`**: take the whitespace *after* the word, up to the end of its
  line. When nothing follows (the last word of a line, or a word right
  before punctuation), take the whitespace before it instead.
- **`off`**: the bare word. `maw`/`maW` then select the same as
  `miw`/`miW`.

Neither side ever absorbs a line's indentation: a leading run that touches
the line start is indentation, not inter-word spacing, so a lone word on an
indented line stays bare. Whenever neither side has whitespace to offer, the
selection stays bare.

The reason: deleting "a word" should leave the surrounding text tidy. With
`one two three` and the cursor on `two`, `mawd` gives `one three` with one
space, not `one  three` (double space) or `onetwo` (no space). Preferring
one side and falling back to the other produces the clean result for the
first and last words of a line too, and never eats indentation.

With the cursor on whitespace there is no word under it, so `maw` picks the
word that whitespace belongs to: the word after it under `before`, the word
before it under `after`, falling back to the word on the other side when
there is none. The space under the cursor is never selected for its own
sake. It is part of the selection when it is that word's chosen whitespace
run, and a newline or a line's indentation under the cursor never is.

## In practice

Inner and around locate the same extent (the bracket pair, the word's
boundaries) and differ in what they select: inner stops just inside the
delimiters, around includes them. For a word, around is not a fixed-width
grab. It grows the word by a scan over the adjacent whitespace, as described
above.

---

*See also: [Word Motions](word-motions.md) for the motions that share the
whitespace rule; [Quote Scanning](quote-scanning.md) and
[Matching Pairs](matching-pairs.md) for how delimiter pairs are found.*
