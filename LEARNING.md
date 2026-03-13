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

Consider `"hello world"` with cursors at **0** (on `h`) and **6** (on `w`),
inserting `!`:

If we apply edits left-to-right, mutating the buffer each time, the first
insert shifts all subsequent offsets. Cursor 2 was recorded as **6** in the
original buffer, but after inserting at 0 the `w` is now at **7**. Inserting
at the stale offset **6** puts the `!` in the wrong place.

One fix is to process edits right-to-left: an edit at position N never shifts
any offset to its left, so leftward input positions stay valid. But the
*output* cursors computed in earlier (rightward) steps become stale once a
later (leftward) edit shifts text to their right, requiring a retroactive
correction pass.

A cleaner approach is left-to-right with a cumulative delta:

1. `delta = 0`. Insert `!` at `0 + 0 = 0` → `"!hello world"`. New cursor **1**.
   `delta = +1` (one char inserted).
2. Adjust input: `6 + 1 = 7`. Insert `!` at 7 → `"!hello !world"`. New cursor **8**.

Both cursors (**1** and **8**) are already correct in `"!hello !world"`. No
second pass needed. Output positions are automatically correct because each
new cursor is produced *after* the current edit, already in the buffer's
current coordinate space.

In HUME, the `ChangeSetBuilder` eliminates the manual delta entirely. All
positions passed to the builder are in **original-buffer space** — the builder
tracks `old_pos` (consumed from old doc) and `new_pos` (produced in new doc)
internally. After each insert, `new_pos()` gives the cursor's position in the
result buffer directly. See the Changesets section below for details.

---

## Changesets: Describing Edits as Data

### What is a changeset?

A changeset is a **compact, invertible description** of a document
transformation. Instead of mutating the buffer for each selection, we build
one changeset that describes all the edits, then apply it once.

The representation is a sequence of three operations:

| Operation | Meaning |
|-----------|---------|
| `Retain(n)` | Skip `n` chars unchanged |
| `Delete(n)` | Remove `n` chars from the old doc |
| `Insert(s)` | Add `s` to the new doc |

**Example:** Insert `!` at positions 0 and 6 in `"hello world"`:

```
Insert("!"), Retain(6), Insert("!"), Retain(5)
```

This single object describes the entire multi-cursor edit. `apply` consumes
the buffer and executes Delete/Insert operations directly on the rope —
each O(log n). Retain operations are free (the chars are already there).
Total cost: O(k log n) for k non-retain operations.

Because `apply` consumes the buffer, the old buffer no longer exists after
application. This is fine — the inverse changeset captures everything needed
to reconstruct it (see undo/redo below). The caller must call `invert`
before `apply` if it needs the inverse.

### Why not just mutate the buffer directly?

Direct mutation (clone + edit per selection) works, but the edit is lost
after application — there is no record of what changed. A changeset preserves
the edit as data, which enables:

1. **Undo/redo.** Invert the changeset to get an undo operation:
   - `Retain(n)` → `Retain(n)` (no change)
   - `Delete(n)` → `Insert(deleted text)` (re-insert what was removed)
   - `Insert(s)` → `Delete(len(s))` (remove what was added)

   Applying the inverse to the result buffer gives back the original.

2. **Composition.** Two sequential changesets A→B and B→C can be merged into
   a single A→C changeset. This is essential for grouping keystrokes into
   undo steps (typing a word should undo as one operation, not per-character).

3. **Position mapping.** Given a position in the old document, `map_pos`
   computes where it ends up in the new document — accounting for all
   insertions and deletions. An `Assoc` parameter (`Before`/`After`) controls
   which side of an insertion the position sticks to.

   Note that edit operations and undo/redo never call `map_pos`. Edit
   operations use `new_pos()` directly; undo/redo restores selections from the
   stored `Transaction` (see below). `Assoc` is reserved for **external
   positions** — things that exist independently of any specific edit, like LSP
   diagnostic ranges or bookmarks. When a diagnostic sits at offset 5 and text
   is inserted at offset 5, `Assoc::Before` keeps it glued to the left of the
   insertion; `Assoc::After` pushes it past.

### The builder pattern

Edit operations build changesets incrementally using `ChangeSetBuilder`. The
builder tracks two cursors:

- `old_pos` — how far we have consumed in the old document
- `new_pos` — how far we have produced in the new document

This dual tracking replaces the manual delta accumulator. After each
`insert()` call, `new_pos()` tells you exactly where a cursor should land
in the result buffer.

```text
Builder state for insert_char('x') with cursor at offset 3 in "hello":

  b.retain(3)     →  old_pos=3, new_pos=3    (skip "hel")
  b.insert("x")   →  old_pos=3, new_pos=4    (insert 'x')
  b.retain_rest()  →  old_pos=5, new_pos=6    (keep "lo")

  Result: Retain(3), Insert("x"), Retain(2)
  Cursor: b.new_pos() at time of insert = 4  →  "helx|lo"
```

All positions are in **original-buffer space** — no delta tracking, no
intermediate buffer clones. The builder handles the coordinate translation
internally.

### Transactions: changesets with cursor state

A `ChangeSet` describes only the text change. A `Transaction` pairs it with the
**post-apply** `SelectionSet` — where the cursors land *after* the changeset is
applied. This invariant holds for every Transaction, forward or inverse.

At edit time you build **two** Transactions from the same changeset:

```text
// 1. Capture the inverse BEFORE apply consumes the buffer.
let inv_cs  = cs.invert(&old_buf);
let new_buf = cs.apply(old_buf);          // old_buf is consumed here

// 2. Build both Transactions from the same cs/inv_cs.
let forward = Transaction::new(cs,     post_edit_sels);  // for redo
let inverse = Transaction::new(inv_cs, pre_edit_sels);   // push to undo stack
```

The inverse Transaction's `selection` is `pre_edit_sels` — the cursors from
*before* the edit — because that is where applying the inverse will leave the
cursors. The "always post-apply" invariant holds: after running the inverse,
the cursors are at `pre_edit_sels`.

**Timing matters.** `invert(&old_buf)` must be called before `apply(old_buf)`.
`apply` consumes the buffer (the old rope is gone). `invert` needs the original
content to reconstruct the `Insert` operations for deleted text — it captures
the deleted chars from the live rope at inversion time.

The history manager stores inverse Transactions. Applying one restores both the
text and the cursor positions in a single step.

### Implementation

- `src/changeset.rs` — `Operation`, `ChangeSet`, `ChangeSetBuilder`, `Assoc`
- `src/transaction.rs` — `Transaction` (pairs ChangeSet with SelectionSet)
- `src/edit.rs` — edit operations build changesets via the builder
