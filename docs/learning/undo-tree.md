# The Undo Tree: Branches, Not a Stack

## The problem with a stack

Most applications implement undo as a stack: each action pushes a step; undo
pops one; redo pushes it back. The stack model works well until you undo a few
steps and then make a new edit. The new edit replaces the steps above the undo
point — they're gone, unreachable. If you realise a moment later that you
wanted one of those discarded edits, you can't get back to it.

## A tree preserves all branches

HUME's undo history is a tree, not a stack. Every edit creates a new node as a
child of the current position. When you undo back two steps and then make a
new edit, a new branch grows from that point:

```
     root
       │
       A
       │
       B   ←── undo to here, then make edit C
      / \
     C   D   ←── D is the branch you undid past; it's still there
```

`D` is still reachable — you can undo `C` and then redo to reach `D`. Undoing
and redoing never throws an edit away; branches only disappear when the tree
is deliberately bounded (see "Bounding the tree" below).

When you redo from a branch point, the most recent child is chosen — after
undoing and making a new edit, subsequent redo takes you along the new branch,
which is usually what you want. The old branch remains accessible via the
branch point.

## What each node stores

Every node in the tree stores:

- **A forward transaction**: the changeset and cursor positions that get you
  from the parent state to this state.
- **An inverse transaction**: the changeset and cursor positions that get you
  from this state back to the parent. Storing the inverse at creation time
  (when the original text is still available) avoids having to reconstruct it
  later.
- A list of child node references.
- A timestamp, reserved for future time-travel navigation (`:earlier N
  minutes`, `:later N minutes`, or any history-browsing UI); not yet wired as
  user commands.

The transactions, not the buffer content, are what's stored. At any point the
editor holds one live buffer. Undoing applies the inverse transaction to the
live buffer; redoing applies the forward transaction. No buffer snapshots are
needed, and very large files don't consume proportionally large memory in the
undo history.

## The arena

All nodes live in a lookup table (an "arena"), and references between nodes
are stable identifiers into that table rather than pointers. This is
idiomatic Rust for tree structures — unlike linked lists built from
heap-allocated nodes connected by owning pointers, an arena has no ownership
cycles for the type system to object to.

Each identifier is assigned once, in creation order, and never reused — not
even after the node it names is later dropped by bounding (below). That
matters for anything outside the tree that remembers a node by identifier
(the save-point marker described next, for instance): a dropped node's
identifier simply stops matching anything, rather than risking a coincidental
match with an unrelated, newer node.

## Bounding the tree

Left alone, the tree grows for as long as the buffer stays open — every edit
is one more node, forever. An `undo-levels` limit caps how many states are
kept per buffer (`0`, the default, means unlimited).

The limit is enforced the moment a new edit is recorded, by discarding from
the *oldest* end of the tree — never from the branch you're currently on.
When the oldest surviving point in the tree has more than one branch growing
from it, the oldest branch that isn't the one you're on is dropped as a
whole, in one step — every node in it, not just its tip. When there's only
one branch to drop from, the tree's starting point advances to the next node
along that branch instead, so a bound tree still remembers as much recent
history as the limit allows even along a single, unbranching line of edits.

Because a bounding pass can drop an entire old branch in one step, the tree
can end up noticeably smaller than the limit right after the pass — the same
tradeoff as any bulk cleanup. Lowering the limit doesn't retroactively trim
anything by itself; it only takes effect the next time an edit is recorded.

## Revision IDs double as the dirty/clean oracle

Each revision in the arena has a stable identifier. The buffer keeps one extra
reference: the id of the revision that was current the last time the file was
saved. The buffer is "dirty" exactly when the current id differs from that saved
id, and "clean" exactly when they match.

This sounds obvious but a separate `dirty: bool` flag could not do the same job.
Undo back to the save point would have to remember the original value of the
flag, and so would any cross-branch jump, and so would any reload that short-
circuits through the identity case. Using the revision id makes the question
*structural* — the buffer is clean precisely at one position in the tree, and
every navigation primitive already updates the current position. No bookkeeping
duplicates that fact.

Bounding the tree respects this oracle rather than fighting it. If the saved
state is the node a bounding pass drops the tree's starting point *to* (the
single-branch case above), the save marker moves along with it, so the clean
state stays reachable. If the saved state instead sits inside a branch that
gets dropped outright, there is no way back to it — the buffer simply reads
dirty from then on, which is correct: that exact state no longer exists
anywhere in the tree.

There's a third case: the saved state can *be* the tree's starting point
itself — the file was opened but never saved since, so "clean" still means
"back at the very beginning." When the single-branch case advances the
starting point, whatever content used to live there is gone; the position in
the tree is the same, but what it represents has changed. The save marker
can't just "stay put" here, because staying put would silently point at a
different state than the one that was actually saved. The buffer reads dirty
from that point on — correct, for the same reason as the dropped-branch case:
the exact state that was saved no longer exists anywhere in the tree.

## Reloads join the tree instead of replacing it

Earlier sections described undo and redo as walking the tree. A reload from disk
does not escape the tree: it derives a forward and inverse transaction from a
line-level diff against the new file content and records that as a normal
revision, branching from wherever the buffer currently sits. Undo after `:e!`
brings the pre-reload text back with its entire prior undo tree intact beneath
it. When the disk content matches the buffer exactly, no revision is recorded —
the saved-id pointer is just re-anchored to the current position.

## Jumping to an arbitrary revision

HUME exposes a goto-revision primitive as a building block for future
history-browsing UI. It is not yet wired to a `:goto-revision`, `:earlier`,
or `:later` user command; today only the editor's own tests call it. Given a
target node anywhere in the tree, the algorithm:

1. Walks up from the current node and from the target node to collect their
   respective ancestor chains.
2. Finds the **lowest common ancestor** (LCA) — the deepest node that both
   chains share.
3. Plays back inverse transactions from the current node up to the LCA
   (undoing each step).
4. Plays forward transactions from the LCA down to the target (redoing each
   step).

The caller applies each transaction in sequence. This correctly handles jumping
across branches, jumping to earlier points on the same branch, and even jumping
forward to future points that are reachable only through a later branch.

---

*See also: [Changesets](changesets.md) for what a transaction contains and why
the inverse is captured at edit time.*
