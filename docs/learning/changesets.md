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

This single object describes the entire multi-cursor edit. Applying it clones
the underlying rope (O(1) — arc-based structural sharing) and executes each
Delete/Insert on the clone — each O(log n). Retain operations are free.
Total cost: O(k log n) for k non-retain operations.

The original buffer remains intact after application. The inverse must be
computed from the original before applying the forward changeset, because
inversion reads the deleted text from the original at that point.

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

3. **Position mapping.** Given a position in the old document, the changeset
   can compute where it ends up in the new document — accounting for all
   insertions and deletions. An association parameter (before/after) controls
   which side of an insertion the position sticks to.

   Edit operations and undo/redo never use position mapping. Edits compute
   result positions directly during construction; undo/redo restores selections
   from the stored transaction (see below). Position mapping is reserved for
   **external positions** — things that exist independently of any specific
   edit, like LSP diagnostic ranges or bookmarks. When a diagnostic sits at
   offset 5 and text is inserted at offset 5, before-sticky keeps it glued to
   the left of the insertion; after-sticky pushes it past.

## The builder pattern

Edit operations build changesets incrementally using a builder. The builder
tracks two cursors:

- consumed position — how far we have read in the old document
- produced position — how far we have written in the new document

This dual tracking replaces manual delta accumulation. After each insert, the
produced position tells you exactly where a cursor should land in the result.

```text
Building insert_char('x') with cursor at offset 3 in "hello":

  retain(3)     →  consumed=3, produced=3    (skip "hel")
  insert("x")   →  consumed=3, produced=4    (insert 'x')
  retain_rest() →  consumed=5, produced=6    (keep "lo")

  Result: Retain(3), Insert("x"), Retain(2)
  Cursor position at insert time = 4  →  "helx|lo"
```

All positions are in **original-buffer space** — no delta tracking, no
intermediate buffer clones. The builder handles coordinate translation
internally.

## Transactions: changesets with cursor state

A changeset describes only the text change. A *transaction* pairs it with the
cursor positions that should be in effect **after** the changeset is applied.
This invariant holds for every transaction, forward or inverse — the cursor
state stored is always where you land after running the transaction, never
before.

The invariant matters because it makes forward and inverse perfectly symmetric.
To undo: apply the inverse transaction. The cursors that come with it are where
you were before the edit. To redo: apply the forward transaction. The cursors
that come with it are where the edit originally left you. Undo is just "apply
the inverse" — no special cursor logic needed.

At edit time you build two transactions from the same changeset:

- The **inverse** (for undo) pairs the inverse changeset with the *pre-edit*
  cursor positions. Applying it returns both the text and the cursors to where
  they were before the edit.
- The **forward** (for redo) pairs the original changeset with the *post-edit*
  cursor positions.

**Timing matters.** The inverse must be computed *before* the forward edit is
applied, because inverting a changeset reads the deleted text from the original
buffer to reconstruct what was there. Once the buffer is overwritten with the
new content, the original deleted text is gone.

The history manager stores both transactions. Applying the inverse restores
both the text and the cursor positions in a single step.

## The undo tree

HUME's undo is a **tree**, not a stack. Every edit creates a new branch point;
undoing and then making a different edit creates a new branch, and the old
branch is preserved. You can navigate back to any past state.

The tree is stored as a flat list of nodes where each node holds integer
indices pointing at its parent and children — like a linked list but with
plain numbers instead of pointers. Lookups are immediate array accesses; the
tree structure doesn't cause any complexity for the memory management system.
The trade-off is that nodes are never individually freed — the whole tree is
dropped only when the buffer closes. For a tree that only ever grows, this
is a fine trade.

The history manager enforces that the inverse changeset is always computed
before the edit is applied to the buffer, keeping the timing invariant intact.
