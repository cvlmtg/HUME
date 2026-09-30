//! LSP server introspection.

use std::sync::Arc;

use hume_engine::pipeline::BufferId;
use hume_rope::position_encoding::PositionEncoding;

use super::token::HostToken;
use crate::types::PaneHandle;

/// LSP server introspection, accessed through [`EditorHost::lsp`](super::EditorHost::lsp).
pub trait LspHost {
    /// The wire `ServerCapabilities` for `bid`'s attached server, or `None` if
    /// `bid` has none attached, or the server hasn't finished its handshake
    /// yet. `Arc`-wrapped so the `JsonHandle` `(lsp-capabilities …)` hands
    /// Steel shares this allocation instead of a value rebuilt per call.
    fn lsp_capabilities(&self, bid: BufferId) -> Option<Arc<serde_json::Value>>;

    /// One entry per running (language, root) server.
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry>;

    /// The registered language for the server attached to buffer `id`, or
    /// `None` if `id` is unknown or has no attached server.
    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String>;

    /// Whether `language` currently has a `register-lsp-server!` config
    /// (registered, not necessarily attached/running). Used by the
    /// `on-language-set` missing-server hint to distinguish "not installed"
    /// from "still starting". Reports state *as of the last completed
    /// drain*: the `lsp-registered-for-language?` builtin overlays this
    /// with the `Effect::LspServerOp` entries queued this eval/init before
    /// falling back here, so same-eval visibility is handled at the builtin
    /// layer, not this trait method.
    fn lsp_registered_for_language(&self, language: &str) -> bool;

    /// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
    /// params for the primary cursor head in `pane`'s own pane, in its
    /// buffer's attached server's negotiated encoding. `Ok(None)` if the
    /// buffer has no path or no attached server. `Err` (kind-B fail-fast)
    /// when `pane` carries no pane, a closed one, or one that no longer
    /// shows `pane`'s buffer.
    fn lsp_position_params(&self, pane: PaneHandle) -> Result<Option<serde_json::Value>, String>;

    /// `(track-position! pane)`: remembers the primary cursor head in
    /// `pane`'s own pane, carried through every edit, and returns the token
    /// naming it. `Err` (kind-B fail-fast) when `pane` carries no pane, a
    /// closed one, or one that no longer shows `pane`'s buffer.
    fn track_position(&mut self, pane: PaneHandle) -> Result<HostToken, String>;

    /// `(tracked-position-params token)`: the
    /// [`lsp_position_params`](Self::lsp_position_params) shape for where the
    /// tracked position is now. `None` for a released or unknown token, a
    /// closed buffer or one whose text was replaced, or a buffer with no
    /// path or no attached server; never an error.
    fn tracked_position_params(&self, token: HostToken) -> Option<serde_json::Value>;

    /// `(untrack-position! token)`: releases the position. A released or
    /// unknown token is a no-op.
    fn untrack_position(&mut self, token: HostToken);

    /// `(keep-tracked-position! token)`: keeps a position an `lsp-request!`
    /// holds through `#:tracked` past its callback. A released or unknown
    /// token is a no-op.
    fn keep_tracked_position(&mut self, token: HostToken);

    /// Same as [`lsp_position_params`](Self::lsp_position_params) but a
    /// `{"textDocument" {"uri"} "range" {"start" "end"}}` shape from the
    /// primary selection alone.
    fn lsp_primary_range_params(
        &self,
        pane: PaneHandle,
    ) -> Result<Option<serde_json::Value>, String>;

    /// `{"textDocument" {"uri"} "ranges" [...]}`: one wire range per
    /// *linewise* selection in `pane`'s own pane, coalescing any run of
    /// selections that touch end-to-end into a single range (an LSP range
    /// is naturally contiguous, so a touching run needs no splitting). A
    /// non-linewise selection is skipped, not an error: `ranges` is simply
    /// empty when none of the selections are linewise. `Ok(None)` (as
    /// opposed to an empty `ranges`) only for the same reason
    /// `lsp_primary_range_params` returns `Ok(None)`: no path or no attached
    /// server. `Err` per [`lsp_position_params`](Self::lsp_position_params).
    fn lsp_linewise_ranges_params(
        &self,
        pane: PaneHandle,
    ) -> Result<Option<serde_json::Value>, String>;

    /// A [`WirePos`](hume_rope::position_encoding::WirePos) → char offset,
    /// decoded in `encoding`. Backs `lsp-range->offsets`. `encoding` comes
    /// from the caller's own tagged `JsonHandle` (the response the position
    /// was read out of), not from `id`'s *currently* attached server: the
    /// two can diverge (a restart renegotiates, a detach leaves none), and
    /// the tag is always the encoding the position was actually written in.
    /// `None` if `id` is unknown. Clamps rather than refuses an out-of-range
    /// `line`/`character` (a range's `end` can legitimately land at the
    /// buffer's char length); point-anchored callers want
    /// [`lsp_wire_point_to_char`](Self::lsp_wire_point_to_char).
    fn lsp_wire_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
        encoding: PositionEncoding,
    ) -> Option<usize>;

    /// Same conversion as [`lsp_wire_to_char`](Self::lsp_wire_to_char), but
    /// backs `lsp-position->offset` specifically: refuses (`None`) rather
    /// than clamping when the wire position would land on the buffer's
    /// trailing phantom line, since every point-anchored decoration setter
    /// (`set-inlay-hints!`) rejects that offset outright. See
    /// `wire_point_to_char_for_buffer`'s doc for why the two must differ.
    fn lsp_wire_point_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
        encoding: PositionEncoding,
    ) -> Option<usize>;

    /// Backs `(lsp-label-offsets->text label offsets)`: the `[start, end)`
    /// slice of `label` named by a `ParameterInformation.label` wire offset
    /// pair, in `encoding` (the builtin's own read of `offsets`'s tagged
    /// producing-server encoding; `offsets` is itself drawn from the same
    /// response `label` came from, via `json-ref`).
    ///
    /// `label` is server-authored display text (a
    /// `SignatureInformation.label`), never document text, so no buffer
    /// holds it and the other converters here don't fit. This needs no
    /// `BufferId` at all, only the encoding.
    fn lsp_label_offsets_to_text(
        &self,
        label: &str,
        start: usize,
        end: usize,
        encoding: PositionEncoding,
    ) -> String;

    /// `(lsp-locations->display-parts locs)`: the filesystem path, wire
    /// line, and column of each raw `Location`/`LocationLink` handle in
    /// `locs`, decoded through the same `hume_lsp::location::decode_location`
    /// `goto-location!` uses. Backs `lsp/location-display`'s drawer rows:
    /// `goto-location!` already converts wire positions correctly for the
    /// *jump*; this is the display-side counterpart.
    ///
    /// The column is an exact grapheme column when the target has an open
    /// buffer, `None` when it's an open buffer whose line is out of range,
    /// and otherwise the location's own wire `character` verbatim. See
    /// `LocationDisplay`'s `grapheme_col_or_wire` field doc for why that last case
    /// is the one sanctioned exception to "never render a wire unit
    /// directly".
    ///
    /// The path and line ride along with the column because all three come
    /// from one decode: reading `range.start.line` a second time in Scheme
    /// to render the row prefix is how a row ends up naming a position that
    /// doesn't match the one its column was read from. Each entry's own
    /// `JsonHandle` carries its own producing-server encoding (same
    /// rationale `GotoTarget::Wire` uses for the actual jump), so a batch
    /// built from locations spanning more than one response decodes each
    /// correctly rather than assuming they share one server.
    ///
    /// `Err` (aborting the whole batch, not just one row) only for a
    /// location whose shape can't be decoded at all (missing `uri`/`range`,
    /// unparseable URI, or untagged): such a location names no destination
    /// `goto-location!` could reach either, so a drawer row for it would be
    /// unselectable. Degrading only this builtin wouldn't
    /// help either: the same malformed location would still abort three
    /// lines later inside `lsp/location-display`, which is why both routes
    /// decode through the one shared `decode_location` instead of
    /// tolerating a bad shape here. See `decode_location`'s doc for the
    /// full rule.
    fn lsp_locations_display_parts(
        &self,
        locs: &[crate::json::JsonHandle],
    ) -> Result<Vec<LocationDisplay>, String>;
}

/// One `lsp-locations->display-parts` result row. See
/// [`LspHost::lsp_locations_display_parts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationDisplay {
    /// The URI's decoded display path (`hume_lsp::uri::uri_to_display_string`).
    pub path: String,
    /// 0-based wire line, straight from the location's `range.start.line`.
    pub line: usize,
    /// 0-based column: a grapheme column when the location's target has an
    /// open buffer, `None` when it does and `line` is out of its range,
    /// otherwise the location's own wire `character` verbatim (the display
    /// companion never reads an unopened target's file to refine this
    /// number; see `location_display_parts`'s doc, `hume-editor`, for the
    /// full reasoning and the resulting unit divergence). Named for both
    /// possible units, not just the common one. See CLAUDE.md's "Line/buffer
    /// columns" invariant's one sanctioned exception.
    pub grapheme_col_or_wire: Option<usize>,
    /// The open buffer the location's target is, or `None` when the file is
    /// not open.
    pub buffer: Option<BufferId>,
}
