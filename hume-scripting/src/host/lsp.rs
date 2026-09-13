//! LSP server introspection — moved out of `host.rs`'s per-capability
//! split.

use hume_engine::pipeline::BufferId;

/// LSP server introspection — accessed through [`EditorHost::lsp`](super::EditorHost::lsp).
pub trait LspHost {
    /// Decoded `ServerCapabilities` for `server` (a registered language name,
    /// or `None` for the focused buffer's attached server) — `None` if
    /// unresolvable or the server hasn't finished its handshake yet.
    fn lsp_capabilities(&self, server: Option<&str>) -> Option<serde_json::Value>;

    /// One entry per running (language, root) server.
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry>;

    /// The registered language for the server attached to buffer `id`, or
    /// `None` if `id` is unknown or has no attached server.
    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String>;

    /// Whether `language` currently has a `register-lsp-server!` config
    /// (registered, not necessarily attached/running) — used by the
    /// `on-language-set` missing-server hint to distinguish "not installed"
    /// from "still starting". Reports state *as of the last completed
    /// drain* — the `lsp-registered-for-language?` builtin overlays this
    /// with the `Effect::LspServerOp` entries queued this eval/init before
    /// falling back here, so same-eval visibility is handled at the builtin
    /// layer, not this trait method.
    fn lsp_registered_for_language(&self, language: &str) -> bool;

    /// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
    /// params for `id`'s primary cursor head, in its attached server's
    /// negotiated encoding — `None` if `id` has no path, no attached server,
    /// or isn't currently shown in any pane.
    fn lsp_position_params(&self, id: BufferId) -> Option<serde_json::Value>;

    /// Same as [`lsp_position_params`](Self::lsp_position_params) but a
    /// `{"textDocument" {"uri"} "range" {"start" "end"}}` shape from the
    /// primary selection alone.
    fn lsp_primary_range_params(&self, id: BufferId) -> Option<serde_json::Value>;

    /// `{"textDocument" {"uri"} "ranges" [...]}` — one wire range per
    /// *linewise* selection in `id`'s buffer, coalescing any run of
    /// selections that touch end-to-end into a single range (an LSP range
    /// is naturally contiguous, so a touching run needs no splitting). A
    /// non-linewise selection is skipped, not an error — `ranges` is simply
    /// empty when none of `id`'s selections are linewise. `None` (as
    /// opposed to an empty `ranges`) only for the same reasons
    /// `lsp_primary_range_params` returns `None`: no path, no attached
    /// server, or the buffer isn't shown in any pane.
    fn lsp_linewise_ranges_params(&self, id: BufferId) -> Option<serde_json::Value>;

    /// A [`WirePos`](hume_rope::position_encoding::WirePos) → char offset in
    /// `id`'s attached server's negotiated encoding — backs
    /// `lsp-range->offsets`. `None` if `id` is unknown or has no attached
    /// server (no negotiated encoding to convert with). Clamps rather than
    /// refuses an out-of-range `line`/`character` (a range's `end` can
    /// legitimately land at the buffer's char length); point-anchored
    /// callers want [`lsp_wire_point_to_char`](Self::lsp_wire_point_to_char).
    fn lsp_wire_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
    ) -> Option<usize>;

    /// Same conversion as [`lsp_wire_to_char`](Self::lsp_wire_to_char), but
    /// backs `lsp-position->offset` specifically: refuses (`None`) rather
    /// than clamping when the wire position would land on the buffer's
    /// trailing phantom line, since every point-anchored decoration setter
    /// (`set-inlay-hints!`) rejects that offset outright — see
    /// `wire_point_to_char_for_buffer`'s doc for why the two must differ.
    fn lsp_wire_point_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
    ) -> Option<usize>;

    /// Backs `(lsp-label-offsets->text bid label offsets)` — the
    /// `[start, end)` slice of `label` named by a
    /// `ParameterInformation.label` wire offset pair (unpacked from that
    /// builtin's `offsets` list), in the encoding `id`'s attached server
    /// negotiated. `None` if `id` is unknown or has no attached server.
    ///
    /// `label` is server-authored display text (a
    /// `SignatureInformation.label`), never document text, so no buffer
    /// holds it and the other converters here don't fit. `id` names the
    /// server, not the text being indexed.
    fn lsp_label_offsets_to_text(
        &self,
        id: BufferId,
        label: &str,
        start: usize,
        end: usize,
    ) -> Option<String>;

    /// `(lsp-locations->display-parts locs)` — the filesystem path, wire
    /// line, and column of each raw `Location`/`LocationLink` hashmap in
    /// `locs`, decoded through the same `hume_lsp::location::decode_location`
    /// `goto-location!` uses. Backs `lsp/location-display`'s drawer rows:
    /// `goto-location!` already converts wire positions correctly for the
    /// *jump*; this is the display-side counterpart.
    ///
    /// The column is an exact grapheme column when the target has an open
    /// buffer, `None` when it's an open buffer whose line is out of range,
    /// and otherwise the location's own wire `character` verbatim — see
    /// `LocationDisplay`'s `grapheme_col_or_wire` field doc for why that last case
    /// is the one sanctioned exception to "never render a wire unit
    /// directly".
    ///
    /// The path and line ride along with the column because all three come
    /// from one decode: reading `range.start.line` a second time in Scheme
    /// to render the row prefix is how a row ends up naming a position that
    /// doesn't match the one its column was read from. Every location shares
    /// the encoding negotiated by the currently focused buffer's attached
    /// server, same as `goto-location!`'s wire shape.
    ///
    /// `Err` — aborting the whole batch, not just one row — only for a
    /// location whose shape can't be decoded at all (missing `uri`/`range`,
    /// unparseable URI): such a location names no destination `goto-location!`
    /// could reach either, so a drawer row for it would be unselectable by
    /// construction. Degrading only this builtin wouldn't help either — the
    /// same malformed location would still abort three lines later inside
    /// `lsp/location-display`, which is why both routes decode through the
    /// one shared `decode_location` instead of tolerating a bad shape here.
    /// See `decode_location`'s doc for the full rule.
    fn lsp_locations_display_parts(
        &self,
        locs: Vec<serde_json::Value>,
    ) -> Result<Vec<LocationDisplay>, String>;
}

/// One `lsp-locations->display-parts` result row — see
/// [`LspHost::lsp_locations_display_parts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationDisplay {
    /// The URI's decoded display path (`hume_lsp::uri::uri_to_display_string`).
    pub path: String,
    /// 0-based wire line, straight from the location's `range.start.line`.
    pub line: usize,
    /// 0-based column — a grapheme column when the location's target has an
    /// open buffer, `None` when it does and `line` is out of its range,
    /// otherwise the location's own wire `character` verbatim (the display
    /// companion never reads an unopened target's file to refine this
    /// number; see `location_display_parts`'s doc, `hume-editor`, for the
    /// full reasoning and the resulting unit divergence). Named for both
    /// possible units, not just the common one — see CLAUDE.md's "Line/buffer
    /// columns" invariant's one sanctioned exception.
    pub grapheme_col_or_wire: Option<usize>,
}
