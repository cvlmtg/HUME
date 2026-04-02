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

Byte offsets are used internally by Rust's `str` and by `ropey`, but they are
**never exposed across module boundaries** in HUME. They are an implementation
detail.

## Char offset

A char offset counts **Unicode scalar values** (Rust's `char` type),
regardless of how many bytes each one takes.

```
"café"
 c  a  f  é
 0  1  2  3   ← char offsets
```

`é` is a single `char` at offset 3 — no partial-character hazard. This is
`ropey`'s native addressing unit, and it is what HUME's `Buffer`, `Selection`,
and `SelectionSet` use for all positions.

Char offsets make sense for an editor at the storage layer:
- `insert(at, text)` and `remove(from, to)` can be expressed cleanly.
- `anchor` and `head` in a `Selection` are meaningful without knowing the
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
entire emoji in one step, not stop five times. This is the job of
`grapheme.rs`: it takes a `&Buffer` and returns the next/previous **valid
grapheme boundary** as a char offset.

## Architectural rule

| Unit | Granularity | Role in HUME |
|------|-------------|--------------|
| Byte offset | Raw memory | Internal to `ropey` — never exposed |
| Char offset | Unicode scalar value (`char`) | Storage, selections, `Buffer` API |
| Grapheme cluster | User-perceived character | Cursor movement, motions (`grapheme.rs`) |

The boundary between layers is strict: `grapheme.rs` **consumes** char offsets
and **produces** char offsets that happen to land on grapheme boundaries.
Everything above it works purely in char offsets and never needs to know about
bytes or grapheme internals.
