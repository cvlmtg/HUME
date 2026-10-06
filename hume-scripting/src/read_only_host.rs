//! [`ReadOnlyHost`]: the editor as seen by a Steel proc that may read it but
//! must not change it (a drawer's row renderer, run between `settle` and the
//! frame it renders into).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;

use crate::attribution::EntryId;
use crate::host::{
    BufferHost, CommandHost, CursorHost, EditorHost, EventHost, LanguageHost, OptionValue,
    SelectionInfo, SettingsHost, UndoNode,
};
use crate::types::{GrammarReg, PaneHandle, SteelCmdDef, SteelTypedCmdDef};

/// Passes every read of the six required capabilities through to the
/// wrapped host, refuses every write, and offers none of the optional
/// capabilities, so their builtins raise "not supported by this host".
///
/// A refused write returns `Err` where its signature allows. Either way it is
/// recorded, and [`Self::refused`] reports the first one, so a proc that
/// catches the error still fails. The reads take `&self` while the wrapped
/// host's accessors take `&mut self`, hence the `RefCell`; reads never nest.
pub(crate) struct ReadOnlyHost<'h> {
    inner: RefCell<&'h mut dyn EditorHost>,
    refused: Cell<Option<&'static str>>,
}

impl<'h> ReadOnlyHost<'h> {
    pub(crate) fn new(inner: &'h mut dyn EditorHost) -> Self {
        Self {
            inner: RefCell::new(inner),
            refused: Cell::new(None),
        }
    }

    /// The first write the proc attempted, as an error message.
    pub(crate) fn refused(&self) -> Option<String> {
        self.refused.get().map(refusal)
    }

    fn refuse(&self, name: &'static str) -> String {
        if self.refused.get().is_none() {
            self.refused.set(Some(name));
        }
        refusal(name)
    }
}

fn refusal(name: &str) -> String {
    format!("{name}: the editor cannot be changed while rendering")
}

impl EditorHost for ReadOnlyHost<'_> {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        self
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        self
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        self
    }
    fn events(&mut self) -> &mut dyn EventHost {
        self
    }
}

impl CursorHost for ReadOnlyHost<'_> {
    fn buffer_cursor_line(&self, pane: PaneHandle) -> Result<usize, String> {
        self.inner.borrow_mut().cursor().buffer_cursor_line(pane)
    }
    fn buffer_selections(&self, pane: PaneHandle) -> Result<Vec<SelectionInfo>, String> {
        self.inner.borrow_mut().cursor().buffer_selections(pane)
    }
    fn offset_to_line(&self, bid: BufferId, idx: usize) -> Option<usize> {
        self.inner.borrow_mut().cursor().offset_to_line(bid, idx)
    }
    fn symbol_under_cursor(&self, pane: PaneHandle) -> Result<String, String> {
        self.inner.borrow_mut().cursor().symbol_under_cursor(pane)
    }
    fn selections_linewise(&self, pane: PaneHandle) -> Result<bool, String> {
        self.inner.borrow_mut().cursor().selections_linewise(pane)
    }
    fn selections_charwise(&self, pane: PaneHandle) -> Result<bool, String> {
        self.inner.borrow_mut().cursor().selections_charwise(pane)
    }
}

impl CommandHost for ReadOnlyHost<'_> {
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.borrow_mut().commands().command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        _name: &str,
        _pane: PaneHandle,
        _count: Option<usize>,
        _extend: bool,
        _register: Option<char>,
    ) -> Result<bool, String> {
        Err(self.refuse("run_command_sync"))
    }
    fn register_command(&mut self, _def: SteelCmdDef) -> Result<(), String> {
        Err(self.refuse("register_command"))
    }
    fn register_typed_command(&mut self, _def: SteelTypedCmdDef) -> Result<(), String> {
        Err(self.refuse("register_typed_command"))
    }
    fn unregister_command(&mut self, _name: &str) {
        self.refuse("unregister_command");
    }
    fn is_valid_register_name(&self, ch: char) -> bool {
        self.inner
            .borrow_mut()
            .commands()
            .is_valid_register_name(ch)
    }
    fn register_lazy_command(&mut self, _name: &str, _plugin: &EntryId) -> Result<(), String> {
        Err(self.refuse("register_lazy_command"))
    }
    fn register_lazy_typed_command(
        &mut self,
        _name: &str,
        _plugin: &EntryId,
    ) -> Result<(), String> {
        Err(self.refuse("register_lazy_typed_command"))
    }
    fn lazy_command_owner(&self, name: &str) -> Option<EntryId> {
        self.inner.borrow_mut().commands().lazy_command_owner(name)
    }
    fn lazy_mappable_command_owner(&self, name: &str) -> Option<EntryId> {
        self.inner
            .borrow_mut()
            .commands()
            .lazy_mappable_command_owner(name)
    }
    fn unregister_lazy_stubs_of(&mut self, _plugin: &EntryId) {
        self.refuse("unregister_lazy_stubs_of");
    }
}

impl LanguageHost for ReadOnlyHost<'_> {
    fn attach_grammar(&mut self, _reg: &GrammarReg) -> Result<(), String> {
        Err(self.refuse("attach_grammar"))
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.inner.borrow_mut().language().has_grammar(language)
    }
}

impl SettingsHost for ReadOnlyHost<'_> {
    fn set_global_option(&mut self, _key: &str, _value: &str) -> Result<(), String> {
        Err(self.refuse("set_global_option"))
    }
    fn set_buffer_option(
        &mut self,
        _key: &str,
        _value: &str,
        _bid: BufferId,
    ) -> Result<(), String> {
        Err(self.refuse("set_buffer_option"))
    }
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        self.inner.borrow_mut().settings().get_global_option(key)
    }
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        self.inner
            .borrow_mut()
            .settings()
            .get_buffer_option(key, bid)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        self.inner.borrow_mut().settings().steel_command_budget_ms()
    }
}

impl BufferHost for ReadOnlyHost<'_> {
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.inner.borrow_mut().buffers().buffer_ids()
    }
    fn panes(&self) -> Vec<PaneHandle> {
        self.inner.borrow_mut().buffers().panes()
    }
    fn focused_pane(&self) -> PaneHandle {
        self.inner.borrow_mut().buffers().focused_pane()
    }
    fn buffer_panes(&self, pane: PaneHandle) -> Vec<PaneHandle> {
        self.inner.borrow_mut().buffers().buffer_panes(pane)
    }
    fn require_focused_pane(&self, pane: PaneHandle) -> Result<(), String> {
        self.inner.borrow_mut().buffers().require_focused_pane(pane)
    }
    fn pane_live(&self, pane: PaneHandle) -> bool {
        self.inner.borrow_mut().buffers().pane_live(pane)
    }
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.inner.borrow_mut().buffers().buffer_exists(id)
    }
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf> {
        self.inner.borrow_mut().buffers().buffer_path(id)
    }
    fn buffer_display_path(&self, id: BufferId) -> Option<String> {
        self.inner.borrow_mut().buffers().buffer_display_path(id)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.inner.borrow_mut().buffers().buffer_display_name(id)
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.inner.borrow_mut().buffers().buffer_is_dirty(id)
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        self.inner.borrow_mut().buffers().buffer_stored_language(id)
    }
    fn open_buffer(&mut self, _path: &Path) -> Result<BufferId, String> {
        Err(self.refuse("open_buffer"))
    }
    fn close_buffer(&mut self, _id: BufferId) -> Result<(), String> {
        Err(self.refuse("close_buffer"))
    }
    fn switch_to_buffer(&mut self, _pane: PaneHandle, _target: BufferId) -> Result<(), String> {
        Err(self.refuse("switch_to_buffer"))
    }
    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        self.inner.borrow_mut().buffers().buffer_generation(id)
    }
    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<UndoNode>> {
        self.inner.borrow_mut().buffers().buffer_undo_tree(id)
    }
    fn buffer_text(&self, id: BufferId) -> Option<String> {
        self.inner.borrow_mut().buffers().buffer_text(id)
    }
    fn buffer_line_count(&self, id: BufferId) -> Option<usize> {
        self.inner.borrow_mut().buffers().buffer_line_count(id)
    }
    fn buffer_lines(
        &self,
        id: BufferId,
        range: ExclusiveRange<ContentLine>,
    ) -> Option<Vec<String>> {
        self.inner.borrow_mut().buffers().buffer_lines(id, range)
    }
    fn line_to_offset(&self, id: BufferId, line: ContentLine) -> Option<usize> {
        self.inner.borrow_mut().buffers().line_to_offset(id, line)
    }
    fn viewport_range(&self, pane: PaneHandle) -> Result<ExclusiveRange<ContentLine>, String> {
        self.inner.borrow_mut().buffers().viewport_range(pane)
    }
}

impl EventHost for ReadOnlyHost<'_> {
    fn known_event_names(&self) -> &'static [&'static str] {
        self.inner.borrow_mut().events().known_event_names()
    }
}
