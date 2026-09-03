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

**Example:** Insert `!` at positions 0 and 6 in `"hello world\n"` (the buffer
includes its structural trailing newline, so it is twelve characters):

```
Insert("!"), Retain(6), Insert("!"), Retain(6)
```

This single object describes the entire multi-cursor edit. Applying it clones
the underlying rope — O(1), because the rope uses arc-based structural sharing
— and then executes each Delete/Insert on the clone. Each edit is O(log n) and
Retain operations are free. Total cost: O(k log n) for k non-retain operations.

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
   Composition walks both changesets in lockstep from left to right; when an
   insertion in one lines up with a deletion in the other, the two cancel
   rather than producing a redundant delete-then-insert pair.

   Composition is one half of keystroke grouping; the other half is the *edit
   group* itself, the live accumulator the editor maintains between an
   explicit "begin" and "commit" pair. While a group is open, every edit a
   command makes composes into the group's placeholder changeset; nothing is
   recorded on the undo tree yet. Only when the group commits does the
   accumulated changeset become a single revision.

3. **Position mapping.** Given a position in the old document, the changeset
   can compute where it ends up in the new document — accounting for all
   insertions and deletions. An association parameter (before/after) controls
   which side of an insertion the position sticks to.

Almost every edit operation avoids position mapping: it computes result
positions directly during construction, and undo/redo restore selections from
the stored transaction (see below). Indent/unindent (shifting a line's
leading whitespace by a level) is the one exception. Rewriting a line's
indent is a replace — old whitespace out, new whitespace in — and a selection
that happens to sit exactly at the line's start is genuinely ambiguous:
should it stay pinned to the start of the line, or land past the freshly
written indent? Rather than hand-deriving an answer per selection, indent
lets position mapping resolve it: it writes the new indent into the
changeset *before* removing the old one, so a position at the line start
meets the insertion first and the answer becomes a plain choice of
association — the start of a whole-line selection stays put (sticks before),
every other position sitting there rides past the new indent (sticks after).
Position mapping otherwise serves everything *else* that holds a position not
tied to the edit being made. One consumer is
**non-acting-pane cursor propagation**: when one pane edits a buffer that
other panes also have open, the other panes' selections must ride the
changeset to stay meaningful in the new text. Others are external positions
the editor stores between edits — LSP diagnostic ranges and decoration
anchors ride every changeset the same way, so a diagnostic keeps pointing at
the right text as the buffer changes around it.

## The builder pattern

Edit operations build changesets incrementally using a builder. The builder
tracks two cursors:

- consumed position — how far we have read in the old document
- produced position — how far we have written in the new document

This dual tracking replaces manual delta accumulation. After each insert, the
produced position tells you exactly where a cursor should land in the result.

```text
Building insert_char('x') with cursor at offset 3 in "hello\n" (six chars,
including the structural newline):

  retain(3)     →  consumed=3, produced=3    (skip "hel")
  insert("x")   →  consumed=3, produced=4    (insert 'x')
  retain_rest() →  consumed=6, produced=7    (keep "lo\n")

  Result: Retain(3), Insert("x"), Retain(3)
  Cursor position at insert time = 4  →  "helx|lo\n"
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

Every edit path computes the inverse changeset before replacing the buffer
text; the history manager stores the pair as one revision. Applying the
inverse restores both the text and the cursor positions in a single step.

Reloads share the same algebra. `:e!` does not throw away the buffer and start
over; it derives a (forward, inverse) changeset pair from a line-level diff
against the disk content and records it as a normal revision. Undo after a
reload brings the pre-reload text back with its full undo tree intact beneath
— the reload is just another edit at another branch tip. When the disk content
matches the buffer exactly, the forward changeset is the identity and no
revision is recorded at all.

One final detail on position mapping: both association modes are exercised in
practice. Cursors ride the *after* side of insertions — they sit on the far
side of newly inserted text, never suspended at the insertion site. Positions
that must stay glued to the text before them — a range's end, an anchor
pinned to what was already there — ask for *before* association instead.
Mapping a whole range uses both at once: its start maps after, its end maps
before, so an insertion at either edge lands outside the range rather than
silently growing it.

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
