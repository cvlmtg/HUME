# The Undo Tree: Branches, Not a Stack

## The problem with a stack

Most applications implement undo as a stack: each action pushes a step; undo
pops one; redo pushes it back. The stack model works well until you undo a few
steps and then make a new edit. The new edit replaces the steps above the undo
point, and they become unreachable. If you realise a moment later that you
wanted one of those discarded edits, you can't get back to it.

## A tree preserves all branches

HUME's undo history is a tree. Every edit creates a new node as a child of the
current position. When you undo back two steps and then make a new edit, a new
branch grows from that point:

```
     root
       │
       A
       │
       B   ←── undo to here, then make edit C
      / \
     C   D   ←── D is the branch you undid past; it's still there
```

`D` is still reachable: it is a branch from `B`, and you can jump straight to
it (see "Jumping to an arbitrary revision" below). Undoing and redoing never
throws an edit away; branches only disappear when the tree is bounded (see
"Bounding the tree" below).

When you redo from a branch point, redo follows the branch you last moved
along: each node remembers which of its children that was. A new edit makes
its branch the one to follow, so after undoing and making a new edit, redo
takes you along the new branch. Jumping into an older branch makes that branch
the one to follow, so redo stays on it. The other branches remain reachable by
jumping.

## What each node stores

Every node except the root, which is the state before any edit, stores:

- **A forward transaction**: the changeset and cursor positions that get you
  from the parent state to this state.
- **An inverse transaction**: the changeset and cursor positions that get you
  from this state back to the parent. It is computed when the edit happens,
  while the original text is still available, so it never has to be
  reconstructed.
- A list of child node references.
- A timestamp, which `:earlier` and `:later` read to travel by age
  (`:earlier 5m` goes back to how the buffer looked five minutes ago). Both
  also take a step count, and each walk is the same one `u` and `Ctrl-r` take.

The transactions, not the buffer content, are what's stored. At any point the
editor holds one live buffer. Undoing applies the inverse transaction to it;
redoing applies the forward transaction. No buffer snapshots are needed, so a
very large file does not make the undo history proportionally large.

See [Changesets](changesets.md) for what a transaction contains.

## The arena

All nodes live in one lookup table (an "arena"), and references between nodes
are stable identifiers into that table rather than pointers. This is idiomatic
Rust for tree structures: unlike a tree of heap-allocated nodes connected by
owning pointers, an arena has no ownership cycles for the type system to object
to.

Each identifier is assigned once, in creation order, and never reused, not even
after the node it names is dropped by bounding (below). Anything outside the
tree that remembers a node by identifier, such as the save marker described
later, therefore stops matching when its node is gone, rather than matching an
unrelated newer node by coincidence.

## Bounding the tree

Left alone, the tree grows for as long as the buffer stays open: every edit is
one more node. An `undo-levels` limit caps how many states are kept per buffer
(`0`, the default, means unlimited).

The limit is enforced when a new edit is recorded, by discarding from the
*oldest* end of the tree, never from the branch you're on. When the oldest
surviving point has more than one branch growing from it, the oldest branch
that isn't yours is dropped whole, every node in it at once. When there is only
one branch, the tree's starting point advances to the next node along it, so a
bounded tree still keeps as much recent history as the limit allows along a
single line of edits.

Because a pass can drop a whole branch at once, the tree can end up noticeably
smaller than the limit right after it. Lowering the limit trims nothing by
itself; it takes effect the next time an edit is recorded.

## Revision IDs answer "is this buffer saved?"

The buffer remembers the identifier of the revision that was current when the
file was last saved. The text matches the file when the current revision is
that saved one. A separate `dirty: bool` flag could not do the same job: undo
back to the save point would have to restore the flag's old value, and so would
every jump across branches and every reload that records nothing. Comparing
revision identifiers makes the question structural. The buffer is clean at one
position in the tree, and every way of moving through the tree already updates
the current position.

Two other things also make a buffer unsaved, because neither is a revision: a
line ending different from the one last saved, and edits in an open insert or
paste session, which become a revision only when the session ends.

Bounding the tree keeps this answer correct. When the starting point advances
onto the saved revision, the save marker follows it, so the clean state stays
reachable. When the saved state is dropped, because it sat in a removed branch
or it was the old starting point whose content the advance replaced, the
buffer reads unsaved from then on. That is correct: the saved state does not
exist anywhere in the tree.

## Reloads join the tree

A reload from disk (`:e!`) does not throw the history away. It derives a
forward and an inverse transaction from a line-level diff between the buffer
and the new file content, and records them as a normal revision, branching
from wherever the buffer currently sits. Undo after `:e!` brings the
pre-reload text back with its whole undo tree intact beneath it. When the disk
content matches the buffer, no revision is recorded. Either way the save marker
moves to the current revision, since the buffer now matches the file.

## Jumping to an arbitrary revision

The history can jump from the current node to any node in the tree. The
`core:undotree` plugin exposes that jump: Enter on a row of its graph moves the
buffer to that revision. `:earlier` and `:later` follow the chain redo follows
instead. Given a target node anywhere in the tree, the walk:

1. Finds the **lowest common ancestor**, the deepest node both the current
   node and the target descend from. A node's identifier is always larger than
   its parent's, so the walk repeatedly steps whichever of the two has the
   larger identifier up to its parent, until they meet.
2. Plays back inverse transactions from the current node up to that ancestor,
   undoing each step.
3. Plays forward transactions from the ancestor down to the target, redoing
   each step.

The transactions are composed into one changeset, which is applied to the
buffer once. The same holds for `5u` and an age-based `:earlier`: however many
steps the walk spans, the buffer changes once, so everything that follows the
text (other panes, syntax, language servers) sees a single change. The walk
handles jumps across branches, back along the same branch, and forward to
states reachable only through another branch.

## Undo restores the text's version

Every text carries a version label, and every set of selections remembers the
version of the text it was made for. A selection set is only readable against
its own text, so undo has to hand back a text that wears the label its
selections were recorded for. Applying a stored transaction therefore gives
the resulting text that recorded label, and pairs it with the stored
selections.

Texts are also ordered by a generation number that only increases, and
language servers and syntax parsing rely on that order. An undo walk that
changes the text gets the next generation. A walk whose changes cancel out,
such as undoing an insertion and its own later deletion, keeps the same text
and the same generation and only relabels it, so nothing downstream sees a
change that did not happen.

---

*See also: [Changesets](changesets.md) for what a transaction contains and why
the inverse is captured at edit time.*
