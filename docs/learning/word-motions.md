# Word Motions: Selecting the Whole Word

Everything below assumes a fixed notion of "word" for clarity, but that
notion is itself configurable per buffer — see [CharClass](charclass.md#word-is-a-policy-not-a-fact-about-the-text).

## A third framework

Word select is one of several distinct patterns for creating selections from
cursor movement — this doc covers the three most directly relevant to it;
see [Motions vs Text Objects](motions-vs-text-objects.md) for a fourth,
structural navigation over whole object spans — tree-sitter objects like
functions and classes, and the paragraph motions:

| Pattern | Returns | Anchor (in Move mode) | Typical use |
|---|---|---|---|
| Motion | a new cursor position | depends on Move vs Extend mode | `h/j/k/l`, goto-line |
| Text object | an optional selected range | the range's start | `iw`, `i(`, `i"` |
| Word select | an optional selected range | the selected span's start | `w/b/W/B` |

(In extend mode both text objects and word selects anchor differently — they
grow an existing selection, preserving its direction; see the extend sections
below.)

Word select occupies a middle ground: it returns a full range like a text
object, but it is navigational like a motion — counting, crossing line
boundaries, and stopping at buffer edges.

When no word exists in that direction, the iteration stops early and the
current selection is preserved — a true no-op. A plain motion always produces
*some* new position; it can never produce a no-op this way.

## Kakoune, Helix, and HUME

Word motions split into two design philosophies, best illustrated by
"change the second word" starting from column 0 in `"hello world"`:

**Kakoune and Helix** (`w` selects the traversed span, anchor at old head):
```
w   → the span you crossed getting there is selected
e   → reanchors at 'w', extends to end of "world"
c   → change "world"       (3 keystrokes)
```
Motions double as selection builders. Composable, but indirect — you select
what you cross on the way, not the word itself, so acting on the destination
word always needs a second gesture (`e` here, or an inner-word text object).

**HUME** (`w` selects the whole destination word):
```
w    → "world" selected directly
c    → change "world"       (2 keystrokes)
```
The common case — act on a word — requires no second gesture. This also
eliminates `e`/`E`: in Helix/Vim, `e` reaches the end of the current word
(complementing `w` which lands on the start of the next). In HUME, `w`
already selects through the end, making `e` redundant.

By default `w` also covers the whitespace *before* the destination word, so
deleting a word never leaves a double space behind: `one two three`, cursor
on `one`, `w` then delete leaves `one three` with a single space. The first
word on a line is an exception — its leading whitespace is indentation, not
inter-word spacing, so it's never touched. That word — and any word with no
whitespace directly before it, say right after punctuation — takes its
*trailing* whitespace instead. This is a separate, toggleable layer on top of the
"select the whole word" behavior above, not a different word-select model —
an option restores the bare-word span shown here.

## Line crossing

The word boundary model treats end-of-line as its own character class (see
[CharClass](charclass.md)). This means a forward word search always stops *at*
the newline character rather than skipping over it. To make `w` cross line
boundaries as users expect, the implementation takes a second forward step when
it lands on a non-final newline — treating the newline as whitespace for that
step only.

The effect: `w` lands on the first word of the next line, not on the newline
itself.

## Mid-word behaviour of `b`

Looking backward from the middle of a word, a raw "find previous word start"
call would return the start of the *current* word — not the previous one. `b`
wants to land on the word *before* the cursor, whether the cursor is between
words or inside one.

The fix is a detection step: after finding the current word's boundaries, check
whether the cursor falls inside them. If it does, take one more backward step
to land on the previous word. The visible guarantee: `b` always selects a
*different* word, never the one you're already on.

## Combining characters and word end positions

A word's end position is stored as the position of the **last character** of
the final grapheme cluster, not the start of it. For most characters this is
the same thing. For a combining sequence like `café` where `é` is encoded as
a base `e` followed by a combining accent mark (two separate Unicode code
points), stopping at the base `e` would leave the accent mark outside the
selection — an orphaned combining mark that no longer has a base to attach to.

Selecting through the accent mark ensures the whole visible character is
covered, and any subsequent deletion removes the whole grapheme.

## Extend-mode word selection

When word motions run in extend mode (sticky extend, or Ctrl+key), each press
grows or shrinks the selection one word at a time, rather than replacing it
entirely. `w` reaches toward the next word; `b` toward the previous one — and
pressing the opposite key walks back the way you came instead of only ever
growing.

The word your selection started on is the anchor for this: it's what stays
put while everything else moves. Growing away from it extends the selection
to fully include the next word in that direction. Shrinking back toward it
peels words off the far end, one whole word at a time — a word is never cut
down to a single character. If you keep shrinking past the anchor word
itself, the selection flips to grow in the *other* direction instead, again
always keeping full words. This mirrors how `Ctrl+h`/`Ctrl+j`/`Ctrl+k`/`Ctrl+l`
already let you walk a selection back and forth character-by-character or
line-by-line — `w`/`b` do the same thing one word at a time.

The whitespace-inclusion behavior described above carries into extend mode
too: growing extends through the target word's own whitespace bookend, and
shrinking back toward the anchor never cuts into the anchor word's own
whitespace either.

## Counts fold into one pass

A count prefix (`3w`, `2b`) is folded per-selection into the word-select
framework rather than re-running the command `count` times from scratch. Each
selection walks `count` words in one pass: in Move mode `3w` jumps three
words and selects the third — `d` then deletes it. In extend mode `3w`
instead grows the selection across all three words, so `d` removes them in
one edit.
