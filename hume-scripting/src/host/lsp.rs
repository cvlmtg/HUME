//! LSP server introspection.

use std::sync::Arc;

use hume_core::{BufferId, HostToken};
use hume_rope::column::GraphemeCol;
use hume_rope::position_encoding::PositionEncoding;

use hume_lsp::backend::ServerId;

use super::ShownHandle;
use super::token::TrackedHold;
use crate::builtins::ids::{DocPos, DocRange, DocUri, ServerRef};
use crate::types::CapabilityQuery;
use hume_core::{Claim, LanguageLists, LspFeature, ServerName, ServerRegistrationTie};

/// A position request's params before encoding: the document and a
/// position, both filled in for each server as the request is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionParams {
    pub doc: DocUri,
    pub pos: DocPos,
}

/// [`PositionParams`] for one range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeParams {
    pub doc: DocUri,
    pub range: DocRange,
}

/// [`PositionParams`] for several ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangesParams {
    pub doc: DocUri,
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
    /// running or has not finished its handshake. Shared through `Arc` so
    /// the `JsonHandle` `(lsp-capabilities …)` returns is not rebuilt per
    /// call.
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

    /// Every server attached to `bid`, in any state, in order. `Err` for a
    /// closed buffer.
    fn lsp_servers(&self, bid: BufferId) -> Result<Vec<ServerRef>, String>;

    /// Where a request of `feature` or `method` (at least one given) for
    /// `bid` would go now: the servers it reaches (running, admitted by
    /// their list entry, advertising the capability), or the reason it
    /// reaches none. `Err` for a closed buffer.
    fn lsp_route(
        &self,
        bid: BufferId,
        feature: Option<LspFeature>,
        method: Option<&str>,
    ) -> Result<crate::LspRoute, String>;

    /// One entry per running server instance.
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry>;

    /// The origin of `name`'s registration (not necessarily attached or
    /// running), empty when it is not registered, as of the last applied
    /// effect: the `lsp-server-registered?` builtin applies the ops queued
    /// in its own eval on top of it.
    fn lsp_server_claim(&self, name: &ServerName) -> Claim<ServerRegistrationTie>;

    /// A key no other call has returned, for `lsp-supersede-key`.
    fn new_supersede_key(&mut self) -> SupersedeKey;

    /// `language`'s server lists as they stand, for a reader to replay this
    /// eval's queued list writes onto and plan from
    /// ([`hume_core::LanguageLists::plan`]).
    fn lsp_language_lists(&self, language: &str) -> LanguageLists;

    /// The document and primary cursor head of `pane`'s pane.
    fn lsp_position_params(&self, pane: ShownHandle) -> Result<PositionParams, String>;

    /// `(track-position! pane)`: remembers the primary cursor head in
    /// `pane`'s pane, carried through every edit, and returns the token
    /// naming it.
    fn track_position(&mut self, pane: ShownHandle) -> Result<HostToken, String>;

    /// `(tracked-position-params token)`: the
    /// [`lsp_position_params`](Self::lsp_position_params) shape for where the
    /// tracked position is now. `None` for a released or unknown token, or
    /// a closed buffer or one whose text was replaced; never an error.
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
    fn lsp_primary_range_params(&self, pane: ShownHandle) -> Result<RangeParams, String>;

    /// One range per linewise selection in `pane`'s pane, with a run of
    /// selections that touch end to end coalesced into one range. A
    /// non-linewise selection is skipped, not an error, so `ranges` is empty
    /// when no selection is linewise.
    fn lsp_linewise_ranges_params(&self, pane: ShownHandle) -> Result<RangesParams, String>;

    /// A wire `[start, end)` range → char offsets, decoded in `encoding`, for
    /// `lsp-range->char-range`. `encoding` is the tag of the handle the range
    /// was read from, not `id`'s current server's, which a restart or detach
    /// can change. `None` when an end is on a line past the document end, or
    /// `id` is unknown. A `character` past its line's end clamps, and an end
    /// may sit at the document end `(line count, 0)`; point-anchored callers
    /// want [`lsp_wire_point_to_char`](Self::lsp_wire_point_to_char).
    fn lsp_wire_range_to_chars(
        &self,
        id: BufferId,
        range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
        encoding: PositionEncoding,
    ) -> Option<(usize, usize)>;

    /// A wire position → char offset, decoded in `encoding`, for
    /// `lsp-position->char-offset`. `None` when the position lands on the
    /// buffer's trailing phantom line or past it, an offset every
    /// point-anchored decoration setter (`set-inlay-hints!`) rejects, or when
    /// `id` is unknown.
    fn lsp_wire_point_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
        encoding: PositionEncoding,
    ) -> Option<usize>;

    /// `(lsp-locations->places locs)`: the path, wire line and column of each
    /// `Location`/`LocationLink` handle in `locs`, for `lsp/place-display`'s
    /// drawer rows. Decoded through `hume_lsp::location::decode_location`,
    /// as `goto-location!` decodes the jump, each handle in its own
    /// producing-server encoding, so one batch may mix responses. The column
    /// is a grapheme column when the target has an open buffer and the wire
    /// `character` otherwise (see [`PlaceCol::Wire`]).
    ///
    /// One entry per input location, in order, never merged. A location
    /// `goto-location!` cannot jump to (a non-`file://` URI, or a line past
    /// the content of the file's open buffer) is [`LocationPlace::Skipped`].
    /// Path, line and column come from one decode, so a row's prefix always
    /// names the position its column was read from.
    ///
    /// `Err`, aborting the whole batch, only for a location that cannot be
    /// decoded at all (missing `uri`/`range`, a malformed `file://` URI, or
    /// untagged): it names no destination, and `lsp/place-display` would
    /// abort on it too. The error does not name a builtin; the calling
    /// builtin adds its own name.
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

/// A [`Place`]'s 0-based column. A grapheme column always names the open
/// buffer it was measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaceCol {
    /// The grapheme column measured on the open buffer's current text.
    Grapheme { buffer: BufferId, col: GraphemeCol },
    /// The location's own wire `character`, shown verbatim for a target with
    /// no open buffer rather than reading the file to convert it. The only
    /// wire unit the editor displays.
    Wire { character: usize },
}

/// One `lsp-locations->places` result entry. See
/// [`LspHost::lsp_location_places`].
#[derive(Debug, Clone)]
pub struct Place {
    /// The URI's decoded display path (`hume_lsp::uri::uri_to_display_string`).
    pub path: String,
    /// 0-based wire line, straight from the location's `range.start.line`.
    pub line: usize,
    /// The column, in the unit its variant names.
    pub col: PlaceCol,
    /// The location the row was decoded from, for `goto-location!`.
    pub location: crate::json::JsonHandle,
}
