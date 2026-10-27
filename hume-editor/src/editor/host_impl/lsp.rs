//! `EditorHostImpl`'s LSP server introspection.

use hume_core::{BufferId, HostToken, LspFeature, PaneHandle};

use super::EditorHostImpl;
use hume_lsp::backend::ServerId;
use hume_scripting::host::{
    LocationPlace, LspHost, PositionParams, RangeParams, RangesParams, TrackedHold,
};
use hume_scripting::{CapabilityQuery, ServerRef};

use crate::editor::lsp::{introspect, params};

impl<'a> LspHost for EditorHostImpl<'a> {
    fn lsp_capabilities(&self, server: ServerId) -> Option<std::sync::Arc<serde_json::Value>> {
        introspect::capabilities(&self.state.lsp, server)
    }

    fn lsp_capability(
        &self,
        server: ServerId,
        query: CapabilityQuery<'_>,
    ) -> Option<std::sync::Arc<serde_json::Value>> {
        introspect::capability(&self.state.lsp, server, query)
    }

    fn lsp_method_feature(&self, method: &str) -> Option<LspFeature> {
        introspect::method_feature(method)
    }

    fn lsp_servers(
        &self,
        bid: BufferId,
        feature: Option<LspFeature>,
        method: Option<&str>,
    ) -> Result<Vec<ServerRef>, String> {
        introspect::servers(self.state, bid, feature, method)
    }

    fn lsp_server_status(&self) -> Vec<hume_scripting::LspServerStatusEntry> {
        crate::editor::lsp::introspect::server_status(&self.state.lsp)
    }

    fn lsp_server_claim(
        &self,
        name: &hume_core::ServerName,
    ) -> hume_core::Claim<hume_core::ServerRegistrationTie> {
        crate::editor::lsp::introspect::server_claim(&self.state.lsp, name)
    }

    fn new_supersede_key(&mut self) -> hume_scripting::host::SupersedeKey {
        self.state.lsp.new_supersede_key()
    }

    fn lsp_language_lists(&self, language: &str) -> hume_core::LanguageLists {
        crate::editor::lsp::introspect::language_lists(&self.state.lsp, language)
    }

    fn lsp_position_params(&self, pane: PaneHandle) -> Result<Option<PositionParams>, String> {
        let t = self.command_pane(pane)?;
        Ok(params::position_params(self.state, self.view, t))
    }

    fn lsp_document_uri(&self, id: BufferId) -> Option<String> {
        params::document_uri(self.state, id)
    }

    fn track_position(&mut self, pane: PaneHandle) -> Result<HostToken, String> {
        let t = self.command_pane(pane)?;
        let bid = t.bid(self.view);
        let text = self.state.buffers.get(bid).text().clone();
        let head = t
            .state(&self.state.panes.entries, self.view)
            .view(&text)
            .primary()
            .head();
        Ok(self.state.panes.tracked.track(bid, &text, head))
    }

    fn tracked_position_params(&self, token: HostToken) -> Option<PositionParams> {
        let (bid, offset) = self
            .state
            .panes
            .tracked
            .position(token, &self.state.buffers)?;
        params::offset_params(self.state, bid, offset)
    }

    fn untrack_position(&mut self, token: HostToken) {
        self.state.panes.tracked.untrack(token);
    }

    fn keep_tracked_position(&mut self, token: HostToken) {
        self.state.panes.tracked.keep(token);
    }

    fn hold_tracked_position(&self, token: HostToken) -> Option<TrackedHold> {
        self.state.panes.tracked.hold(token)
    }

    fn lsp_primary_range_params(&self, pane: PaneHandle) -> Result<Option<RangeParams>, String> {
        let t = self.command_pane(pane)?;
        Ok(params::primary_range_params(self.state, self.view, t))
    }

    fn lsp_linewise_ranges_params(&self, pane: PaneHandle) -> Result<Option<RangesParams>, String> {
        let t = self.command_pane(pane)?;
        Ok(params::linewise_ranges_params(self.state, self.view, t))
    }

    fn lsp_wire_range_to_chars(
        &self,
        id: BufferId,
        range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Option<(usize, usize)> {
        crate::editor::lsp::introspect::wire_range_to_chars_for_buffer(
            self.state, id, range, encoding,
        )
        .map(|range| (range.start.index(), range.end.index()))
    }

    fn lsp_wire_point_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Option<usize> {
        crate::editor::lsp::introspect::wire_point_to_char_for_buffer(self.state, id, pos, encoding)
            .map(|offset| offset.index())
    }

    fn lsp_location_places(
        &self,
        locs: &[(
            hume_scripting::json::JsonHandle,
            hume_rope::position_encoding::PositionEncoding,
        )],
    ) -> Result<Vec<LocationPlace>, String> {
        crate::editor::lsp::introspect::location_places(self.state, locs)
    }
}
