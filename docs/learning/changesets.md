# Changesets: Describing Edits as Data

## What is a changeset?

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

This single object describes the entire multi-cursor edit. `apply` takes the
buffer by reference, clones the underlying rope (O(1) — Ropey uses Arc-based
structural sharing), and executes Delete/Insert operations on the clone —
each O(log n). Retain operations are free (the chars are already there).
Total cost: O(k log n) for k non-retain operations.

The original buffer remains intact after `apply`. The caller must still call
`invert` before `apply` if it needs the inverse, because `invert` reads
deleted text from the original rope at inversion time.

## Why not just mutate the buffer directly?

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

## The builder pattern

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

## Transactions: changesets with cursor state

A `ChangeSet` describes only the text change. A `Transaction` pairs it with the
**post-apply** `SelectionSet` — where the cursors land *after* the changeset is
applied. This invariant holds for every Transaction, forward or inverse.

At edit time you build **two** Transactions from the same changeset:

```text
// 1. Capture the inverse BEFORE apply — both read from the same old_buf.
let inv_cs  = cs.invert(&old_buf);
let new_buf = cs.apply(&old_buf);         // old_buf still valid after this

// 2. Build both Transactions from the same cs/inv_cs.
let forward = Transaction::new(cs,     post_edit_sels);  // for redo
let inverse = Transaction::new(inv_cs, pre_edit_sels);   // push to undo stack
```

The inverse Transaction's `selection` is `pre_edit_sels` — the cursors from
*before* the edit — because that is where applying the inverse will leave the
cursors. The "always post-apply" invariant holds: after running the inverse,
the cursors are at `pre_edit_sels`.

**Timing matters.** `invert(&old_buf)` must be called before discarding
`old_buf`. `invert` reads deleted text from the original rope to reconstruct
the `Insert` operations — it captures those chars at inversion time. In
practice `Buffer::apply_edit` enforces this: it calls `cs.invert(&self.buf)`
while `self.buf` still holds the pre-edit content, then overwrites it.

The history manager stores both Transactions. Applying the inverse restores
both the text and the cursor positions in a single step (undo); applying the
forward Transaction redoes the edit.

## Implementation

- `editor/src/core/changeset/` — `Operation`, `ChangeSet`, `ChangeSetBuilder`, `Assoc`
- `editor/src/core/transaction.rs` — `Transaction` (pairs ChangeSet with SelectionSet)
- `editor/src/ops/edit/` — edit operations build changesets via the builder
- `src/core/history.rs` — arena-based undo tree; stores forward/inverse Transaction pairs per revision

  An **arena** is a `Vec` that owns all the nodes of a tree or graph. Instead
  of linking nodes with pointers or `Rc<RefCell<...>>`, each node stores plain
  integer indices into the `Vec`. Lookups are O(1) array accesses; there are no
  reference cycles for the borrow checker to worry about; and the allocator sees
  one contiguous allocation instead of many small heap objects. The trade-off is
  that nodes are never individually freed — the whole arena is dropped at once.
  For an undo tree that only grows, this is fine.
- `editor/src/editor/buffer.rs` — orchestrates Buffer + SelectionSet + History; enforces the invert-before-apply timing invariant in `apply_edit`
