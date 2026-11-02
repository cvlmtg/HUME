//! LSP-driven text edits, workspace edits and go-to-location, plus
//! `goto-revision!`, `insert-key!`, `paste-text!` and `edit-text!`. They
//! share one accessor (`EditorHost::edits`), which gates every
//! buffer-mutating builtin that isn't a named editor command.

use hume_core::{BufferId, PaneHandle};
use hume_platform::path::AbsolutePath;
use hume_rope::position_encoding::PositionEncoding;
use termina::event::KeyEvent;

use super::{FocusedHandle, ShownHandle};

/// One `apply-text-edits!` entry: a wire range plus its replacement text.
/// `encoding` is the producing server's negotiated encoding; entries from
/// different responses may disagree, so the host checks that every entry in
/// a batch agrees before applying any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextEdit {
    pub range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
    pub new_text: String,
    pub encoding: PositionEncoding,
}

/// One `edit-text!` entry: the chars `start..end` of the buffer as it was
/// before the call, replaced by `text`. Plain offsets, converted to
/// `CharOffset` and checked against the buffer by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharTextEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// LSP-driven text edits, workspace edits and go-to-location, accessed
/// through [`EditorHost::edits`](super::EditorHost::edits).
///
/// A method acts through `pane`'s pane, not the focused one, so a response
/// that lands after the user switched panes still reaches the pane the
/// request came from. [`Self::apply_text_edits`],
/// [`Self::apply_workspace_edit`] and [`Self::edit_text`] take any `pane`
/// and edit through it while it shows the edited buffer, through no pane
/// otherwise.
pub trait EditHost {
    /// `(apply-text-edits! pane edits #:expect-generation gen)` applies wire
    /// range edits as one undo step. Editing through no pane carries every
    /// pane's selections and is refused while any session is open on the
    /// buffer.
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;

    /// `(apply-workspace-edit! pane edit #:expect-generation gen)`: `edit` is
    /// an LSP `WorkspaceEdit` wire JSON value, borrowed from the builtin's
    /// argument. Every position in it is counted in `encoding`, the
    /// argument's tagged producing-server encoding. Each file is edited as
    /// [`Self::apply_text_edits`] edits its buffer. Reports and returns the
    /// number of buffers modified.
    ///
    /// `expect_gen` is the requesting pane's buffer generation when the
    /// request was sent, checked before any file's changeset is built, so an
    /// edit computed against since-changed text fails with an error instead
    /// of applying to the wrong text.
    fn apply_workspace_edit(
        &mut self,
        pane: PaneHandle,
        edit: &serde_json::Value,
        encoding: PositionEncoding,
        expect_gen: Option<u64>,
    ) -> Result<usize, String>;

    /// `(goto-location! pane target)` for a raw `Location`/`LocationLink`,
    /// decoded through `hume_lsp::location::decode_location` as
    /// `lsp-locations->places` decodes it. `encoding` is `loc`'s tagged
    /// producing-server encoding. `Ok(Some(handle))` names the pane and the
    /// buffer it now shows; `Ok(None)` when the location's line no longer
    /// exists in its file: nothing moves, and a buffer this call opened for
    /// it is closed again.
    fn goto_location_value(
        &mut self,
        pane: ShownHandle,
        loc: &serde_json::Value,
        encoding: PositionEncoding,
    ) -> Result<Option<PaneHandle>, String>;

    /// `(goto-location! pane target)` for a `(hash 'target 'line 'char-col)`
    /// path target, made absolute against the working directory, already
    /// char-indexed. `line` is a bare `usize` because the file may not be
    /// open; the host checks it with `ContentLine::checked` against the text
    /// it opens. `Ok(None)` when `line` is past the last content line, with
    /// [`Self::goto_location_value`]'s cleanup. `char_col` clamps to its line.
    fn goto_location_path(
        &mut self,
        pane: ShownHandle,
        path: AbsolutePath,
        line: usize,
        char_col: usize,
    ) -> Result<Option<PaneHandle>, String>;

    /// `(goto-location! pane target)` for a `(hash 'target 'char-offset)`
    /// buffer target. `Err` when `char_offset` is past the buffer's end:
    /// Scheme computed it for a live buffer.
    fn goto_location_buffer(
        &mut self,
        pane: ShownHandle,
        target: BufferId,
        char_offset: usize,
    ) -> Result<PaneHandle, String>;

    /// `(goto-revision! pane revision)`: moves `pane`'s buffer to revision
    /// `revision` of its undo history, across branches, as one undo walk.
    /// `revision` comes from `(buffer-undo-tree pane)` and is validated here.
    /// `Err` for a revision the buffer lacks (never recorded, or trimmed
    /// away) or a read-only buffer, so a refusal never reads as a jump.
    fn goto_revision(&mut self, pane: ShownHandle, revision: usize) -> Result<(), String>;

    /// `(insert-key! pane key)`: runs `key`'s Insert-mode default behaviour
    /// (indent-style-aware Tab, auto-pairs, auto-indented Enter, …) on
    /// `pane` without walking the Insert keymap, so a command bound to an
    /// Insert-mode key can hand the key back as if unbound. `pane` must be
    /// focused: the Insert session's undo group, autoindent ownership and
    /// completion session belong to the focused pane. `Err` when the focused
    /// mode isn't Insert, when not called from a command an Insert-mode key's
    /// binding dispatches (a hook or timer would edit untracked by
    /// dot-repeat), or when `key` has no default Insert-mode behaviour (an
    /// arrow, Esc, an unhandled Ctrl-chord, …).
    fn insert_key(&mut self, pane: FocusedHandle, key: KeyEvent) -> Result<(), String>;

    /// `(paste-text! pane text)`: pastes `values` into whatever has focus,
    /// through the path a terminal paste takes. Several values are placed at
    /// the selections as `p` places a register's. `Err` when the paste is
    /// refused (its buffer is read-only, or busy with a pipe).
    fn paste_text(&mut self, pane: FocusedHandle, values: Vec<String>) -> Result<(), String>;

    /// `(edit-text! pane edits #:expect-generation gen)`: applies `edits`,
    /// all positioned against the text before the call, to `pane`'s buffer
    /// as one undo step, carrying every selection through the change. An
    /// empty list changes nothing and is checked for nothing. Otherwise
    /// `Err` for an offset past the end, a reversed or overlapping range, a
    /// read-only buffer, or a buffer whose generation is not `expect_gen`.
    fn edit_text(
        &mut self,
        pane: PaneHandle,
        edits: Vec<CharTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;
}
