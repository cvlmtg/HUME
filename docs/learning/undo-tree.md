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

`D` is still reachable — you can undo `C` and then redo to reach `D`, or jump
directly with `:goto-revision`. No edit is ever thrown away; the tree only
ever grows.

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
- A timestamp (for future `:earlier N minutes` style navigation).

The transactions, not the buffer content, are what's stored. At any point the
editor holds one live buffer. Undoing applies the inverse transaction to the
live buffer; redoing applies the forward transaction. No buffer snapshots are
needed, and very large files don't consume proportionally large memory in the
undo history.

## The arena

All nodes live in a flat list (an "arena"), and references between nodes are
just integer indices into that list. This is idiomatic Rust for tree structures
— unlike linked lists built from heap-allocated nodes connected by owning
pointers, an arena has no ownership cycles for the type system to object to,
and every access is a direct array index.

The trade-off: individual nodes are never freed. The whole arena is discarded
only when the buffer closes. For an undo tree that only ever grows, this is
fine — the tree is bounded by the number of edits ever made in a session, which
is manageable.

## Jumping to an arbitrary revision

HUME exposes `goto_revision` (used internally for `:earlier`/`:later` and
available as a building block for future history-browsing UI). Given a target
node anywhere in the tree, the algorithm:

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
