# hume-editing: the selection and edit model

A map of how a text, its selections and an edit fit together, and where each
invariant is enforced. The reasons behind each type live on its own doc
comment; this page only says where to look.

## Types

| Type | What it is | Made by |
|---|---|---|
| `ClusterStart`, `ClusterBound`, `ClusterRange` (`hume-rope/src/cluster.rs`) | A cluster start, a cluster start or the text end, a non-empty run of whole clusters | `hume-rope`'s grapheme and line primitives; a foreign offset enters through `BufferText::snap`/`covering`/`within` |
| `BufferText`, `TextVersion` (`text.rs`) | The rope; which content it holds; `BufferText::generation`, where it stands in the order its lineage's texts were made in | `BufferText::from`; applying a changeset that changes the text gives a new version and the next generation, and undo restores an earlier version at the next generation |
| `Selection` (`selection/single.rs`) | Anchor, head, sticky column | Commands, from cluster positions |
| `SelectionSet` (`selection/mod.rs`) | The stored selections of one pane on one buffer, tagged with a `TextVersion` | `EditState::into_selections` |
| `EditState` (`state.rs`) | A text and a set that fits it: the input and output of every command | `EditState::bind`, `EditState::with_cursor` |
| `EditView`, `SelectionView` (`selection/view.rs`) | Reads of a set against its text: every extent a command needs | `EditState::view`, `EditView::bind` |
| `EditBuilder`, `NewPos`, `Mark` (`edit/builder.rs`) | The operations of one edit, recorded against the old text, and positions in the text it produces | `edit::edit` |
| `Landing`, `Landings` (`edit/builder.rs`) | The selections an edit leaves, described before the new text exists | A command's `edit` closure |
| `Edited`, `TextChange` (`edit.rs`) | A finished edit; a change seen with the texts on both sides | `edit::edit`, `Edited::from_changes`, `Edited::rebased`; `TextChange::new` |
| `Resolver` (`selection/resolver.rs`) | Maps old-text positions into the new text | `Landings` resolution, `SelectionSet::translate` |
| `Transaction`, `History` (`transaction.rs`, `history.rs`) | A changeset with the selections it lands on; the undo tree | `History::record` |
| `Tracked<T>` (`tracked.rs`) | A value readable only against the text it was computed for | `Tracked::new` |

## One edit, end to end

1. `Buffer::apply_edit` (`hume-editor/src/editor/buffer/mod.rs`) pairs the
   pane's `SelectionSet` with the buffer text through `EditState::bind`.
2. The command (`hume-ops`, usually through `hume-ops/src/edit/mod.rs`'s
   `apply_edit`) calls `edit::edit`, records operations on the
   `EditBuilder`, and returns one `Landing` per selection.
3. `EditBuilder` sorts the operations by position, merges overlapping
   deletions, and applies them under `edit::apply_keeping_final_break`.
4. Each `Landing` resolves against the new text into a `Selection`, and
   `SelectionSet::from_parts` sorts and merges them. The result is an
   `Edited`.
5. `Buffer::run_edit` checks `Edited::base` against the buffer's text.
   `Buffer::record_revision` stores the selection sets before and after the
   edit, and `Buffer::install` swaps the text in.
6. `install` calls `PositionStores::carry`
   (`hume-editor/src/editor/position_stores.rs`), which carries every stored
   position through the `TextChange`: each pane's `SelectionSet` through
   `SelectionSet::translate`, the other stores through their own `Tracked`
   values or translate methods.

## A motion

`apply_doc_motion` (`hume-editor/src/editor/doc_ops.rs`) binds the pane's set
into an `EditState`, and the motion returns a new one through `EditState::map`,
`flat_map`, `with_selections` or `replace_primary`. The text does not change,
so only the pane's set is stored back.

## Undo

`History` holds a forward and an inverse `Transaction` per revision except
the root. A `Transaction`'s selection set is tagged with the version of the
text it lands on, and `Transaction::apply` gives the text it produces that
version through `ChangeSet::apply_as`, then pairs the two. A walk whose
changes cancel out keeps the text and its generation and relabels it; every
other walk gets the next generation, so what orders texts never goes back.
`Edited::rebased` uses the same step to land a paste cycle's result on the
live text.

## Where each invariant is enforced

| Invariant | Enforced by |
|---|---|
| No selection end splits a cluster | `ClusterStart` has no public constructor; `Selection` holds only `ClusterStart`s |
| A set is read only against its own text | The `TextVersion` tag, checked by `selection/fit.rs` from `EditState::bind` and `EditView::bind` |
| Equal versions hold equal content | New versions come only from applying a changeset; an earlier one is given back only by `Transaction::apply`, from a changeset that reproduces its content |
| Language servers, syntax and change announcements see texts in order | `BufferText::generation` only increases; `Buffer::install` asserts it |
| A set is non-empty, sorted, non-overlapping, with a valid primary | `SelectionSet::from_parts` |
| Commands do not compute extents | `SelectionView`: `covered`, `content`, `append_point`, `line_spans` |
| No edit splits a cluster, and recording order does not change the result | `EditBuilder` takes `ClusterBound`/`ClusterRange`, and applies operations in position order |
| The text ends with `\n` | `edit::apply_keeping_final_break` |
| A new-text position is used only by the edit that made it | The `'id` brand on `NewPos`, `Mark` and `Landing` |
| An edit is installed only on the text it was made from | `Buffer::run_edit` |
| Stored positions follow the text | `PositionStores::carry`; a `Tracked` value it does not carry reads as absent once the text changes |

## Tests

`marked.rs` defines the notation tests write a text and its selections in
(`parse`, `render`).
