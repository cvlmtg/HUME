# HUME — Undo-Tree Visualizer

Design hub for `core:undotree`: a navigable graph over HUME's undo history, in
the spirit of `mbbill/undotree`. The graph and jumping to a node ship in the
bottom drawer (Phase 1). The side-panel host (Phase 2) and the revision diff
are open.

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
- **No revision text.** History stores transactions, never texts, so a
  revision's full text exists only by walking the buffer to it. The revision
  diff below never needs one.
- **The tree is not persisted.** `hume-editing` has no `serde` dependency, so
  the undo tree dies with the session — there is no cross-session equivalent
  of Vim's `undofile`.
- **One drawer at a time.** Opening any other drawer replaces the tree.

## Revision diff

While the drawer is open, the buffer shows what separates the current text
from the previous revision, drawn inline the way `core:git-diff`'s inline mode
draws a diff against a git ref: virtual deleted lines, word highlights and a
line tint.

### Hunks come from the changeset, not from a text diff

`History::goto_revision` already finds the path between two revisions and
returns its transactions. Composed with `ChangeSet::compose_all`, they are one
changeset `C` that maps the current text to the target revision's text.
Walking `C`'s ops (`Retain`, `Delete`, `Insert`) over the current text gives
every changed span:

- `Retain(n)`: text both sides share.
- `Delete(n)`: current text the revision lacks, the plus side.
- `Insert(s)`: revision text the current text lacks, the minus side, carried
  in `s` itself.

Touching or adjacent changed line ranges merge into one hunk. A hunk's new-side
lines are read from the current buffer, and its old-side lines are those same
lines with `C` spliced in. The old-side line number is the new-side one,
corrected by the line breaks `C` deletes and inserts before it. No full text is
rebuilt and no Myers pass runs, so the cost scales with the edit, not the
buffer.

The word spans are the `Delete` and `Insert` positions themselves, so a hunk
carries them and needs no `diff-words` call. A revision recorded as
whole-line replacements (`:e!`'s reload, through `changesets_from_line_diff`)
has no word-level detail; for its paired lines the renderer falls back to
`diff-words`.

### Hunks cross the Steel boundary, changesets do not

`(buffer-revision-diff pane id)` returns a list of hunks in the shape
`diff-buffer-lines` returns, plus each hunk's word spans.

- **One shape for every diff.** A git diff and a revision diff are both hunks,
  so one renderer draws both.
- **No raw offsets in plugins.** A changeset's ops are bare char counts, valid
  only against the one text they were built for. Hunks are line-domain and
  carry their own text.
- **The changeset stays internal.** Its op encoding and how history stores it
  remain free to change.
- **No second edit path.** An exposed changeset invites an
  `apply-changeset!`, which would bypass `EditBuilder` and
  `edit::apply_keeping_final_break`. No plugin has a use for raw changesets
  that the native side does not already cover (incremental LSP and tree-sitter
  sync, `PositionStores`).

### Decisions

- **The diff is the current revision against its parent.** Enter jumps and
  the drawer stays open, so after each jump the buffer shows what that
  revision changed. The root has no parent and shows no diff.
- **Inline in the buffer, under the plugin's own decoration source.** It never
  overwrites `core:git-diff`'s decorations, and closing the drawer clears it.
- **One renderer.** `core:git-diff`'s hunk renderers take the decoration
  source as a parameter and move where both plugins can reach them. Where
  that is stays open: `core:undotree` depends on no other plugin today.

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

### Revision diff

- [ ] Read-only path query on `History`: the transactions from `current` to a
      target without moving `current` or the redo targets. `goto_revision`
      calls it, so the LCA walk has one implementation (`hume-editing`)
- [ ] Changeset-to-hunks walk with word spans, tested on inserted and deleted
      line breaks, edits at either end of the text, and the structural final
      `\n` (`hume-editing`)
- [ ] `DiffHost` method and `(buffer-revision-diff pane id)`, regenerated
      `hume-globals.scm` (`hume-scripting`)
- [ ] `core:git-diff`'s hunk renderers parameterized by decoration source and
      shared, using a hunk's own word spans when present
- [ ] `core:undotree` draws the diff on open, jump and history change, and
      clears it on close

### Not planned

- Absolute timestamps — needs a `SystemTime` field with no current use case
  beyond this
- A `+N/-M` size summary per row
- Raw changesets in Steel — see "Hunks cross the Steel boundary, changesets do
  not"
- Cross-session persistence — `hume-editing` has no serialization story at all
  today; out of scope for this feature to introduce
