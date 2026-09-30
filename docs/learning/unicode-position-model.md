# Unicode Position Model: Bytes, Chars, and Grapheme Clusters

Understanding this hierarchy is essential for HUME's architecture. Three
different units can describe a "position" in text, and choosing the wrong one
at the wrong layer causes subtle, hard-to-reproduce bugs.

## Byte offset

A byte offset is a raw index into memory. In UTF-8 (Rust's string encoding),
characters are **variable-width**: 1 to 4 bytes each.

```
"café"
 c  a  f  é
 1  1  1  2   ← bytes per character
```

| Char | Bytes   | Byte offsets |
|------|---------|-------------|
| `c`  | `63`    | 0 |
| `a`  | `61`    | 1 |
| `f`  | `66`    | 2 |
| `é`  | `C3 A9` | 3, 4 |

`é` occupies bytes 3 **and** 4. Byte offset 4 points into the **middle** of a
character — it is not a valid character boundary. This is why `s[3..4]` on
`"café"` panics in Rust: slicing through a multi-byte character is undefined.

Byte offsets are used internally by Rust's `str` and by the rope library HUME
stores buffer text in, but they are **never used for buffer positions** in
HUME. They surface only at narrow interoperability seams, where an external
system speaks other units: regular-expression matchers and tree-sitter speak
bytes, and language servers speak line-plus-column positions whose column
counts UTF-8 bytes or UTF-16 code units — a fourth unit that exists only at
that wire boundary and is converted to char offsets on arrival. Outside those
seams, byte offsets are an implementation detail.

## Char offset

A char offset counts **Unicode scalar values** (Rust's `char` type),
regardless of how many bytes each one takes.

```
"café"
 c  a  f  é
 0  1  2  3   ← char offsets
```

`é` is a single `char` at offset 3 — no partial-character hazard. This is the
rope library's native addressing unit, and it is what HUME's buffer,
selections, and selection sets use for all positions.

Char offsets make sense for an editor at the storage layer:
- `insert(at, text)` and `remove(from, to)` can be expressed cleanly.
- The anchor and head of a selection are meaningful without knowing the
  encoding of any particular character.

## Grapheme cluster

A char offset solves the byte problem, but there is a level above it:
**grapheme clusters** — what a user perceives as a single indivisible
character, which may be composed of multiple Unicode scalar values.

```
"é"  can be:
  U+00E9             → 1 char  (precomposed NFC form)
  U+0065 + U+0301    → 2 chars (base 'e' + combining acute accent)

"👨‍👩‍👧"              → 1 visible character, but 5 chars
                       (joined with zero-width joiners U+200D)
```

Pressing the right-arrow key on `"👨‍👩‍👧"` should advance the cursor past the
entire emoji in one step, not stop five times. This is the job of the
grapheme layer: given the buffer, it returns the next/previous **valid
grapheme boundary** as a char offset.

## Architectural rule

| Unit | Granularity | Role in HUME |
|------|-------------|--------------|
| Byte offset | Raw memory | Internal to the text storage library — never exposed |
| Char offset | Unicode scalar value (`char`) | Storage, selection positions, buffer API |
| Grapheme cluster | User-perceived character | Cursor movement, motions, and what a selection covers |

The boundary between layers is strict: the grapheme layer **consumes** char
offsets and **produces** char offsets that happen to land on grapheme
boundaries. Everything above it works purely in char offsets and never needs
to know about bytes or grapheme internals.

## Selections address clusters

A selection's anchor and head are char offsets, but they are always the
*start* of a grapheme cluster. A cursor on `é` (`e` + combining accent) sits at
the offset of the `e`; it covers both chars, because the cluster is the
smallest thing a cursor can be on.

```
"café" with a decomposed é
 c  a  f  e  ◌́
 0  1  2  3  4
              cursor on é: anchor = head = 3, covers offsets 3 and 4
```

So a selection has two ways to describe where it ends, and they answer
different questions:

- the **head** is where the cursor is drawn and where the next motion starts;
- the **extent** is the chars it covers, which runs to the end of the last
  cluster.

A command that deletes, yanks, or replaces a selection needs the extent. If it
used the head as the end of the range it would delete the `e` and leave the
accent behind on whatever comes next. Keeping the head as a raw offset that
callers must remember to extend is a convention, and conventions get forgotten.
HUME instead hides the raw head-of-the-far-end from other crates and hands
out the extent through methods that always cover the whole last cluster, and
a char range (say, a text object's result) becomes a selection through one
conversion that snaps both ends onto cluster starts.

An edit is the one place a position can land inside a cluster: it produces
positions for text that does not exist yet, and the text it leaves behind can
re-form clusters (deleting the base letter leaves its accent joining the
previous character; deleting one half of a flag re-pairs the rest). The result
of every edit is snapped onto the clusters of the new text.

The grapheme layer also answers a related family of questions for vertical and
horizontal layout: the display column of a position once tab stops are
expanded, the position that lands on a given display column, the number of
graphemes in a range. The same abstraction that gives you "next grapheme
boundary" gives you tab-aware column arithmetic for free.
