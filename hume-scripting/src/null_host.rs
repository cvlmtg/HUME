//! [`NullHost`]: a minimal [`EditorHost`] for scripting crate unit tests.
//!
//! Does not depend on any editor types.  Read methods return empty / default
//! values; **all mutators return `Err`**, so any test that accidentally drives
//! a mutating builtin through NullHost fails loudly instead of silently succeeding.
//!
//! Suitable only for guard tests (init-guard, activation-state, budget, register
//! validation) where the host mutators are never reached.  Tests that need working
//! mutations (set-option!, attach-grammar!, …) must use `MockHost` in the editor
//! crate.
//!
//! [`InlineOutputHost`], [`RecordingInlineOutputHost`], [`CommandStoreHost`]
//! and [`EmptyLspHost`] each embed a real `NullHost` and delegate every
//! capability accessor to it, overriding only the one or two
//! accessors/methods that make them distinct.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;
use hume_rope::line::ContentLine;

use crate::attribution::EntryId;
use crate::command_name::fold;
use crate::command_procs::CommandProc;
use crate::host::{
    BufferHost, CommandHost, CursorHost, DisplayPathForm, EditorHost, EventHost, HostToken,
    LanguageHost, LocationPlace, LspHost, OptionInput, OptionValue, OutputHost, PositionParams,
    RangeParams, RangesParams, SelectionInfo, SettingsHost, TrackedHold,
};
use crate::types::{GrammarReg, PaneHandle, SteelCmdDef, SteelTypedCmdDef};

/// Event names `NullHost` reports as known: the names scripting-crate unit
/// tests actually register (`on-buffer-open`, `on-buffer-save`,
/// `on-lsp-notification`), plus one synthetic name (`on-stub-only`) the
/// editor never defines. That divergence from the editor's real event set is
/// deliberate: it lets tests prove that `register-hook!`/`declare-plugin!`
/// validate through `EditorHost::events()` rather than a compiled-in table.
const NULL_HOST_EVENT_NAMES: &[&str] = &[
    "on-buffer-open",
    "on-buffer-save",
    "on-lsp-notification",
    "on-stub-only",
    HELD_EVENT,
];

/// The one event `NullHost` reports as waiting for its handlers, so
/// scripting-crate tests cover the `done` handler shape without naming an
/// editor event.
pub(crate) const HELD_EVENT: &str = "on-stub-held";

#[derive(Default)]
pub(crate) struct NullHost;

impl EditorHost for NullHost {
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
    fn permit_plugin_activation(&mut self, _plugin: &EntryId) -> Result<(), String> {
        Ok(())
    }
}

impl EventHost for NullHost {
    fn known_event_names(&self) -> &'static [&'static str] {
        NULL_HOST_EVENT_NAMES
    }
    fn held_event_arg_count(&self, name: &str) -> Option<usize> {
        (name == HELD_EVENT).then_some(1)
    }
    fn finish_hold(&mut self, _hold: HostToken) {}
}

impl BufferHost for NullHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        vec![]
    }
    fn panes(&self) -> Vec<PaneHandle> {
        vec![]
    }
    fn focused_pane(&self) -> PaneHandle {
        PaneHandle::buffer_only(BufferId::default())
    }
    fn buffer_panes(&self, pane: PaneHandle) -> Vec<PaneHandle> {
        vec![pane]
    }
    // Read-like, not a mutator: there's no real focus state to check a
    // `pane` against here, so this permissively passes. The `UiHost`
    // openers this backs have no `NullHost` implementation at all (`ui()`
    // returns `None`), so a test exercising the gate itself needs
    // `EditorHostImpl`/`MockHost`, not this stub.
    fn require_focused_pane(&self, _pane: PaneHandle) -> Result<(), String> {
        Ok(())
    }
    fn pane_live(&self, _pane: PaneHandle) -> bool {
        false
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        false
    }
    fn buffer_path(&self, _id: BufferId) -> Option<PathBuf> {
        None
    }
    fn buffer_display_path(&self, _id: BufferId, _form: DisplayPathForm) -> Option<String> {
        None
    }
    fn buffer_display_name(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn buffer_is_dirty(&self, _id: BufferId) -> Option<bool> {
        None
    }
    fn buffer_stored_language(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn cwd(&self) -> PathBuf {
        PathBuf::from(".")
    }
    fn search_path(&self) -> Option<std::ffi::OsString> {
        None
    }
    fn set_cwd(&mut self, _path: &Path) -> Result<PathBuf, String> {
        Err("NullHost: set_cwd not available".into())
    }
    fn open_buffer(&mut self, _path: &Path) -> Result<BufferId, String> {
        Err("NullHost: open_buffer not available".into())
    }
    fn close_buffer(&mut self, _id: BufferId) -> Result<(), String> {
        Err("NullHost: close_buffer not available".into())
    }
    fn switch_to_buffer(&mut self, _pane: PaneHandle, _target: BufferId) -> Result<(), String> {
        Err("NullHost: switch_to_buffer not available".into())
    }
    fn show_read_only_buffer(
        &mut self,
        _pane: PaneHandle,
        _name: &str,
        _text: &str,
    ) -> Result<(), String> {
        Err("NullHost: show_read_only_buffer not available".into())
    }
    fn buffer_generation(&self, _id: BufferId) -> Option<u64> {
        None
    }
    fn buffer_undo_tree(&self, _id: BufferId) -> Option<Vec<crate::host::UndoNode>> {
        None
    }
    fn buffer_text(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn buffer_line_count(&self, _id: BufferId) -> Option<usize> {
        None
    }
    fn buffer_lines(
        &self,
        _id: BufferId,
        _range: hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
    ) -> Option<Vec<String>> {
        None
    }
    fn line_to_char_offset(
        &self,
        _id: BufferId,
        _line: hume_rope::line::ContentLine,
    ) -> Option<usize> {
        None
    }
    fn viewport_range(
        &mut self,
        _pane: PaneHandle,
    ) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>, String> {
        Err("NullHost: viewport_range not available".into())
    }
}

impl SettingsHost for NullHost {
    fn set_global_option(&mut self, _key: &str, _value: OptionInput) -> Result<(), String> {
        Err("NullHost: set_global_option not available".into())
    }
    fn set_buffer_option(
        &mut self,
        _key: &str,
        _value: OptionInput,
        _bid: BufferId,
    ) -> Result<(), String> {
        Err("NullHost: set_buffer_option not available".into())
    }
    fn get_global_option(&self, _key: &str) -> Result<OptionValue, String> {
        Err("NullHost: get_global_option not available".into())
    }
    fn get_buffer_option(&self, _key: &str, _bid: BufferId) -> Result<OptionValue, String> {
        Err("NullHost: get_buffer_option not available".into())
    }
    fn steel_command_budget_ms(&self) -> u64 {
        10_000
    }
}

impl LanguageHost for NullHost {
    fn attach_grammar(&mut self, _reg: &GrammarReg) -> Result<(), String> {
        Err("NullHost: attach_grammar not available".into())
    }
    fn has_grammar(&self, _language: &str) -> bool {
        false
    }
}

impl CommandHost for NullHost {
    fn is_valid_register_name(&self, _ch: char) -> bool {
        false
    }
    fn command_is_native(&self, _name: &str) -> Result<bool, String> {
        // No registry: treat every command as Steel/forward-raw.
        Ok(false)
    }
    fn run_command_sync(
        &mut self,
        _name: &str,
        _pane: PaneHandle,
        _call: crate::host::NativeCall,
    ) -> Result<bool, String> {
        Err("stub host has no native command registry".into())
    }
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String> {
        Err(format!(
            "NullHost: '{}' rejected, no command registry",
            def.name
        ))
    }
    fn register_typed_command(&mut self, def: SteelTypedCmdDef) -> Result<(), String> {
        Err(format!(
            "NullHost: '{}' rejected, no command registry",
            def.name
        ))
    }
    fn defined_command(&self, _name: &str) -> Option<CommandProc> {
        None
    }
    fn unregister_commands_defined_by(&mut self, _plugin: &EntryId) {}
    fn register_lazy_command(&mut self, _name: &str, _plugin: &EntryId) -> Result<(), String> {
        Ok(())
    }
    fn register_lazy_typed_command(
        &mut self,
        _name: &str,
        _plugin: &EntryId,
    ) -> Result<(), String> {
        Ok(())
    }
    fn lazy_command_owner(&self, _name: &str) -> Option<EntryId> {
        None
    }
    fn lazy_mappable_command_owner(&self, _name: &str) -> Option<EntryId> {
        None
    }
    fn unregister_lazy_stubs_of(&mut self, _plugin: &EntryId) {}
}

impl CursorHost for NullHost {
    fn buffer_cursor_line(&self, _pane: PaneHandle) -> Result<ContentLine, String> {
        Err("NullHost: buffer_cursor_line not available".into())
    }
    fn buffer_selections(&self, _pane: PaneHandle) -> Result<Vec<SelectionInfo>, String> {
        Err("NullHost: buffer_selections not available".into())
    }
    fn char_offset_to_line(&self, _bid: BufferId, _idx: usize) -> Result<ContentLine, String> {
        Err("NullHost: char_offset_to_line not available".into())
    }
    fn symbol_under_cursor(&self, _pane: PaneHandle) -> Result<String, String> {
        Err("NullHost: symbol_under_cursor not available".into())
    }
    fn selections_linewise(&self, _pane: PaneHandle) -> Result<bool, String> {
        Err("NullHost: selections_linewise not available".into())
    }
    fn selections_charwise(&self, _pane: PaneHandle) -> Result<bool, String> {
        Err("NullHost: selections_charwise not available".into())
    }
}

/// Like [`NullHost`] but reports `is_inline_output_command() == true`, and
/// counts calls to `ensure_inline_output_screen`. Lets a test assert a
/// builtin opens the inline-output bracket exactly when (and only when) it
/// has real terminal output to produce, without a real terminal. Exercises
/// the `SteelCtx::new_command` wiring that reads the flag off the host
/// without pulling in the editor crate's real `EditorHostImpl`.
pub(crate) struct RecordingInlineOutputHost {
    inner: NullHost,
    /// Backs `is_inline_output_command`. Defaults to `true` (see the manual
    /// `Default` impl below: `#[derive(Default)]` would default this `false`
    /// and silently close the gate every other test using this host relies
    /// on being open). Set to `false` explicitly by the one test that needs
    /// it isolated from `EvalSession::Init`'s own, independent safety reason.
    pub(crate) inline_output: bool,
    pub(crate) ensure_calls: usize,
}

impl Default for RecordingInlineOutputHost {
    fn default() -> Self {
        Self {
            inner: NullHost,
            inline_output: true,
            ensure_calls: 0,
        }
    }
}

impl EditorHost for RecordingInlineOutputHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        &mut self.inner
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        &mut self.inner
    }
    fn events(&mut self) -> &mut dyn EventHost {
        &mut self.inner
    }
    fn permit_plugin_activation(&mut self, plugin: &EntryId) -> Result<(), String> {
        self.inner.permit_plugin_activation(plugin)
    }
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        Some(self)
    }
}

impl OutputHost for RecordingInlineOutputHost {
    fn is_inline_output_command(&self) -> bool {
        self.inline_output
    }
    fn ensure_inline_output_screen(&mut self) -> Result<(), String> {
        self.ensure_calls += 1;
        Ok(())
    }
    // This host has no command registry to consult (unlike
    // `is_inline_output_command` above, which is a fixed stance for gate
    // tests, not a per-command decision). The `call!`-nesting behavior
    // these two back is exercised against the real `EditorHostImpl`
    // instead (`hume-editor`'s inline-output dispatch tests).
    fn arm_inline_output(&mut self, _name: &str) -> Option<usize> {
        None
    }
    fn truncate_inline_output(&mut self, _depth: usize) {}
}

/// Like [`NullHost`] but stores commands like a minimal `CommandRegistry`:
/// each defined command's proc, and each `Lazy` stub's owning plugin. For
/// tests that define a command and then call it, attribute it, or collide
/// with it, without a real editor.
///
/// Mappable and typed names share one namespace, as in the real registry,
/// so a name defined as one kind collides when re-defined as the other. A
/// define replaces a same-name `Lazy` stub. Names ignore case: both maps
/// are keyed by [`fold`].
#[derive(Default)]
pub(crate) struct CommandStoreHost {
    inner: NullHost,
    defined: rustc_hash::FxHashMap<String, CommandProc>,
    lazy: rustc_hash::FxHashMap<String, EntryId>,
}

impl EditorHost for CommandStoreHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        &mut self.inner
    }
    fn events(&mut self) -> &mut dyn EventHost {
        &mut self.inner
    }
    fn permit_plugin_activation(&mut self, plugin: &EntryId) -> Result<(), String> {
        self.inner.permit_plugin_activation(plugin)
    }
}

impl CommandStoreHost {
    /// Seeds a defined command directly, for a test that needs one owned by
    /// a plugin without evaluating that plugin.
    pub(crate) fn define(&mut self, name: &str, proc: CommandProc) {
        self.register_defined(name, proc)
            .expect("CommandStoreHost::define: name already defined");
    }

    /// The proc stored for `name`, for a test that calls it through a
    /// different host. Panics if `name` was never defined here.
    pub(crate) fn proc(&self, name: &str) -> CommandProc {
        self.defined
            .get(fold(name).as_ref())
            .cloned()
            .unwrap_or_else(|| panic!("CommandStoreHost: no command named '{name}'"))
    }

    /// Shared body behind `register_command`/`register_typed_command`.
    fn register_defined(&mut self, name: &str, proc: CommandProc) -> Result<(), String> {
        let key = fold(name);
        if self.defined.contains_key(key.as_ref()) {
            return Err(format!("'{name}' conflicts with existing command"));
        }
        self.lazy.remove(key.as_ref());
        self.defined.insert(key.into_owned(), proc);
        Ok(())
    }

    /// Shared body behind `register_lazy_command`/`register_lazy_typed_command`.
    fn register_lazy(&mut self, name: &str, plugin: &EntryId) -> Result<(), String> {
        let key = fold(name);
        if self.defined.contains_key(key.as_ref()) {
            return Err(format!("'{name}' conflicts with an existing command"));
        }
        if let Some(owner) = self.lazy.get(key.as_ref()) {
            return if owner == plugin {
                Ok(())
            } else {
                Err(format!("'{name}' already claimed by lazy plugin '{owner}'"))
            };
        }
        self.lazy.insert(key.into_owned(), plugin.clone());
        Ok(())
    }
}

impl CommandHost for CommandStoreHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        self.inner.is_valid_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        pane: PaneHandle,
        call: crate::host::NativeCall,
    ) -> Result<bool, String> {
        self.inner.run_command_sync(name, pane, call)
    }
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String> {
        self.register_defined(&def.name, CommandProc::Mappable(def.body.proc))
    }
    fn register_typed_command(&mut self, def: SteelTypedCmdDef) -> Result<(), String> {
        self.register_defined(&def.name, CommandProc::Typed(def.body.proc))
    }
    fn defined_command(&self, name: &str) -> Option<CommandProc> {
        self.defined.get(fold(name).as_ref()).cloned()
    }
    fn unregister_commands_defined_by(&mut self, plugin: &EntryId) {
        self.defined.retain(|_, proc| !proc.defined_by(plugin));
    }
    fn register_lazy_command(&mut self, name: &str, plugin: &EntryId) -> Result<(), String> {
        self.register_lazy(name, plugin)
    }
    fn register_lazy_typed_command(&mut self, name: &str, plugin: &EntryId) -> Result<(), String> {
        self.register_lazy(name, plugin)
    }
    fn lazy_command_owner(&self, name: &str) -> Option<EntryId> {
        self.lazy.get(fold(name).as_ref()).cloned()
    }
    // `lazy` tracks no kind (see the field doc). This test double can't
    // distinguish a typed-only stub from a mappable one, so it answers the
    // same as `lazy_command_owner`. Fine for every current caller: no
    // `hume-scripting` test dispatches `call!` against a typed-only lazy
    // stub through this host (that scenario needs the real `EditorHostImpl`;
    // see `lazy_mappable_command_owner`'s own doc).
    fn lazy_mappable_command_owner(&self, name: &str) -> Option<EntryId> {
        self.lazy.get(fold(name).as_ref()).cloned()
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &EntryId) {
        self.lazy.retain(|_, p| p != plugin);
    }
}

/// Like [`NullHost`], plus an LSP capability that knows no server, buffer or
/// position: every read answers "nothing", and the two methods that must hand
/// something back (`track_position`, `lsp_location_places`) return `Err`. For
/// tests of a builtin's own decoding and fallbacks once the capability is
/// present.
#[derive(Default)]
pub(crate) struct EmptyLspHost {
    inner: NullHost,
}

impl EditorHost for EmptyLspHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        &mut self.inner
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        &mut self.inner
    }
    fn events(&mut self) -> &mut dyn EventHost {
        &mut self.inner
    }
    fn lsp(&mut self) -> Option<&mut dyn LspHost> {
        Some(self)
    }
    fn permit_plugin_activation(&mut self, plugin: &EntryId) -> Result<(), String> {
        self.inner.permit_plugin_activation(plugin)
    }
}

impl LspHost for EmptyLspHost {
    fn lsp_capabilities(
        &self,
        _server: hume_lsp::backend::ServerId,
    ) -> Option<std::sync::Arc<serde_json::Value>> {
        None
    }
    fn lsp_capability(
        &self,
        _server: hume_lsp::backend::ServerId,
        _query: crate::types::CapabilityQuery<'_>,
    ) -> Option<std::sync::Arc<serde_json::Value>> {
        None
    }
    fn lsp_method_feature(&self, _method: &str) -> Option<crate::types::LspFeature> {
        None
    }
    fn lsp_servers(
        &self,
        _bid: BufferId,
        _feature: Option<crate::types::LspFeature>,
        _method: Option<&str>,
    ) -> Result<Vec<crate::ServerRef>, String> {
        Ok(Vec::new())
    }
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry> {
        Vec::new()
    }
    fn lsp_server_registered(&self, _name: &crate::types::ServerName) -> bool {
        false
    }
    fn lsp_language_servers(&self, _language: &str) -> Vec<crate::types::ListEntry> {
        Vec::new()
    }
    fn lsp_position_params(&self, _pane: PaneHandle) -> Result<Option<PositionParams>, String> {
        Ok(None)
    }
    fn lsp_document_uri(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn track_position(&mut self, _pane: PaneHandle) -> Result<HostToken, String> {
        Err("EmptyLspHost: track_position not available".into())
    }
    fn tracked_position_params(&self, _token: HostToken) -> Option<PositionParams> {
        None
    }
    fn untrack_position(&mut self, _token: HostToken) {}
    fn keep_tracked_position(&mut self, _token: HostToken) {}
    fn hold_tracked_position(&self, _token: HostToken) -> Option<TrackedHold> {
        None
    }
    fn lsp_primary_range_params(&self, _pane: PaneHandle) -> Result<Option<RangeParams>, String> {
        Ok(None)
    }
    fn lsp_linewise_ranges_params(
        &self,
        _pane: PaneHandle,
    ) -> Result<Option<RangesParams>, String> {
        Ok(None)
    }
    fn lsp_wire_range_to_chars(
        &self,
        _id: BufferId,
        _range: hume_rope::offset::ExclusiveRange<hume_rope::position_encoding::WirePos>,
        _encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Option<(usize, usize)> {
        None
    }
    fn lsp_wire_point_to_char(
        &self,
        _id: BufferId,
        _pos: hume_rope::position_encoding::WirePos,
        _encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Option<usize> {
        None
    }
    fn lsp_location_places(
        &self,
        _locs: &[(
            crate::json::JsonHandle,
            hume_rope::position_encoding::PositionEncoding,
        )],
    ) -> Result<Vec<LocationPlace>, String> {
        Err("EmptyLspHost: lsp_location_places not available".into())
    }
}
