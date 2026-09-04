# HUME — Undo-Tree Visualizer

Design hub for a `core:undotree` plugin (unbuilt) — a navigable graph over
HUME's undo history, in the spirit of `mbbill/undotree`. Diff-per-node is a
nice-to-have; the graph and node-jumping are the hard requirement.

## How to use this document

Same rules as `docs/LSP.md`:

1. **Verify before you write.** The codebase moves — `rg 'symbol_name'` before
   relying on anything named here.
2. **If the doc contradicts the code, STOP** and report; don't silently adapt.
3. Where a decision cites a source location, treat the source as authoritative
   if the two ever diverge.

## What exists today

HUME's undo history is already a tree, not a stack — see
[The Undo Tree: Branches, Not a Stack](learning/undo-tree.md) for the concept.
The Rust substrate is largely there:

- An arena of revisions keyed by a stable, monotonically-assigned,
  never-reused id — `hume-editing/src/history.rs`, `struct History`.
- `parent`/`children` links on every node, plus a per-node `timestamp`.
- A working cross-branch jump: `History::goto_revision` walks up to the lowest
  common ancestor of the current and target nodes, then back down, returning
  the ordered transaction list to apply (`history.rs`, `goto_revision`).
- The save point (the dirty/clean oracle) is tracked outside the tree as
  `Buffer::saved_revision` (`hume-editor/src/editor/buffer/mod.rs`).
- The propagation path `undo`/`redo` already use —
  `doc_ops::apply_doc_undo`/`apply_doc_redo` → `finish_edit` — is what any new
  tree navigation must reuse: it re-syncs other panes' selections, tree-sitter,
  LSP, decorations, and jump lists after the buffer changes underneath them.

## What is missing, by layer

### `hume-editing`

The tree is not enumerable. `History` exposes only `parent(id)`,
`current_id()`, `len()`, and the `ROOT` constant — `Revision` itself is a
private struct with no accessor for its `children` or `timestamp` fields. From
outside the module you can walk *up* one ancestor chain and nothing else, so
siblings and branches — the entire content a graph needs to draw — are
invisible. `RevisionId` wraps a `pub(crate) usize` with no `impl` block at
all, so an outside crate can neither read the ordinal nor construct one from
an integer.

Needed: a public, read-only way to enumerate every revision's id, parent,
children, and timestamp, plus a `RevisionId` accessor/reconstructor.

### `hume-editor`

`Buffer::goto_revision` exists but is `#[cfg(test)]`-only and bypasses
everything a production mutation must go through: no read-only guard, no
`finish_edit`, so no pane propagation, no tree-sitter reparse, no LSP sync, no
decoration remap, no jump-list entry.

This means `docs/ROADMAP.md`'s `:earlier`/`:later` item, which says "the
substrate already exists (`History::goto_revision`, `Revision::timestamp`)",
is accurate only at the `hume-editing` layer — at the `hume-editor` layer, the
production path does not exist yet.

Needed: a `doc_ops::apply_doc_goto_revision` mirroring
`apply_doc_undo`/`apply_doc_redo` (loop the transaction list, `finish_edit`
per step), and a `Buffer::saved_revision()` accessor for the "this is the
saved node" marker.

### `hume-scripting`

No history builtins exist at all. `undo`/`redo` in the generated globals list
are just the two native command names (`(undo)`, `(redo)`) — there is no
`can-undo?`, no revision id, no revision list, no `goto-revision!`.

Needed: two builtins, e.g. `(buffer-undo-tree bid)` (returns per-node id,
parent, seconds-ago, current?, saved?) and `(goto-revision! bid id)`, backed
by new `BufferHost`/`EditHost` methods. Registering them requires
regenerating `runtime/plugins/core/steel-server/lsp-home/hume-globals.scm`
(`HUME_WRITE_STEEL_GLOBALS=1 cargo test -p hume-editor
hume_globals_scm_matches_generated_host_names`), which a drift test enforces.

## UI host options

| Host | New Rust needed | Trade-off |
|---|---|---|
| Bottom drawer (`show-drawer-list!`) | None | Capped height, flat unstyled rows, re-showing to refresh resets the selection to 0 |
| `set-virtual-lines!` with `'segments` | None | The only per-row styling primitive Steel has today, but it paints into an existing buffer rather than living in its own pane |
| Docked pane (the real undotree shape) | Docked-pane layout variant; a builtin to mint a non-file view buffer; per-buffer keymaps | Correct shape, but blocked on three separate pieces of infrastructure — see [Roadmap](#roadmap) |

The docked-pane route is blocked three ways specifically:

1. **No Steel way to mint a buffer that isn't backed by a file.** The only
   lifecycle surface is `open-buffer!` (path-based). The Rust-only
   `Editor::open_read_only_view` (used for `[messages]`, `[buffers]`,
   `[plugin-status]`, `[lsp-status]`) is the right shape but takes a
   `&'static str` label — a Steel-facing version needs an owned `String`.
2. **Panes are write-only from Steel.** `current-pane`/`panes` return
   `PaneId`s, but no builtin accepts one as input. `(pane-vsplit)` takes no
   arguments and splits the focused pane onto the *same* buffer; `:split
   <path>` is a typed command, and typed commands aren't callable from Steel.
   There is no way to say "open this buffer in a narrow left pane."
3. **No per-buffer keymaps** (`docs/ROADMAP.md`'s per-buffer-keymaps item,
   built on the existing `on-buffer-enter` hook). Without them, `Enter`-to-jump
   inside an `[undotree]` buffer can only be a global binding that checks the
   current buffer's name.

See [Panes, bands, and overlays](learning/splits-and-panes.md#panes-bands-and-overlays)
for the vocabulary a docked pane sits between.

## Constraints that shape the feature

- **No absolute timestamps.** `Revision::timestamp` is `std::time::Instant` —
  monotonic, not wall-clock, not serializable. "5 minutes ago" is free via
  `elapsed()`; "saved at 14:02" would need a `SystemTime` field added
  alongside it.
- **Per-node diff is a real lift, not a small one.** Each node stores its
  forward/inverse `ChangeSet`, but there is no way to obtain a revision's full
  *text* without actually navigating to it — there's no snapshot cache. A
  `+N/-M` size summary derived from the stored changeset is nearly free; an
  actual line diff between two arbitrary nodes is not, and `diff-lines`
  exists but has nothing to feed it today.
- **The tree is not persisted.** `hume-editing` has no `serde` dependency, so
  the undo tree dies with the session — there is no cross-session equivalent
  of Vim's `undofile`.

## Roadmap

Two phases. Phase 1 first — it needs one prerequisite instead of three, and it
de-risks the part of this feature that is actually unproven: whether an ASCII
undo graph reads well in a terminal, and whether jump-by-node feels good.

### Phase 1 — navigate the tree, drawer-hosted

- [ ] Public read-only enumeration API on `History` (`hume-editing`)
- [ ] `RevisionId` accessor + reconstructor (`hume-editing`)
- [ ] `Buffer::saved_revision()` accessor (`hume-editor`)
- [ ] Production `doc_ops::apply_doc_goto_revision` (`hume-editor`)
- [ ] `BufferHost`/`EditHost` methods backing the above (`hume-scripting`)
- [ ] `(buffer-undo-tree bid)` and `(goto-revision! bid id)` builtins, plus
      regenerated `hume-globals.scm` (`hume-scripting`)
- [ ] Graph renderer — pure Scheme, git-log-style lane assignment over the
      parent/children lists (`core:git-diff`'s `render.scm` is the shape:
      pure data → display records, no editor state inside the renderer)
- [ ] `core:undotree` plugin: `plugin.scm` + `render.scm` + `manifest.scm`,
      following `runtime/plugins/core/git-diff/`'s layout

### Phase 2 — the side panel

- [ ] Docked-pane `LayoutTree` variant (see `docs/ROADMAP.md`'s docked-panes
      item) and scoping `equalize` to skip docked panes
- [ ] Steel builtin to mint a read-only view buffer, the `[messages]` shape
      with an owned-`String` label
- [ ] Per-buffer keymaps
- [ ] Port the Phase 1 graph renderer unchanged — it is pure data → strings,
      independent of which host displays it

### Not planned

- Absolute timestamps — needs a `SystemTime` field with no current use case
  beyond this
- Full per-node diff view — real lift for a nice-to-have; a `+N/-M` summary
  covers most of the value
- Cross-session persistence — `hume-editing` has no serialization story at all
  today; out of scope for this feature to introduce
