# Word Motions: Selecting the Whole Word

Word select is one of the four selection patterns described in
[Motions vs Text Objects](motions-vs-text-objects.md). It occupies a middle
ground: it returns a full range like a text object, but it is navigational
like a motion, counting, crossing line boundaries, and stopping at buffer
edges.

When no word exists in that direction, the iteration stops early and the
current selection is preserved, a true no-op. A plain motion always produces
*some* new position; it can never produce a no-op this way.

Everything below assumes a fixed notion of "word", but that notion is
configurable per buffer; see
[Character Classes](charclass.md#word-is-a-policy-not-a-fact-about-the-text).

## Kakoune, Helix, and HUME

Word motions split into two design philosophies, best illustrated by
"change the second word" starting from column 0 in `"hello world"`:

**Kakoune and Helix** (`w` selects the traversed span, anchor at old head):
```
w   → the span you crossed getting there is selected
e   → reanchors at 'w', extends to end of "world"
c   → change "world"       (3 keystrokes)
```
Motions double as selection builders. This composes well but is indirect:
you select what you cross on the way, not the word itself, so acting on the
destination word always needs a second gesture (`e` here, or an inner-word
text object).

**HUME** (`w` selects the whole destination word):
```
w    → "world" selected directly
c    → change "world"       (2 keystrokes)
```
The common case, acting on a word, needs no second gesture. This also
removes the need for `e`/`E`. In Helix, `e` reaches the end of the current
word, complementing `w`, which lands on the start of the next. In HUME, `w`
already selects through the end.

By default `w` also covers the whitespace next to the destination word, so
deleting a word never leaves a double space behind: `one two three`, cursor
on `one`, `w` then delete leaves `one three` with a single space. This is a
configurable layer on top of "select the whole word", shared with the
around-word text object; [Inner vs Around](inner-vs-around.md#the-whitespace-rule-for-mawmaw)
describes which side it takes and how to change it.

## Line crossing

The word boundary model treats end-of-line as its own character class (see
[Character Classes](charclass.md)). A forward word search therefore always
stops *at* the newline character rather than skipping over it. To make `w`
cross line boundaries as users expect, the motion takes a second forward
step when it lands on a non-final newline, treating the newline as
whitespace for that step only.

The effect: `w` lands on the first word of the next line, not on the newline
itself.

## Mid-word behaviour of `b`

Looking backward from the middle of a word, a raw "find previous word start"
call would return the start of the *current* word, not the previous one.
`b` wants to land on the word *before* the cursor, whether the cursor is
between words or inside one.

The fix is a detection step: after finding the current word's boundaries,
check whether the cursor falls inside them. If it does, take one more
backward step to land on the previous word. The visible guarantee: `b`
always selects a *different* word, never the one you are already on.

## Combining characters and word ends

A selection end always sits on the start of a character cluster, and an end
covers the whole cluster that starts there. A word that finishes with `é`,
written as a base `e` followed by a combining accent mark (two separate
Unicode code points), therefore ends on the cluster that starts at the `e`,
and the selection covers both the `e` and the accent. The accent is never
left outside the selection as an orphan, and a subsequent deletion removes
the whole visible character.

The same unit decides which class a character belongs to. Each cluster is
classified by its first code point, so a letter with a combining accent is a
Word character, and a base character followed by a combining mark does not
change class in the middle of the cluster.

## Extend-mode word selection

When word motions run in extend mode (sticky extend, or Ctrl-key), each
press grows or shrinks the selection one word at a time rather than
replacing it. `w` reaches toward the next word, `b` toward the previous one,
and pressing the opposite key walks back the way you came.

The word your selection started on is the anchor: it stays put while
everything else moves. Growing away from it extends the selection to fully
include the next word in that direction. Shrinking back toward it peels
words off the far end, one whole word at a time; a word is never cut down to
a single character. If you keep shrinking past the anchor word itself, the
selection flips to grow in the *other* direction, again keeping full words.
This mirrors how `Ctrl-h`/`Ctrl-j`/`Ctrl-k`/`Ctrl-l` walk a selection back
and forth character by character or line by line.

The whitespace rule carries into extend mode too: growing extends through
the target word's own whitespace, and shrinking back toward the anchor never
cuts into the anchor word's own whitespace.

## Counts fold into one pass

A count prefix (`3w`, `2b`) is folded per selection into the word-select
framework rather than re-running the command `count` times from scratch.
Each selection walks `count` words in one pass. In move mode `3w` jumps
three words and selects the third, and `d` then deletes it. In extend mode
`3w` grows the selection across all three words, so `d` removes them in one
edit.

---

*See also: [Character Classes](charclass.md) for where word boundaries come
from; [Inner vs Around](inner-vs-around.md) for the whitespace rule.*
