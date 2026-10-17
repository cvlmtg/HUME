//! LSP-driven text edits, workspace edits, and go-to-location, plus
//! `goto-revision!` (undo-history navigation) and `insert-key!`, which edit
//! too but aren't LSP-driven. They live here rather than in their own trait
//! because one capability accessor (`EditorHost::edits`) already gates every
//! buffer-mutating builtin that isn't a named editor command, and a second
//! accessor for them would only duplicate that gate.

use hume_engine::pipeline::BufferId;
use hume_rope::position_encoding::PositionEncoding;
use termina::event::KeyEvent;

use crate::types::PaneHandle;

/// One `apply-text-edits!` entry: a wire range plus its replacement text.
/// `encoding` is this entry's own producing server's negotiated encoding:
/// each entry decodes from its own `JsonHandle`, so a batch built from more
/// than one response could in principle disagree; the host implementation
/// checks every entry in a batch agrees before applying any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextEdit {
    pub range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
    pub new_text: String,
    pub encoding: PositionEncoding,
}

/// LSP-driven text edits, workspace edits, and go-to-location, accessed
/// through [`EditorHost::edits`](super::EditorHost::edits).
///
/// Every method but [`Self::insert_key`] is kind-B: it acts through `pane`'s
/// own pane (mapping selections through the edit, moving the cursor on
/// goto), not the focused one. A response landing after the user has since
/// switched panes must still edit or navigate the pane the request was
/// actually made from. `Err` (fail-fast) when `pane` carries no pane, a
/// closed one, or one that shows another buffer. The exceptions are
/// [`Self::apply_text_edits`] and [`Self::apply_workspace_edit`], which edit
/// through the pane while it shows the buffer being edited and otherwise for
/// no pane.
pub trait EditHost {
    /// `(apply-text-edits! pane edits #:expect-generation gen)`: `edits` is
    /// a list of wire-coordinate ranges plus replacement text. Applied as
    /// one undo step. A `pane` that names a pane showing the buffer edits
    /// through it; any other `pane` edits that buffer for no pane, carrying
    /// every pane's selections and refusing while any session is open on it.
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;

    /// `(apply-workspace-edit! pane edit #:expect-generation gen)`: `edit`
    /// is an LSP `WorkspaceEdit` wire JSON blob, read-only here (deserialized
    /// into a typed `WorkspaceEdit` without needing ownership), so the
    /// builtin passes a borrow of its `JsonHandle`/hashmap argument rather
    /// than cloning it. `encoding` is the builtin's own read of that
    /// handle's tagged producing-server encoding
    /// (`JsonHandle::position_encoding`); every position in `edit` is
    /// counted in it. A `WorkspaceEdit` can touch many files at once; each is
    /// edited through `pane`'s pane while that pane shows it and for no pane
    /// otherwise, as `apply_text_edits` does. Returns the number of buffers
    /// modified.
    ///
    /// `expect_gen`, like `apply_text_edits`'s own, is the *requesting*
    /// pane's buffer generation at the time the request that produced
    /// `edit` was sent, checked against that buffer's current generation
    /// before any file's changeset is built, so an edit computed against
    /// text that has since changed fails loudly instead of either silently
    /// vanishing (a request-side staleness drop, before this is ever
    /// called) or applying against the wrong text.
    fn apply_workspace_edit(
        &mut self,
        pane: PaneHandle,
        edit: &serde_json::Value,
        encoding: PositionEncoding,
        expect_gen: Option<u64>,
    ) -> Result<usize, String>;

    /// `(goto-location! pane target)`, raw `Location`/`LocationLink`
    /// hashmap/handle shape, `loc` decoded through
    /// `hume_lsp::location::decode_location`, the same decoder
    /// `lsp-locations->places` uses for drawer rows. `encoding` is
    /// the builtin's own read of `loc`'s tagged producing-server encoding;
    /// see [`Self::apply_workspace_edit`]'s doc for the same reasoning.
    /// `Ok(false)` when the location's line no longer exists in its file:
    /// nothing moves, and a buffer this call opened for it is closed again.
    fn goto_location_value(
        &mut self,
        pane: PaneHandle,
        loc: &serde_json::Value,
        encoding: PositionEncoding,
    ) -> Result<bool, String>;

    /// `(goto-location! pane target)`, `(hash 'target 'line 'char-col)` shape
    /// with a path string target, already char-indexed. `line` and `char_col`
    /// are bare `usize`: the file may not be open, so no rope exists to check
    /// `line` against before the host opens one, and the host mints it with
    /// `ContentLine::checked` against that text. `Ok(false)` when `line` is
    /// past the text's last content line, with the same cleanup as
    /// [`Self::goto_location_value`]; `char_col` clamps to its line and is the
    /// sanctioned addressing-unit exception, same as everywhere else on this
    /// trait.
    fn goto_location_path(
        &mut self,
        pane: PaneHandle,
        path: String,
        line: usize,
        char_col: usize,
    ) -> Result<bool, String>;

    /// `(goto-location! pane target)`, `(hash 'target 'char-offset)` shape
    /// with a buffer target. `char_offset` is a buffer position at the host
    /// seam, so a bare `usize`. `Err` when it is past the buffer's end:
    /// Scheme computed it for a live buffer.
    fn goto_location_buffer(
        &mut self,
        pane: PaneHandle,
        target: BufferId,
        char_offset: usize,
    ) -> Result<(), String>;

    /// `(goto-revision! pane revision)`: moves `pane`'s buffer to revision
    /// number `revision` of its undo history, across branches, as one undo
    /// walk. `revision` is a raw number from `(buffer-undo-tree pane)`,
    /// validated here. `Err` for a number the buffer has no revision for
    /// (never recorded, or trimmed away), or for a read-only buffer, so a
    /// refusal is never mistaken for a jump that happened.
    fn goto_revision(&mut self, pane: PaneHandle, revision: usize) -> Result<(), String>;

    /// `(insert-key! pane key)`: runs `key`'s Insert-mode default
    /// behaviour (tab-style-aware Tab, auto-pairs, auto-indented Enter, …)
    /// against `pane`, without walking the Insert keymap. Kind-A: `pane`
    /// must be the focused pane, since this acts on live Insert-session
    /// bookkeeping (the open undo group, autoindent ownership, a live
    /// completion session), which is inherently focus-bound rather than a
    /// property of an arbitrary background pane. `Err` when `pane` isn't
    /// the focused pane, the focused mode isn't Insert, the call isn't from
    /// inside a command an Insert-mode key's own binding is dispatching
    /// (a hook or timer calling this instead would edit untracked for dot-
    /// repeat), or `key` has no default Insert-mode behaviour to run
    /// (an arrow, Esc, an unhandled Ctrl-chord, …).
    ///
    /// Exists so a command bound to an Insert-mode key can fall back to
    /// that key's normal behaviour after deciding it doesn't want to
    /// override it (e.g. Tab: complete after a letter, else insert a tab).
    /// There is otherwise no way to hand a key back to "as if unbound"
    /// once a binding has claimed it.
    fn insert_key(&mut self, pane: PaneHandle, key: KeyEvent) -> Result<(), String>;

    /// `(insert-text! pane text)`: pastes `text` into whatever has focus,
    /// through the same path a terminal paste takes, so it does whatever a
    /// paste of `text` would do there. `Err` only when `pane` isn't the
    /// focused pane.
    fn insert_text(&mut self, pane: PaneHandle, text: String) -> Result<(), String>;
}
