//! The optional capabilities [`ReadOnlyHost`] offers: each read passes
//! through to the wrapped host, each write is refused.

use std::sync::Arc;

use hume_core::{
    AnswerEnd, BufferId, Claim, DiagSeverity, HostToken, LanguageLists, LspFeature, NonEmpty,
    PaneHandle, PopupLayout, ServerName, ServerRegistrationTie, Side, TruncateEnd,
};
use hume_lsp::backend::ServerId;
use hume_platform::path::AbsolutePath;
use hume_rope::line::ContentLine;
use hume_rope::position_encoding::PositionEncoding;
use steel::rvals::SteelVal;
use termina::event::KeyEvent;

use super::{ReadOnlyHost, offered};
use crate::host::{
    CharTextEdit, CompletionHost, DecorationHost, DiagnosticCounts, DiagnosticEntry,
    DiagnosticRange, DrawerItems, EditHost, EmittedWord, FocusedHandle, LivePickerOpts,
    LocationPlace, LspHost, MarkedLines, PasteAudience, PasteQuestion, PickerBinding,
    PickerFeedMode, PickerOpts, PickerSourceOpts, PickerStep, PopupKind, PositionParams,
    RangeParams, RangesParams, RegisterHost, RowDisplay, SetError, ShownHandle, SupersedeKey,
    TrackedHold, UiHost, WireTextEdit,
};
use crate::json::JsonHandle;
use crate::types::{CapabilityQuery, ConcealOverlaps, ConcealSpec, VirtualLineSpec};
use crate::{Callback, Rooted};

impl ReadOnlyHost<'_> {
    fn refuse_set(&self) -> SetError {
        SetError::Call(self.refuse())
    }
}

impl RegisterHost for ReadOnlyHost<'_> {
    fn read_register(&mut self, name: char) -> Option<Vec<String>> {
        offered(self.inner.get_mut().registers()).read_register(name)
    }
    fn write_register(&mut self, _name: char, _values: Vec<String>) {
        self.refuse();
    }
}

impl CompletionHost for ReadOnlyHost<'_> {
    fn completion_emit(
        &mut self,
        _id: HostToken,
        _parts: Vec<JsonHandle>,
        _incomplete: bool,
        _end: AnswerEnd,
        _word: EmittedWord,
    ) -> Result<bool, String> {
        Err(self.refuse())
    }
    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        offered(self.inner.borrow_mut().completions()).completion_top(n)
    }
    fn completion_accept(&mut self, _idx: usize) -> Result<(), String> {
        Err(self.refuse())
    }
    fn completion_dismiss(&mut self) -> Result<(), String> {
        Err(self.refuse())
    }
}

impl DecorationHost for ReadOnlyHost<'_> {
    fn set_inlay_hints(
        &mut self,
        _source: String,
        _bid: BufferId,
        _hints: Vec<(usize, String, Side)>,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn register_sign_source(
        &mut self,
        _source: String,
        _bid: BufferId,
        _priority: i64,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn unregister_sign_source(&mut self, _source: &str, _bid: BufferId) {
        self.refuse();
    }
    fn set_signs(
        &mut self,
        _source: String,
        _bid: BufferId,
        _signs: Vec<(MarkedLines, String, String)>,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn set_virtual_lines(
        &mut self,
        _source: String,
        _bid: BufferId,
        _lines: Vec<(ContentLine, VirtualLineSpec)>,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn set_extra_highlights(
        &mut self,
        _source: String,
        _bid: BufferId,
        _spans: crate::ScopedSpans,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn set_conceals(
        &mut self,
        _source: String,
        _bid: BufferId,
        _entries: Vec<ConcealSpec>,
        _overlaps: ConcealOverlaps,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn set_eol_text(
        &mut self,
        _source: String,
        _bid: BufferId,
        _lines: Vec<(ContentLine, String, String)>,
        _hide_on_insert_line: bool,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn set_line_backgrounds(
        &mut self,
        _source: String,
        _bid: BufferId,
        _entries: Vec<(MarkedLines, String)>,
    ) -> Result<(), SetError> {
        Err(self.refuse_set())
    }
    fn set_statusline_text(
        &mut self,
        _source: String,
        _bid: BufferId,
        _text: String,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn diagnostics_for_buffer(
        &mut self,
        bid: BufferId,
        severity_floor: Option<DiagSeverity>,
        range: Option<DiagnosticRange>,
    ) -> Result<Vec<DiagnosticEntry>, String> {
        offered(self.inner.get_mut().decorations()).diagnostics_for_buffer(
            bid,
            severity_floor,
            range,
        )
    }
    fn diagnostic_counts(&self, bid: BufferId, floor: Option<DiagSeverity>) -> DiagnosticCounts {
        offered(self.inner.borrow_mut().decorations()).diagnostic_counts(bid, floor)
    }
}

// A supersede key does nothing until a request sent with it is queued, and
// a request's hold on a tracked position is released with its request. A
// read-only scope queues no request, so neither changes what another call
// sees.
impl LspHost for ReadOnlyHost<'_> {
    fn lsp_capabilities(&self, server: ServerId) -> Option<Arc<serde_json::Value>> {
        offered(self.inner.borrow_mut().lsp()).lsp_capabilities(server)
    }
    fn lsp_capability(
        &self,
        server: ServerId,
        query: CapabilityQuery<'_>,
    ) -> Option<Arc<serde_json::Value>> {
        offered(self.inner.borrow_mut().lsp()).lsp_capability(server, query)
    }
    fn lsp_method_feature(&self, method: &str) -> Option<LspFeature> {
        offered(self.inner.borrow_mut().lsp()).lsp_method_feature(method)
    }
    fn lsp_servers(&self, bid: BufferId) -> Result<Vec<crate::builtins::ids::ServerRef>, String> {
        offered(self.inner.borrow_mut().lsp()).lsp_servers(bid)
    }
    fn lsp_route(
        &self,
        bid: BufferId,
        feature: Option<LspFeature>,
        method: Option<&str>,
    ) -> Result<crate::LspRoute, String> {
        offered(self.inner.borrow_mut().lsp()).lsp_route(bid, feature, method)
    }
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry> {
        offered(self.inner.borrow_mut().lsp()).lsp_server_status()
    }
    fn lsp_server_claim(&self, name: &ServerName) -> Claim<ServerRegistrationTie> {
        offered(self.inner.borrow_mut().lsp()).lsp_server_claim(name)
    }
    fn new_supersede_key(&mut self) -> SupersedeKey {
        offered(self.inner.get_mut().lsp()).new_supersede_key()
    }
    fn lsp_language_lists(&self, language: &str) -> LanguageLists {
        offered(self.inner.borrow_mut().lsp()).lsp_language_lists(language)
    }
    fn lsp_position_params(&self, pane: ShownHandle) -> Result<PositionParams, String> {
        offered(self.inner.borrow_mut().lsp()).lsp_position_params(pane)
    }
    fn track_position(&mut self, _pane: ShownHandle) -> Result<HostToken, String> {
        Err(self.refuse())
    }
    fn tracked_position_params(&self, token: HostToken) -> Option<PositionParams> {
        offered(self.inner.borrow_mut().lsp()).tracked_position_params(token)
    }
    fn untrack_position(&mut self, _token: HostToken) {
        self.refuse();
    }
    fn keep_tracked_position(&mut self, _token: HostToken) {
        self.refuse();
    }
    fn hold_tracked_position(&self, token: HostToken) -> Option<TrackedHold> {
        offered(self.inner.borrow_mut().lsp()).hold_tracked_position(token)
    }
    fn lsp_primary_range_params(&self, pane: ShownHandle) -> Result<RangeParams, String> {
        offered(self.inner.borrow_mut().lsp()).lsp_primary_range_params(pane)
    }
    fn lsp_linewise_ranges_params(&self, pane: ShownHandle) -> Result<RangesParams, String> {
        offered(self.inner.borrow_mut().lsp()).lsp_linewise_ranges_params(pane)
    }
    fn lsp_location_places(
        &self,
        locs: &[(JsonHandle, PositionEncoding)],
    ) -> Result<Vec<LocationPlace>, String> {
        offered(self.inner.borrow_mut().lsp()).lsp_location_places(locs)
    }
}

impl UiHost for ReadOnlyHost<'_> {
    fn prompt(
        &mut self,
        _pane: FocusedHandle,
        _label: String,
        _prefill: String,
        _callback: Callback,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn on_next_char(&mut self, _callback: Callback) -> Result<(), String> {
        Err(self.refuse())
    }
    fn show_popup(
        &mut self,
        _pane: FocusedHandle,
        _text: String,
        _kind: PopupKind,
        _layout: PopupLayout,
        _lang: Option<String>,
    ) -> Result<Option<HostToken>, String> {
        Err(self.refuse())
    }
    fn close_popup(&mut self, _token: HostToken) -> Result<(), String> {
        Err(self.refuse())
    }
    fn show_menu(
        &mut self,
        _pane: FocusedHandle,
        _items: NonEmpty<(RowDisplay, Rooted)>,
        _callback: Callback,
    ) -> Result<Option<HostToken>, SetError> {
        Err(self.refuse_set())
    }
    fn close_menu(&mut self, _token: HostToken) -> Result<(), String> {
        Err(self.refuse())
    }
    fn show_drawer_list(
        &mut self,
        _pane: FocusedHandle,
        _items: DrawerItems,
        _callback: Callback,
        _selected: usize,
        _truncate: TruncateEnd,
    ) -> Result<Option<HostToken>, SetError> {
        Err(self.refuse_set())
    }
    fn close_drawer(&mut self, _token: HostToken) -> Result<(), String> {
        Err(self.refuse())
    }
    fn update_drawer_list(
        &mut self,
        _token: HostToken,
        _items: DrawerItems,
        _callback: Callback,
        _selected: usize,
    ) -> Result<bool, SetError> {
        Err(self.refuse_set())
    }
    fn drawer_selection(&self, token: HostToken) -> Option<(SteelVal, usize)> {
        offered(self.inner.borrow_mut().ui()).drawer_selection(token)
    }
    fn open_picker(
        &mut self,
        _pane: FocusedHandle,
        _items: Vec<(String, Rooted)>,
        _keymap: Vec<PickerBinding>,
        _opts: PickerOpts,
    ) -> Result<Option<HostToken>, String> {
        Err(self.refuse())
    }
    fn open_live_picker(
        &mut self,
        _pane: FocusedHandle,
        _keymap: Vec<PickerBinding>,
        _opts: LivePickerOpts,
    ) -> Result<Option<HostToken>, String> {
        Err(self.refuse())
    }
    fn picker_feed(
        &mut self,
        _token: HostToken,
        _items: Vec<(String, Rooted)>,
        _mode: PickerFeedMode,
    ) -> bool {
        self.refuse();
        false
    }
    fn picker_source_spawn(
        &mut self,
        _token: HostToken,
        _cmd: &str,
        _args: Vec<String>,
        _opts: PickerSourceOpts,
    ) -> Result<bool, String> {
        Err(self.refuse())
    }
    fn picker_source_stop(&mut self, _token: HostToken) -> bool {
        self.refuse();
        false
    }
    fn picker_close(&mut self, _token: HostToken) {
        self.refuse();
    }
    fn picker_move(&mut self, _token: HostToken, _count: isize, _step: PickerStep) {
        self.refuse();
    }
}

impl EditHost for ReadOnlyHost<'_> {
    fn apply_text_edits(
        &mut self,
        _pane: PaneHandle,
        _edits: Vec<WireTextEdit>,
        _expect_gen: Option<u64>,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn apply_workspace_edit(
        &mut self,
        _pane: PaneHandle,
        _edit: &serde_json::Value,
        _encoding: PositionEncoding,
        _expect_gen: Option<u64>,
    ) -> Result<usize, String> {
        Err(self.refuse())
    }
    fn goto_location_value(
        &mut self,
        _pane: ShownHandle,
        _loc: &serde_json::Value,
        _encoding: PositionEncoding,
    ) -> Result<Option<PaneHandle>, String> {
        Err(self.refuse())
    }
    fn goto_location_path(
        &mut self,
        _pane: ShownHandle,
        _path: AbsolutePath,
        _line: usize,
        _char_col: usize,
    ) -> Result<Option<PaneHandle>, String> {
        Err(self.refuse())
    }
    fn goto_location_buffer(
        &mut self,
        _pane: ShownHandle,
        _target: BufferId,
        _char_offset: usize,
    ) -> Result<PaneHandle, String> {
        Err(self.refuse())
    }
    fn goto_revision(&mut self, _pane: ShownHandle, _revision: usize) -> Result<(), String> {
        Err(self.refuse())
    }
    fn insert_key(&mut self, _pane: FocusedHandle, _key: KeyEvent) -> Result<(), String> {
        Err(self.refuse())
    }
    fn paste_text(&mut self, _pane: FocusedHandle, _values: Vec<String>) -> Result<(), String> {
        Err(self.refuse())
    }
    fn start_paste_finish(&mut self) -> bool {
        self.refuse();
        false
    }
    fn paste_audience(&mut self) -> Option<PasteAudience> {
        offered(self.inner.get_mut().edits()).paste_audience()
    }
    fn paste_question(&mut self) -> Option<PasteQuestion> {
        offered(self.inner.get_mut().edits()).paste_question()
    }
    fn apply_paste(&mut self, _answers: Vec<Option<String>>) -> Option<Result<(), String>> {
        Some(Err(self.refuse()))
    }
    fn edit_text(
        &mut self,
        _pane: PaneHandle,
        _edits: Vec<CharTextEdit>,
        _expect_gen: Option<u64>,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
}
