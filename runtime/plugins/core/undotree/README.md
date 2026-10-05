# core:undotree

A navigable graph of a buffer's undo history in the bottom drawer. Enter on a
row jumps the buffer to that revision, across branches. With `core:git-diff`
loaded, the buffer also shows what the current revision changed. Needs no other
plugin.

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on `:undotree` and `toggle-undotree`. |
| `plugin.scm` | The drawer session, the two commands, the revision diff, and the hooks that keep the drawer current. |
| `render.scm` | Pure data to rows: `undotree/render`, `undotree/format-age`. Calls no editor builtin. |

## Usage

`:undotree` and the bindable `toggle-undotree` run one function: open the
drawer on the focused buffer's tree, or close it when it is already open. The
two names differ because mappable and typed commands share one namespace. A
lazy plugin ships no key, so bind one in `init.scm`:

```scheme
(bind-key! 'normal "z u" "toggle-undotree")
```

The drawer opens with the highlight on the current revision. The drawer's own
keys move the highlight (see the manual's drawer paragraph under `core:lsp`);
Enter jumps to the highlighted revision and leaves the drawer open, so
stepping through history is move, Enter, move, Enter. Esc closes it.

A row is the graph, two marker cells, and the age. `@` marks the current
revision and `S` the revision the file was last saved at (or first opened
as). After a jump, redo continues along the branch the jump entered.

## Graph

`undotree/render` takes the list `(buffer-undo-tree pane)` returns: one
`(hash 'id 'parent 'age-secs 'current? 'saved?)` per revision, ascending by
id, never empty. A revision's parent always has a smaller id, so walking the
list newest first visits every child before its parent. It returns
`(hash 'nodes 'render 'current 'current-node)`: the input hashes newest first,
one per row; a `(render start nodes)` procedure that formats the rows from
index `start` for those hashes; the index of the current revision's row; and
that revision's input hash. The plugin hands `'nodes` to the drawer as its row
keys and `'render` as its `#:render`, so only the rows the drawer shows are
formatted.

The renderer keeps a list of lanes. A lane is the id of the parent a column of
the graph is waiting to reach, or `#f` when the column is free. For each
revision, newest first:

1. Collect the lanes waiting for this revision's id. The leftmost is the
   revision's column, `c`; the others are merging lanes, `M`. With none
   waiting the revision is a branch tip and takes the first free column,
   appending one when there is none.
2. Draw one row, a cell per column. Column `c` is `o`, a merging lane is `'`,
   any other occupied lane is `|`, and a free column is blank. Between two cells the gap is `-` when
   `c <= i < max(M)`, else blank.
3. Point lane `c` at the revision's parent (free for the root), free every
   merging lane, and drop trailing free lanes.

Each row is the graph padded to the widest graph, two spaces, two marker
cells, a space, and the age right-aligned in a column as wide as the oldest
revision's age, and at least 3 cells: an age in seconds, minutes or hours is
at most 3 cells, and an age in days only grows with age, so the column fits
every row without measuring each one. The first marker cell is `@` for
the current revision, the second `S` for the saved one, and either is blank
otherwise. They sit in a column of their own so they stay visible next to a
wide graph. Here the current revision is 4 and the saved one is 0:

```
o          2m
| o    @   4m
| | o      6m
| | o      7m
| o-'      9m
o-'     S 12m
```

Every row is a revision, so a row always names a revision to jump to.

The graph depends only on each revision's id and parent. `undotree/render`
keeps the graph rows of its last call and reuses them when the list of
`(id . parent)` pairs, newest first, is `equal?` to that call's. An undo, redo
or jump moves only the markers, so it skips the lane walk; the markers and ages
are formatted on every render. The key is the whole input to the lane walk, so
there is nothing to invalidate.

## Age

`undotree/format-age` floors whole seconds to the largest unit that fits:
`Ns` under a minute, `Nm` under an hour, `Nh` under a day, else `Nd`. These
are the units `:earlier` and `:later` accept.

## Session

One session at a time, since the drawer is one slot: a hash of an id, the
drawer token, the pane the tree is read from, that pane's `buffer-key`, and
the revision id behind each row. The id guards every drawer callback. A
callback from a drawer the session has since replaced finds a different id
and does nothing. Esc, or another feature opening its own drawer, delivers
`#f` to the callback and ends the session.

`goto-revision!` raises for a read-only buffer, and the error reaches the user
as any Steel error does; the session stays open.

## Refresh

The rows are re-rendered in place, keeping the highlight on the same revision
number, or on the current revision when that number is gone or the tree
belongs to a different buffer.

- After a jump, through the next item: the jump moves the current revision.
- On `on-undo-history-changed` for the session's buffer. The editor runs that
  hook before it draws the next frame, so the text change, the `@` and the
  revision diff appear together. The event fires once per revision change, not
  per typed key, and also for an undo whose net change to the text is nothing,
  which `on-text-changed` would miss.
- Every 60 seconds while the session is open, so the ages keep up with the
  clock. The timer is cancelled when the session ends.
- On `on-buffer-enter` for another buffer, which retargets the session to the
  buffer now shown. When the session's pane has closed or shows another
  buffer, the next refresh or Enter retargets to the focused pane the same way.

## Revision diff

While the drawer is open the buffer shows what separates the current revision
from its parent, drawn inline under the decoration source `"undotree"`:
the parent's lines as deleted lines, word highlights and a line tint. The
hunks come from `(buffer-revision-diff pane parent-id)`, so the parent's text
is the old side and the buffer's text the new, and `core:git-diff` draws them
through its `git-diff/render-diff` command (see its `docs/rendering.md`). The
root has no parent and shows nothing. From the first draw until the session
clears it through `git-diff/release-diff`, `core:git-diff` hides its own inline
diff of the buffer, so changed lines are not drawn twice. On the root it stays
hidden too, since a diff against git there would read as the revision's own.

The plugin calls no other plugin's code unless `(command-exists?
"git-diff/render-diff")` and `(command-exists? "git-diff/release-diff")` hold,
which is true once `core:git-diff` is loaded, even before it has run. Without
it nothing is drawn and no error is raised; opening the drawer shows one
message in the status line saying to load it. A user who loaded
`core:git-diff` with an activation list that leaves either command out gets
the same message.

The session remembers the buffer and revision it last drew. Wherever the rows
are re-rendered (after a jump, on a history change, on the age timer, when the
session retargets) the diff is redrawn only when the buffer or its current
revision differs from that, since a revision's text never changes. So the age
timer never redraws it. Retargeting to another buffer first clears the diff of
the buffer it leaves. Ending the session clears it too, for a buffer that is
still open.

## Limits

- The tree is in memory only, so it starts empty after a restart.
- Ages are relative. A saved timestamp would need a wall-clock format the
  history does not keep.
- Opening any other drawer, such as the LSP references list, replaces this one.
