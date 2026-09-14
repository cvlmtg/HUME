//! `EditorHostImpl`'s LSP server introspection.

use hume_engine::pipeline::BufferId;

use super::EditorHostImpl;
use hume_scripting::host::{LocationDisplay, LspHost};

impl<'a> LspHost for EditorHostImpl<'a> {
    fn lsp_capabilities(&self, server: Option<&str>) -> Option<serde_json::Value> {
        let lsp = self.lsp.as_deref()?;
        let bid = crate::editor::commands::focused_buffer_id(self.state, self.view);
        crate::editor::lsp::introspect::capabilities(self.state, lsp, bid, server)
    }

    fn lsp_server_status(&self) -> Vec<hume_scripting::LspServerStatusEntry> {
        self.lsp
            .as_deref()
            .map(crate::editor::lsp::introspect::server_status)
            .unwrap_or_default()
    }

    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String> {
        crate::editor::lsp::introspect::server_for_buffer(self.state, self.lsp.as_deref()?, id)
    }

    fn lsp_registered_for_language(&self, language: &str) -> bool {
        self.lsp.as_deref().is_some_and(|lsp| {
            crate::editor::lsp::introspect::registered_for_language(lsp, language)
        })
    }

    fn lsp_position_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::position_params(
            self.state,
            self.view,
            self.lsp.as_deref()?,
            id,
        )
    }

    fn lsp_primary_range_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::primary_range_params(
            self.state,
            self.view,
            self.lsp.as_deref()?,
            id,
        )
    }

    fn lsp_linewise_ranges_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::linewise_ranges_params(
            self.state,
            self.view,
            self.lsp.as_deref()?,
            id,
        )
    }

    fn lsp_wire_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
    ) -> Option<usize> {
        crate::editor::lsp::introspect::wire_to_char_for_buffer(
            self.state,
            self.lsp.as_deref()?,
            id,
            pos,
        )
        .map(|offset| offset.index())
    }

    fn lsp_wire_point_to_char(
        &self,
        id: BufferId,
        pos: hume_rope::position_encoding::WirePos,
    ) -> Option<usize> {
        crate::editor::lsp::introspect::wire_point_to_char_for_buffer(
            self.state,
            self.lsp.as_deref()?,
            id,
            pos,
        )
        .map(|offset| offset.index())
    }

    fn lsp_label_offsets_to_text(
        &self,
        id: BufferId,
        label: &str,
        start: usize,
        end: usize,
    ) -> Option<String> {
        crate::editor::lsp::introspect::label_slice_for_buffer(
            self.state,
            self.lsp.as_deref()?,
            id,
            label,
            start,
            end,
        )
    }

    fn lsp_locations_display_parts(
        &self,
        locs: Vec<serde_json::Value>,
    ) -> Result<Vec<LocationDisplay>, String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("lsp-locations->display-parts: no LSP state available".to_string());
        };
        let bid = crate::editor::commands::focused_buffer_id(self.state, self.view);
        crate::editor::lsp::introspect::location_display_parts(self.state, lsp, bid, &locs)
    }
}
