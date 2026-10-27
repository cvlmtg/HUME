//! LSP server introspection.

use std::sync::Arc;

use hume_core::{BufferId, ConfigOrigin};
use hume_rope::position_encoding::PositionEncoding;

use hume_lsp::backend::ServerId;

use super::token::{HostToken, TrackedHold};
use crate::builtins::ids::{DocPos, DocRange, ServerRef};
use crate::types::{CapabilityQuery, LspFeature, PaneHandle};

/// A position request's params before encoding: the document URI and a
/// position each server receives in its own encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionParams {
    pub uri: String,
    pub pos: DocPos,
}

/// [`PositionParams`] for one range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeParams {
    pub uri: String,
    pub range: DocRange,
}

/// [`PositionParams`] for several ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangesParams {
    pub uri: String,
    pub ranges: Vec<DocRange>,
}

/// A `#:supersede` key, as Steel holds it: an opaque value only
/// `lsp-supersede-key` mints, so two callers never share a key unless one
/// hands its key to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SupersedeKey(u64);

impl SupersedeKey {
    /// A key with id `raw`, for a test that needs a known one.
    #[cfg(test)]
    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The id its [`SupersedeKeys`] minted.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Mints [`SupersedeKey`]s for a host: each one no earlier call returned.
#[derive(Debug, Default)]
pub struct SupersedeKeys(u64);

impl SupersedeKeys {
    pub fn mint(&mut self) -> SupersedeKey {
        self.0 += 1;
        SupersedeKey(self.0)
    }
}

/// LSP server introspection, accessed through [`EditorHost::lsp`](super::EditorHost::lsp).
pub trait LspHost {
    /// `server`'s wire `ServerCapabilities`, or `None` if it is no longer
    /// running or has not finished its handshake. `Arc`-wrapped so the
    /// `JsonHandle` `(lsp-capabilities …)` hands Steel shares this
    /// allocation instead of a value rebuilt per call.
    fn lsp_capabilities(&self, server: ServerId) -> Option<Arc<serde_json::Value>>;

    /// What `server` advertises for `feature`, or for the one capability
    /// `method` needs: the provider's wire value (`true` or an options
    /// object). `None` when it advertises none, the method belongs to no
    /// feature, or `server` is not running.
    fn lsp_capability(
        &self,
        server: ServerId,
        query: CapabilityQuery<'_>,
    ) -> Option<Arc<serde_json::Value>>;

    /// The feature a standard request `method` belongs to (`textDocument/hover`
    /// is `hover`'s), or `None` for any other method.
    fn lsp_method_feature(&self, method: &str) -> Option<LspFeature>;

    /// The servers attached to `bid`, in order. With neither `feature` nor
    /// `method`, every attached server in any state; otherwise the servers
    /// a request of that feature or method would go to now (running,
    /// admitted by their list entry, advertising the capability). `Err` for
    /// a closed buffer.
    fn lsp_servers(
        &self,
        bid: BufferId,
        feature: Option<LspFeature>,
        method: Option<&str>,
    ) -> Result<Vec<ServerRef>, String>;

    /// One entry per running server instance.
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry>;

    /// The origin of `name`'s registration (not necessarily attached or
    /// running), `None` when it is not registered, as of the last applied
    /// effect: the `lsp-server-registered?` builtin applies the ops queued
    /// in its own eval on top of it.
    fn lsp_server_origin(&self, name: &crate::types::ServerName) -> Option<ConfigOrigin>;

    /// A key no other call has returned, for `lsp-supersede-key`.
    fn new_supersede_key(&mut self) -> SupersedeKey;

    /// The servers `language`'s buffers attach to, in order, each with the
    /// feature filter its list entry gives it.
    fn lsp_language_servers(&self, language: &str) -> Vec<crate::types::ListEntry>;

    /// The document URI and primary cursor head of `pane`'s own pane.
    /// `Ok(None)` if the buffer has no path. `Err` (kind-B fail-fast) when
    /// `pane` carries no pane, a closed one, or one that no longer shows
    /// `pane`'s buffer.
    fn lsp_position_params(&self, pane: PaneHandle) -> Result<Option<PositionParams>, String>;

    /// The document URI of buffer `id`. `None` for a closed buffer or one
    /// with no path. Reads only the buffer, so any `PaneHandle` works.
    fn lsp_document_uri(&self, id: BufferId) -> Option<String>;

    /// `(track-position! pane)`: remembers the primary cursor head in
    /// `pane`'s own pane, carried through every edit, and returns the token
    /// naming it. `Err` (kind-B fail-fast) when `pane` carries no pane, a
    /// closed one, or one that no longer shows `pane`'s buffer.
    fn track_position(&mut self, pane: PaneHandle) -> Result<HostToken, String>;

    /// `(tracked-position-params token)`: the
    /// [`lsp_position_params`](Self::lsp_position_params) shape for where the
    /// tracked position is now. `None` for a released or unknown token, a
    /// closed buffer or one whose text was replaced, or a buffer with no
    /// path; never an error.
    fn tracked_position_params(&self, token: HostToken) -> Option<PositionParams>;

    /// `(untrack-position! token)`: releases the position. A released or
    /// unknown token is a no-op.
    fn untrack_position(&mut self, token: HostToken);

    /// `(keep-tracked-position! token)`: keeps a position an `lsp-request!`
    /// holds through `#:tracked` past its callback. A released or unknown
    /// token is a no-op.
    fn keep_tracked_position(&mut self, token: HostToken);

    /// `lsp-request!`'s `#:tracked`: the request's hold on `token`'s
    /// position, taken as the request is queued, so a request discarded
    /// with the code that queued it releases the position too. `None` for a
    /// released or unknown token.
    fn hold_tracked_position(&self, token: HostToken) -> Option<TrackedHold>;

    /// Same as [`lsp_position_params`](Self::lsp_position_params) but the
    /// range the primary selection covers.
    fn lsp_primary_range_params(&self, pane: PaneHandle) -> Result<Option<RangeParams>, String>;

    /// One range per *linewise* selection in `pane`'s own pane, coalescing
    /// any run of selections that touch end-to-end into a single range (an
    /// LSP range is naturally contiguous, so a touching run needs no
    /// splitting). A non-linewise selection is skipped, not an error:
    /// `ranges` is empty when none of the selections are linewise.
    /// `Ok(None)` (as opposed to an empty `ranges`) only for a buffer with
    /// no path. `Err` per [`lsp_position_params`](Self::lsp_position_params).
    fn lsp_linewise_ranges_params(&self, pane: PaneHandle) -> Result<Option<RangesParams>, String>;

    /// A wire `[start, end)` range → char offsets, decoded in `encoding`.
    /// Backs `lsp-range->char-range`. `encoding` comes from the caller's own
    /// tagged `JsonHandle` (the response the range was read out of), not
    /// from `id`'s *currently* attached server: the two can diverge (a
    /// restart renegotiates, a detach leaves none), and the tag is always
    /// the encoding the range was actually written in. `None` when the range
    /// cannot be converted for `id`: an end is on a line past the document
    /// end, which names no offset, or `id` is unknown. A `character` past its
    /// line's end clamps, and an end may sit at the document end
    /// `(line count, 0)`; point-anchored callers want
    /// [`lsp_wire_point_to_char`](Self::lsp_wire_point_to_char).
    fn lsp_wire_range_to_chars(
        &self,
        id: BufferId,
        range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
        encoding: PositionEncoding,
    ) -> Option<(usize, usize)>;

    /// A wire position → char offset, decoded in `encoding`, for
    /// `lsp-position->char-offset`. A point must lie on a content line:
    /// `None` when the position would land on the buffer's trailing phantom
    /// line or past it, since every point-anchored decoration setter
    /// (`set-inlay-hints!`) rejects that offset outright, or when `id` is
    /// unknown. See `wire_point_to_char_for_buffer`'s doc for why a point and
    /// a range differ.
    fn lsp_wire_point_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
        encoding: PositionEncoding,
    ) -> Option<usize>;

    /// `(lsp-locations->places locs)`: the filesystem path, wire
    /// line, and column of each raw `Location`/`LocationLink` handle in
    /// `locs`, decoded through the same `hume_lsp::location::decode_location`
    /// `goto-location!` uses. Backs `lsp/place-display`'s drawer rows:
    /// `goto-location!` already converts wire positions correctly for the
    /// *jump*; this is the display-side counterpart.
    ///
    /// The column is an exact grapheme column when the target has an open
    /// buffer, and otherwise the location's own wire `character` verbatim. See
    /// `Place`'s `grapheme_col_or_wire` field doc for why that last case
    /// is the one sanctioned exception to "never render a wire unit
    /// directly".
    ///
    /// One entry per input location, in order, and entries are not merged:
    /// whether two locations are the same place is the caller's policy. A
    /// location with no place `goto-location!` can jump to (a URI that is
    /// not a `file://` one, or a line past the content of the file's open
    /// buffer) is [`LocationPlace::Skipped`].
    ///
    /// The path and line ride along with the column because all three come
    /// from one decode: reading `range.start.line` a second time in Scheme
    /// to render the row prefix is how a row ends up naming a position that
    /// doesn't match the one its column was read from. Each handle comes
    /// paired with its own producing-server encoding (same rationale
    /// `GotoTarget::Wire` uses for the actual jump), so a batch built from
    /// locations spanning more than one response decodes each correctly
    /// rather than assuming they share one server.
    ///
    /// `Err` (aborting the whole batch, not just one row) only for a
    /// location whose shape can't be decoded at all (missing `uri`/`range`,
    /// a malformed `file://` URI, or untagged): such a location names no
    /// destination `goto-location!` could reach either, so a drawer row for it
    /// would be unselectable. Degrading only this builtin wouldn't
    /// help either: the same malformed location would still abort three
    /// lines later inside `lsp/place-display`, which is why both routes
    /// decode through the one shared `decode_location` instead of
    /// tolerating a bad shape here. See `decode_location`'s doc for the
    /// full rule. The error does not name a builtin: the builtin that called
    /// this adds its own name.
    fn lsp_location_places(
        &self,
        locs: &[(crate::json::JsonHandle, PositionEncoding)],
    ) -> Result<Vec<LocationPlace>, String>;
}

/// One entry of [`LspHost::lsp_location_places`]'s result.
#[derive(Debug, Clone)]
pub enum LocationPlace {
    Found(Place),
    /// The location names no place `goto-location!` can jump to: its URI is
    /// not a `file://` one, or its line is past the content of the file's
    /// open buffer.
    Skipped,
}

/// One `lsp-locations->places` result entry. See
/// [`LspHost::lsp_location_places`].
#[derive(Debug, Clone)]
pub struct Place {
    /// The URI's decoded display path (`hume_lsp::uri::uri_to_display_string`).
    pub path: String,
    /// 0-based wire line, straight from the location's `range.start.line`.
    pub line: usize,
    /// 0-based column: a grapheme column when the location's target has an
    /// open buffer, otherwise the location's own wire `character` verbatim (the display
    /// companion never reads an unopened target's file to refine this
    /// number; see `location_places`'s doc, `hume-editor`, for the
    /// full reasoning and the resulting unit divergence). Named for both
    /// possible units, not just the common one. See CLAUDE.md's "Line/buffer
    /// columns" invariant's one sanctioned exception.
    pub grapheme_col_or_wire: usize,
    /// The open buffer the location's target is, or `None` when the file is
    /// not open.
    pub buffer: Option<BufferId>,
    /// The location the row was decoded from, for `goto-location!`.
    pub location: crate::json::JsonHandle,
}
