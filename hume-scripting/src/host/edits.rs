//! LSP-driven text edits, workspace edits, and go-to-location.

use hume_engine::pipeline::BufferId;
use hume_rope::position_encoding::PositionEncoding;

use crate::types::PaneHandle;

/// One `apply-text-edits!` entry: a wire range plus its replacement text.
/// `encoding` is this entry's own producing server's negotiated encoding —
/// each entry decodes from its own `JsonHandle`, so a batch built from more
/// than one response could in principle disagree; the host implementation
/// checks every entry in a batch agrees before applying any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextEdit {
    pub range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
    pub new_text: String,
    pub encoding: PositionEncoding,
}

/// LSP-driven text edits, workspace edits, and go-to-location — accessed
/// through [`EditorHost::edits`](super::EditorHost::edits).
///
/// Every method here is kind-B: it acts through `pane`'s own pane (mapping
/// selections through the edit, moving the cursor on goto), not the focused
/// one — a response landing after the user has since switched panes must
/// still edit or navigate the pane the request was actually made from. `Err`
/// (fail-fast) when `pane` carries no pane, a closed one, or one that no
/// longer shows its buffer.
pub trait EditHost {
    /// `(apply-text-edits! pane edits #:expect-generation gen)` — `edits` is
    /// a list of wire-coordinate ranges plus replacement text. Applied as
    /// one undo step.
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;

    /// `(apply-workspace-edit! pane edit)` — `edit` is an LSP `WorkspaceEdit`
    /// wire JSON blob, read-only here (deserialized into a typed
    /// `WorkspaceEdit` without needing ownership), so the builtin passes a
    /// borrow of its `JsonHandle`/hashmap argument rather than cloning it.
    /// `encoding` is the builtin's own read of that handle's tagged
    /// producing-server encoding (`JsonHandle::position_encoding`) — every
    /// position in `edit` is counted in it. A `WorkspaceEdit` can touch many
    /// files at once, each mapped through `pane`'s own pane regardless of
    /// which buffer it edits — the same "seed a selection entry for a
    /// buffer this pane doesn't show" fallback `apply_text_edits` uses,
    /// applied per file. Returns the number of buffers modified.
    fn apply_workspace_edit(
        &mut self,
        pane: PaneHandle,
        edit: &serde_json::Value,
        encoding: PositionEncoding,
    ) -> Result<usize, String>;

    /// `(goto-location! pane target)`, raw `Location`/`LocationLink`
    /// hashmap/handle shape — `loc` decoded through
    /// `hume_lsp::location::decode_location`, the same decoder
    /// `lsp-locations->display-parts` uses for drawer rows. `encoding` is
    /// the builtin's own read of `loc`'s tagged producing-server encoding —
    /// see [`Self::apply_workspace_edit`]'s doc for the same reasoning.
    fn goto_location_value(
        &mut self,
        pane: PaneHandle,
        loc: &serde_json::Value,
        encoding: PositionEncoding,
    ) -> Result<(), String>;

    /// `(goto-location! pane target)`, `(list target line char-col)` shape
    /// with a path or `file://` URI string target — already char-indexed.
    /// `line` is minted trusted, unvalidated, by the one builtin
    /// (`goto-location!`) that calls this — there is no rope to validate
    /// against for a path target that names no open buffer; `char_col`
    /// stays the sanctioned bare-`usize` addressing-unit exception, same as
    /// everywhere else on this trait.
    fn goto_location_path(
        &mut self,
        pane: PaneHandle,
        path_or_uri: String,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String>;

    /// `(goto-location! pane target)`, `(list target line char-col)` shape
    /// with a buffer target — already char-indexed. See
    /// [`Self::goto_location_path`] for `line`'s trusted-mint rationale.
    fn goto_location_buffer(
        &mut self,
        pane: PaneHandle,
        target: BufferId,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String>;
}
