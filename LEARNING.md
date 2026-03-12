# HUME — Learning Notes

Concepts that come up while building HUME, explained in enough depth to be
useful later. Sections are added as topics arise during development.

---

## Unicode Position Model: Bytes, Chars, and Grapheme Clusters

Understanding this hierarchy is essential for HUME's architecture. Three
different units can describe a "position" in text, and choosing the wrong one
at the wrong layer causes subtle, hard-to-reproduce bugs.

### Byte offset

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

### Char offset

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

### Grapheme cluster

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
`grapheme.rs`: it takes a `RopeSlice` and returns the next/previous **valid
grapheme boundary** as a char offset.

### Architectural rule

| Unit | Granularity | Role in HUME |
|------|-------------|--------------|
| Byte offset | Raw memory | Internal to `ropey` — never exposed |
| Char offset | Unicode scalar value (`char`) | Storage, selections, `Buffer` API |
| Grapheme cluster | User-perceived character | Cursor movement, motions (`grapheme.rs`) |

The boundary between layers is strict: `grapheme.rs` **consumes** char offsets
and **produces** char offsets that happen to land on grapheme boundaries.
Everything above it works purely in char offsets and never needs to know about
bytes or grapheme internals.

---

## Edit Operations: Acting on Selections

### The select-then-act model

In HUME, edit operations never act on a bare cursor position. They act on a
`SelectionSet` — which is always a `Vec<Selection>`. Each `Selection` is either:

- **Collapsed** (`anchor == head`): a plain cursor with no selected text.
- **Non-collapsed** (`anchor != head`): a region of selected text.

An operation like "insert character `x`" means:

- For a **collapsed selection**: insert `x` at the cursor position.
- For a **non-collapsed selection**: replace the selected region with `x` (delete
  the selection, then insert).

This is the same rule in both cases — "replace the selected region with the
input, where an empty selection replaces nothing". Single-cursor editing,
visual-mode editing, and multicursor editing all fall out of the same loop.

### The right-to-left rule

A `SelectionSet` can contain multiple selections simultaneously (multicursor).
When an edit touches multiple positions, **the order of application matters**.

Consider inserting `!` with two cursors at offsets 3 and 7 in `"foo bar"`:

```
Before:   f o o   b a r
offsets:  0 1 2 3 4 5 6
cursors:        ^       ^
                3       7
```

If we apply left-to-right (offset 3 first):

1. Insert `!` at 3 → buffer becomes `"foo! bar"` (8 chars).
   The character that used to be at offset 7 (`r`) is now at offset **8**.
2. Insert `!` at 7 → we insert at the old offset, hitting `a` instead of `r`.
   **Wrong result: `"foo! ba!r"`**

If we apply right-to-left (offset 7 first):

1. Insert `!` at 7 → buffer becomes `"foo bar!"` (8 chars).
   Offsets 0–6 are **unchanged** — nothing to the left shifted.
2. Insert `!` at 3 → buffer becomes `"foo! bar!"`.
   **Correct result: `"foo! bar!"`**

The rule: **sort selections by position descending and apply edits from the
rightmost position to the leftmost**. An edit at position N never affects
offsets less than N, so all earlier selections remain valid.

After all edits, each selection's offset must be adjusted to account for the
characters inserted or removed before it — but because we went right-to-left,
each adjustment is independent and can be calculated from the edit delta alone.
