# core:undotree

A navigable graph of a buffer's undo history in the bottom drawer. Enter on a
row jumps the buffer to that revision, across branches. Depends on no other
plugin.

| File | Role |
|---|---|
| `manifest.scm` | Lazy activation on `:undotree` and `toggle-undotree`. |
| `plugin.scm` | The drawer session, the two commands, and the hooks that keep the drawer current. |
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
`(hash 'rows 'ids 'current)`: one row string per revision newest first, the
revision id behind each row, and the index of the current revision's row.

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
cells, a space, and the age right-aligned. The first marker cell is `@` for
the current revision, the second `S` for the saved one, and either is blank
otherwise. They sit in a column of their own so they stay visible next to a
wide graph. Here the current revision is 4 and the saved one is 0:

```
o          4m
| o    @   2m
| | o      6m
| | o      7m
| o-'      9m
o-'     S 12m
```

Every row is a revision, so a row always names a revision to jump to.

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

- After a jump, synchronously, so the `@` has moved when Enter returns.
- On `on-undo-history-changed` for the session's buffer, debounced 150 ms. That
  event also fires for an undo whose net change to the text is nothing, which
  `on-text-changed` would miss.
- On `on-buffer-enter` for another buffer, which retargets the session to the
  buffer now shown. When the session's pane has closed or shows another
  buffer, the next refresh or Enter retargets to the focused pane the same way.

## Limits

- The tree is in memory only, so it starts empty after a restart.
- Ages are relative. A saved timestamp would need a wall-clock format the
  history does not keep.
- Opening any other drawer, such as the LSP references list, replaces this one.
