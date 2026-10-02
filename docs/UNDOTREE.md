# HUME — Undo-Tree Visualizer

Design hub for `core:undotree`: a navigable graph over HUME's undo history, in
the spirit of `mbbill/undotree`. The graph and jumping to a node ship in the
bottom drawer (Phase 1). The side-panel host (Phase 2) is open.

## How to use this document

Same rules as `docs/LSP.md`:

1. **Verify before you write.** The codebase moves — `rg 'symbol_name'` before
   relying on anything named here.
2. **If the doc contradicts the code, STOP** and report; don't silently adapt.
3. Where a decision cites a source location, treat the source as authoritative
   if the two ever diverge.

## What Phase 1 built

HUME's undo history is a tree, not a stack — see
[The Undo Tree: Branches, Not a Stack](learning/undo-tree.md) for the concept.

- **The tree is enumerable.** `History::nodes(now)` returns every revision's
  id, parent and age in id order, and `RevisionId::checked(&History, n)` is the
  only public way to mint an id from a number, so a number from Steel is
  validated against the history it names (`hume-editing/src/history.rs`).
- **Redo follows the child last walked through.** Each revision keeps its
  children in creation order, which `undo-levels` eviction relies on, and a
  redo target the private `Children` type keeps a member of them
  (`hume-editing/src/history/children.rs`). A new edit makes its branch the
  target and so does a jump into one, so after jumping into an older branch redo
  continues along it. `:later` walks the same chain as `redo`.
- **One production jump.** `commands::goto_revision` mints the revision through
  `Buffer::revision`, then runs `Buffer::goto_revision` through
  `doc_ops::apply_doc_history_walk`, the funnel `u`/`U`/`:earlier`/`:later`
  also use, so a jump re-syncs other panes' selections, tree-sitter, LSP,
  decorations and jump lists like any other edit. `EditHost::goto_revision` is
  its only other caller.
- **The funnel owns the acting pane's session.** `apply_doc_history_walk`
  commits a paste session open on its own pane and buffer before walking, and
  refuses a walk under that pane's open Insert session. Every history walk goes
  through it, so a Steel caller is covered too.
- **`on-undo-history-changed`.** Raised by diffing `History::change_seq`
  against a per-buffer baseline at the drain point `on-text-changed` uses, so it
  fires for a walk whose changes cancel out and `on-text-changed` does not.
- **Two builtins.** `(buffer-undo-tree pane)` returns one
  `(hash 'id 'parent 'age-secs 'current? 'saved?)` per revision, and
  `(goto-revision! pane id)` jumps. Both are `cmd`-gated, and
  `runtime/plugins/core/steel-server/lsp-home/hume-globals.scm` lists them.
- **The plugin.** `runtime/plugins/core/undotree/` — `render.scm` is a pure
  data-to-rows renderer, `plugin.scm` holds the drawer session. Its README
  documents the lane algorithm, the session and the refresh rules.

## Decisions

- **One row per revision.** A merge is drawn on the fork's own row (`o-'`), so
  every row names a revision and Enter always has somewhere to jump. The
  alternative, git-style connector rows (`|/`), adds rows with no revision
  behind them.
- **Markers in a column of their own.** `@` (current) and `S` (saved) sit after
  the graph, not among its glyphs, where they were hard to see in a wide graph.
  A row shows no revision number, since Enter jumps by row.
- **Enter jumps, moving the highlight does not.** The drawer's callback fires
  on Enter and Esc only, and a jump per highlight move would push a jump-list
  entry and an LSP sync per row.
- **Names.** `:undotree` and `toggle-undotree` run one function. Mappable and
  typed commands share one namespace, so they cannot both be `undotree`.
- **A lazy plugin ships no key.** Its commands are its only entry points, so the
  user binds one in `init.scm`.

## Constraints

- **No absolute timestamps, only relative ones.** `Revision::timestamp` is a
  wall-clock `std::time::SystemTime`, not serializable across sessions — "5
  minutes ago" is free via `duration_since`; "saved at 14:02" would need
  formatting support, not a new field.
- **Per-node diff is a real lift, not a small one.** Each node stores its
  forward/inverse `ChangeSet`, but there is no way to obtain a revision's full
  *text* without actually navigating to it — there's no snapshot cache.
- **The tree is not persisted.** `hume-editing` has no `serde` dependency, so
  the undo tree dies with the session — there is no cross-session equivalent
  of Vim's `undofile`.
- **One drawer at a time.** Opening any other drawer replaces the tree.

## Roadmap

### Phase 1 — navigate the tree, drawer-hosted

- [x] Public read-only enumeration on `History` (`hume-editing`)
- [x] `RevisionId::checked` / `RevisionId::index` (`hume-editing`)
- [x] Redo follows the last-walked child (`hume-editing`)
- [x] `Buffer::saved_revision()` and the other `Buffer` delegates (`hume-editor`)
- [x] Production `Buffer::goto_revision` through `doc_ops::apply_doc_history_walk`,
      which handles the acting pane's own session (`hume-editor`)
- [x] `on-undo-history-changed` (`hume-editor`)
- [x] `BufferHost`/`EditHost` methods, `(buffer-undo-tree pane)` and
      `(goto-revision! pane id)`, regenerated `hume-globals.scm` (`hume-scripting`)
- [x] Graph renderer, pure Scheme (`render.scm`)
- [x] `core:undotree` plugin: `manifest.scm`, `plugin.scm`, `render.scm`

### Phase 2 — the side panel

- [ ] Docked-pane `LayoutTree` variant (see `docs/ROADMAP.md`'s docked-panes
      item) and scoping `equalize` to skip docked panes
- [ ] Steel builtin to mint a read-only view buffer, the `[messages]` shape
      with an owned-`String` label (`Editor::open_read_only_view` takes a
      `&'static str` today)
- [ ] Panes as Steel inputs: `focused-pane`/`panes` return `PaneId`s but no
      builtin accepts one to place a buffer in a narrow pane
- [ ] Per-buffer keymaps
- [ ] Port the Phase 1 renderer unchanged — it is pure data to strings,
      independent of which host displays it

### Not planned

- Absolute timestamps — needs a `SystemTime` field with no current use case
  beyond this
- A `+N/-M` size summary or full per-node diff view — real lift for a
  nice-to-have
- Cross-session persistence — `hume-editing` has no serialization story at all
  today; out of scope for this feature to introduce
