//! `EditorHostImpl`'s LSP server introspection.

use hume_engine::pipeline::BufferId;

use super::EditorHostImpl;
use hume_lsp::backend::ServerId;
use hume_scripting::host::{
    HostToken, LocationDisplay, LspHost, PositionParams, RangeParams, RangesParams,
};
use hume_scripting::{CapabilityQuery, LspFeature, PaneHandle, ServerRef};

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

    fn lsp_server_registered(&self, name: &hume_scripting::ServerName) -> bool {
        crate::editor::lsp::introspect::server_registered(&self.state.lsp, name)
    }

    fn lsp_language_servers(&self, language: &str) -> Vec<hume_scripting::ListEntry> {
        crate::editor::lsp::introspect::language_servers(&self.state.lsp, language)
    }

    fn lsp_position_params(&self, pane: PaneHandle) -> Result<Option<PositionParams>, String> {
        let t = self.command_pane(pane)?;
        Ok(params::position_params(self.state, self.view, t))
    }

    fn track_position(&mut self, pane: PaneHandle) -> Result<HostToken, String> {
        let t = self.command_pane(pane)?;
        let bid = t.bid(self.view);
        let text = self.state.buffers.get(bid).text().clone();
        let head = t
            .state(&self.state.panes.state, self.view)
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

    fn lsp_primary_range_params(&self, pane: PaneHandle) -> Result<Option<RangeParams>, String> {
        let t = self.command_pane(pane)?;
        Ok(params::primary_range_params(self.state, self.view, t))
    }

    fn lsp_linewise_ranges_params(&self, pane: PaneHandle) -> Result<Option<RangesParams>, String> {
        let t = self.command_pane(pane)?;
        Ok(params::linewise_ranges_params(self.state, self.view, t))
    }

    fn lsp_wire_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Option<usize> {
        crate::editor::lsp::introspect::wire_to_char_for_buffer(self.state, id, pos, encoding)
            .map(|offset| offset.index())
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

    fn lsp_label_offsets_to_text(
        &self,
        label: &str,
        start: usize,
        end: usize,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> String {
        crate::editor::lsp::introspect::label_slice(label, start, end, encoding)
    }

    fn lsp_locations_display_parts(
        &self,
        locs: &[hume_scripting::json::JsonHandle],
    ) -> Result<Vec<LocationDisplay>, String> {
        crate::editor::lsp::introspect::location_display_parts(self.state, locs)
    }
}
