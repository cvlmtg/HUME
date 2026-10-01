# Changesets: Describing Edits as Data

## What is a changeset?

A changeset is a **compact, invertible description** of a document
transformation. Instead of mutating the buffer for each selection, we build
one changeset that describes all the edits, then apply it once.

The representation is a sequence of three operations:

| Operation | Meaning |
|-----------|---------|
| `keep(n)` | Skip `n` chars unchanged |
| `delete(n)` | Remove `n` chars from the old doc |
| `insert(s)` | Add `s` to the new doc |

**Example:** Insert `!` at positions 0 and 6 in `"hello world\n"` (the buffer
includes its structural trailing newline, so it is twelve characters):

```
insert("!"), keep(6), insert("!"), keep(6)
```

This single object describes the entire multi-cursor edit. Applying it clones
the underlying rope — O(1), because the rope uses arc-based structural sharing
— and then executes each delete and insert on the clone. Each edit is O(log n) and
keep operations are free. Total cost: O(k log n) for k non-keep operations.

The original buffer remains intact after application. The inverse must be
computed from the original before applying the forward changeset, because
inversion reads the deleted text from the original at that point.

## Why not just mutate the buffer directly?

Direct mutation (clone + edit per selection) works, but the edit is lost
after application — there is no record of what changed. A changeset preserves
the edit as data, which enables:

1. **Undo/redo.** Invert the changeset to get an undo operation:
   - `keep(n)` → `keep(n)` (no change)
   - `delete(n)` → `insert(deleted text)` (re-insert what was removed)
   - `insert(s)` → `delete(len(s))` (remove what was added)

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

Position mapping is how selections follow an edit they did not make. When
one pane edits a buffer that other panes also show, the other panes'
selections ride the changeset to stay meaningful in the new text. Each end of
a selection maps past text inserted at it, lands on the character cluster of
the new text that holds it, and selections the change folds together merge. A
sticky column survives only when the change left the head's line alone.

The same mapping serves every position the editor stores between edits, not
only selections:

- the selections of every pane that has ever shown the buffer, whether or not
  it shows it now;
- jump lists, marks, and positions a script is tracking;
- the snapshots of open prompts and the open completion session;
- diagnostics and decoration anchors, so a diagnostic keeps pointing at the
  right text as the buffer changes around it;
- the clusters typed by the most recent insert session.

Some stored values are only valid for the text they were computed against,
such as the cache of search matches or the anchor of a completion menu. Each
carries the version of its text, and reading it against a text of another
version gives nothing. A value that is not carried through a change therefore
reads as absent after the change, and no code can use it by accident.

A command's own result does not need mapping. An edit says where its
selections land in terms of the text it produces (a cursor after the inserted
text, the run an insertion became), or it carries an old selection through the
edit and picks a side of any insertion at its ends. Indent shows the second
case. Rewriting a line's indent is a replace: old whitespace out, new
whitespace in. A selection that sits at the line's start is ambiguous, because
it could stay pinned to the line start or land past the new indent. The edit
writes the new indent before removing the old one, so a position at the line
start meets the insertion first and the answer becomes a plain choice of
side. The start of a whole-line selection stays put (sticks before) and every
other position sitting there rides past the new indent (sticks after).

## The builder pattern

A changeset is put together front to back by a builder with two cursors:

- consumed position: how far it has read in the old document
- produced position: how far it has written in the new document

This dual tracking replaces manual delta accumulation. After each insert, the
produced position tells you exactly where a cursor should land in the result.

```text
Building an insert of "x" with the cursor at offset 3 in "hello\n" (six
chars, including the structural newline):

  keep(3)       →  consumed=3, produced=3    (skip "hel")
  insert("x")   →  consumed=3, produced=4    (insert 'x')
  keep_rest()   →  consumed=6, produced=7    (keep "lo\n")

  Result: keep(3), insert("x"), keep(3)
  Cursor position at insert time = 4  →  "helx|lo\n"
```

All positions are in **original-buffer space**: no delta tracking, no
intermediate buffer clones.

## Building an edit from operations

Commands do not drive that builder directly. They record operations against
the old text with an edit builder, and the builder produces the changeset.
Each operation names a position or a range of the old text, and none depends
on what was recorded before it. The builder sorts the operations by position
and then writes the changeset front to back, so the order a command records
them in does not change the result. The one exception is operations at the
same position, which take effect in the order they were recorded.

With several cursors, ranges can overlap. Two deletions that overlap are
merged into their union, so a character covered by both is removed once. A
position strictly inside a deleted range resolves to the point where the
deletion happened, the same rule position mapping applies to any position.

```text
Delete "bcd" and delete "def" in "abcdefg":

  recorded:  delete(1..4), delete(3..6)
  merged:    delete(1..6)
  result:    "ag"
```

An insertion does not return an offset, since the new text does not exist yet.
It returns a handle for the run of text it produces. A command uses the handle
to say where a selection lands, and the handles are resolved once, against the
new text, when the edit finishes. A handle belongs to the one edit that made
it, so a position from one edit cannot be resolved against another.

Operations only name whole character clusters, so an edit cannot split an
accented letter or an emoji sequence. The structural trailing newline survives
every edit by one rule: text inserted at the very end of the buffer gets a
newline of its own, and when nothing is inserted there, every deletion stops
before the structural newline. See [Buffer Invariants](buffer-invariants.md).

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
By default the tree only grows and is dropped when the buffer closes. An
`undo-levels` limit bounds it by discarding the oldest states, so nodes can
be freed from the old end; see [The Undo Tree](undo-tree.md).

The buffer computes the inverse changeset before it replaces its text with the
forward result, which keeps the timing invariant intact.
