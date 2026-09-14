//! LSP-driven text edits, workspace edits, and go-to-location.

use hume_engine::pipeline::BufferId;

/// One `apply-text-edits!` entry: a wire range plus its replacement text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTextEdit {
    pub range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
    pub new_text: String,
}

/// LSP-driven text edits, workspace edits, and go-to-location — accessed
/// through [`EditorHost::edits`](super::EditorHost::edits).
pub trait EditHost {
    /// `(apply-text-edits! bid edits #:expect-generation gen)` — `edits` is
    /// a list of wire-coordinate ranges plus replacement text. Applied as
    /// one undo step.
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;

    /// `(apply-workspace-edit! edit)` — `edit` is a decoded LSP
    /// `WorkspaceEdit` JSON blob. Returns the number of buffers modified.
    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String>;

    /// `(goto-location! target)`, raw `Location`/`LocationLink` hashmap
    /// shape — `loc` decoded through `hume_lsp::location::decode_location`,
    /// the same decoder `lsp-locations->display-parts` uses for drawer rows.
    fn goto_location_value(&mut self, loc: serde_json::Value) -> Result<(), String>;

    /// `(goto-location! target)`, `(list target line char-col)` shape with a
    /// path or `file://` URI string target — already char-indexed. `line` is
    /// minted trusted, unvalidated, by the one builtin (`goto-location!`)
    /// that calls this — there is no rope to validate against for a path
    /// target that names no open buffer; `char_col` stays the sanctioned
    /// bare-`usize` addressing-unit exception, same as everywhere else on
    /// this trait.
    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String>;

    /// `(goto-location! target)`, `(list target line char-col)` shape with a
    /// `bid` target — already char-indexed. See [`Self::goto_location_path`]
    /// for `line`'s trusted-mint rationale.
    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String>;
}
