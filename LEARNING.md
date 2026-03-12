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

### Multi-selection edit ordering

A `SelectionSet` can contain multiple selections simultaneously (multicursor).
When an edit touches multiple positions, **the order of application matters**.

Consider inserting `!` with two cursors at offsets 3 and 7 in `"foo bar"`:

```
Before:   f o o   b a r
offsets:  0 1 2 3 4 5 6
cursors:        ^       ^
                3       7
```

**Naïve left-to-right (broken):**

1. Insert `!` at 3 → buffer becomes `"foo! bar"` (8 chars).
   The character that used to be at offset 7 (`r`) is now at offset **8**.
2. Insert `!` at 7 → we insert at the *stale* offset, hitting `a` instead of `r`.
   **Wrong result: `"foo! ba!r"`**

The input positions go stale as soon as the first edit shifts the buffer.

**Right-to-left (solves the input problem, creates an output problem):**

Apply edits from the rightmost selection to the leftmost. An edit at position N
never shifts any offset to its left, so the next (leftward) input position is
still valid.

Consider `"hello world"` with cursors at **0** (on `h`) and **6** (on `w`),
inserting `!`:

1. Insert `!` at 6 → `"hello !world"`. Store new cursor at **7**. ✓
2. Insert `!` at 0 → `"!hello !world"`. New cursor at **1**. ✓

The input positions were fine — but step 2 shifted everything right by 1, so
the cursor stored in step 1 (**7**) is now wrong. It should be **8**. A
retroactive correction pass is required after the loop, which adds complexity.

**Left-to-right with cumulative delta (the actual algorithm):**

Process selections in ascending order. Before each edit, adjust the selection's
position by `delta` — the net char-count change from all *previous* edits.
The resulting new cursors are already correct in the final buffer; no retroactive
pass is needed.

Same example — `"hello world"`, cursors at **0** and **6**, inserting `!`:

1. `delta = 0`. Insert `!` at `0 + 0 = 0` → `"!hello world"`. New cursor **1**.
   `delta = +1` (one char inserted).
2. Adjust input: `6 + 1 = 7`. Insert `!` at 7 → `"!hello !world"`. New cursor **8**.

Both cursors (**1** and **8**) are already correct in `"!hello !world"`. No
second pass needed.

**Why output positions are automatically correct:** each new cursor is produced
*after* the current edit, so it's already expressed in the buffer's current
coordinate space. The only positions that need adjusting are the *inputs* —
the original selections recorded before any edits ran. The `delta` handles
exactly that.

This is what `apply_to_each` in `src/edit.rs` implements: left-to-right
iteration with a running `delta` that shifts each input selection before the
closure is called.
