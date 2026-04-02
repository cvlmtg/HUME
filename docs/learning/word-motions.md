# Word Motions: Selecting the Whole Word

## A third framework

Three distinct patterns exist for creating selections from cursor movement:

| Framework | Inner fn signature | Anchor | Typical use |
|---|---|---|---|
| `apply_motion` | `fn(&Buffer, usize) -> usize` | Via `MotionMode` | `h/j/k/l`, paragraph, goto-line |
| `apply_text_object` | `fn(&Buffer, usize) -> Option<(usize, usize)>` | Always `start` | `iw`, `i(`, `i"` |
| `apply_word_select` | `fn(&Buffer, usize) -> Option<(usize, usize)>` | Always `word_start` | `w/b/W/B` |

`apply_word_select` occupies a middle ground: its inner function returns a full
range like a text object, but it is navigational like a motion — counting,
crossing line boundaries, and stopping at buffer edges.

```rust
fn apply_word_select(
    buf: &Buffer,
    sels: SelectionSet,
    count: usize,
    motion: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet
```

When the inner function returns `None` (no word in that direction), the
iteration stops early and the current selection is preserved — a true no-op.
Compare `apply_motion`, where the inner function always returns a position.

## Kakoune, Helix, and HUME

Word motions reflect three distinct design philosophies, best illustrated by
"change the second word" starting from column 0 in `"hello world"`:

**Kakoune** (`w` selects the traversed span, anchor at old head):
```
w   → "hello w" selected (traversed span)
e   → reanchors at 'w', extends to end of "world"
c   → change "world"       (3 keystrokes)
```
Motions double as selection builders. Composable, but indirect — you select
what you cross on the way, not the word itself.

**Helix** (`w` = Move, pure navigation):
```
w    → cursor jumps to 'w', single-char selection
iw   → text object selects "world"
c    → change "world"       (3 keystrokes)
```
Predictable — `w` always means "go there". But acting on a word always needs
a second gesture (`iw`).

**HUME** (`w` selects the whole destination word):
```
w    → "world" selected directly
c    → change "world"       (2 keystrokes)
```
The common case — act on a word — requires no second gesture. This also
eliminates `e`/`E`: in Helix/Vim, `e` reaches the end of the current word
(complementing `w` which lands on the start of the next). In HUME, `w`
already selects through the end, making `e` redundant.

## Line crossing: the double-step in `select_next_word`

`Eol` is its own `CharClass` (see [CharClass](charclass.md)), so `next_word_start`
always stops at a `\n`. A single call from the end of a line lands on the
newline itself, not the first word of the next line. `select_next_word` adds
a second step when this happens:

```rust
let mut word_start = next_word_start(buf, pos, is_boundary);

// If we landed on a non-trailing '\n', cross the line.
if word_start < len.saturating_sub(1) {
    if classify_char(buf.char_at(word_start).expect("word_start < len")) == CharClass::Eol {
        word_start = next_word_start(buf, word_start, is_boundary);
    }
}
```

The second call treats the `\n` as whitespace and advances to the first word
of the next line — making `w` cross line boundaries as users expect.

## Mid-word detection: the double-step in `select_prev_word`

`prev_word_start` finds the start of the word *at or containing* a position.
When the cursor is mid-word, it returns the current word's start — not the
previous word. `select_prev_word` detects this and takes an extra backward step:

```rust
let word_start = prev_word_start(buf, pos, is_boundary);
let word_end   = find_word_end_from(buf, word_start, is_boundary);

// pos is inside the current word — one more step back to reach the previous word.
if pos >= word_start && pos <= word_end {
    if word_start == 0 { return None; }  // already at the first word
    let prev_start = prev_word_start(buf, word_start, is_boundary);
    let prev_end   = find_word_end_from(buf, prev_start, is_boundary);
    return Some((prev_start, prev_end));
}
```

This means `b` always selects a *different* word — the one before the cursor —
whether the cursor is between words or inside one.

## `find_word_end_from` and multi-codepoint graphemes

`find_word_end_from` returns the position of the **last codepoint** of the
final grapheme cluster in the word. For single-codepoint graphemes (the common
case) this is the grapheme's only position. For a combining sequence like
`café` where `é` = `U+0065 + U+0301`:

```
c  a  f  e  ◌́
0  1  2  3  4
              ↑  find_word_end_from returns 4, not 3
```

Returning 3 (the base `e`) would leave `U+0301` outside the selection — an
orphaned combining mark. Returning 4 ensures `Selection::new(word_start, 4)`
covers the complete grapheme.

This interacts with `Selection::end_inclusive(buf)` in edit operations. Edit
code calls `end_inclusive` (which computes `next_grapheme_boundary(buf, end) - 1`)
instead of `end()` when building deletion ranges. For a selection built with
`find_word_end_from` (which already stored the last codepoint as `head`), this
is a no-op. For other selections (e.g. a text object that stopped at a
grapheme-start), `end_inclusive` extends to the full grapheme. Both paths
handle combining marks correctly.

## Two sets of word commands

Word motions appear in two flavours with different semantics:

| Command | Framework | Semantics |
|---------|-----------|-----------|
| `cmd_select_next_word` | `apply_word_select` | Fresh selection of the whole word |
| `cmd_extend_select_next_word` | `apply_word_select_extend_forward` | Union of current selection and next word |

Both sets are hand-written functions — neither can be generated by the
`motion_cmd!` macro, which only wraps `apply_motion` (position-to-position
functions). The select variants call `apply_word_select`; the extend variants
call `apply_word_select_extend_forward` or `apply_word_select_extend_backward`,
which union the current selection with the newly selected word rather than
replacing it.
