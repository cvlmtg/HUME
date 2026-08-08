//! [`NullHost`] — a minimal [`EditorHost`] for scripting crate unit tests.
//!
//! Does not depend on any editor types.  Read methods return empty / default
//! values; **all mutators return `Err`** — so any test that accidentally drives
//! a mutating builtin through NullHost fails loudly instead of silently succeeding.
//!
//! Suitable only for guard tests (init-guard, activation-state, budget, register
//! validation) where the host mutators are never reached.  Tests that need working
//! mutations (set-option!, attach-grammar!, …) must use `MockHost` in the editor
//! crate.
//!
//! [`FailingRegisterHost`], [`InlineOutputHost`], and [`RecordingInlineOutputHost`]
//! each embed a real `NullHost` and delegate every capability accessor to it,
//! overriding only the one or two accessors/methods that make them distinct.

use std::ops::Range;
use std::path::{Path, PathBuf};

use hume_engine::pipeline::{BufferId, PaneId};

use crate::attribution::PluginId;
use crate::host::{
    BufferHost, CommandHost, CursorHost, EditorHost, EventHost, LanguageHost, OptionValue,
    OutputHost, SettingsHost,
};
use crate::types::SteelCmdDef;

/// Event names `NullHost` reports as known — the names scripting-crate unit
/// tests actually register (`on-buffer-open`, `on-buffer-save`), plus one
/// synthetic name (`on-stub-only`) the editor never defines. That divergence
/// from the editor's real event set is deliberate: it's the independent
/// oracle proving `register-hook!`/`declare-plugin` validate through
/// `EditorHost::events()` rather than a compiled-in table.
const NULL_HOST_EVENT_NAMES: &[&str] = &["on-buffer-open", "on-buffer-save", "on-stub-only"];

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
}

impl EventHost for NullHost {
    fn known_event_names(&self) -> &'static [&'static str] {
        NULL_HOST_EVENT_NAMES
    }
}

impl BufferHost for NullHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        vec![]
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        vec![]
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        false
    }
    fn buffer_path(&self, _id: BufferId) -> Option<PathBuf> {
        None
    }
    fn buffer_display_path(&self, _id: BufferId) -> Option<String> {
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
    fn open_buffer(&mut self, _path: &Path) -> Result<BufferId, String> {
        Err("NullHost: open_buffer not available".into())
    }
    fn close_buffer(&mut self, _id: BufferId) -> Result<BufferId, String> {
        Err("NullHost: close_buffer not available".into())
    }
    fn switch_to_buffer(&mut self, _current: BufferId, _target: BufferId) -> Result<(), String> {
        Err("NullHost: switch_to_buffer not available".into())
    }
    fn buffer_generation(&self, _id: BufferId) -> Option<u64> {
        None
    }
    fn buffer_text(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn buffer_line_count(&self, _id: BufferId) -> Option<usize> {
        None
    }
    fn buffer_lines(&self, _id: BufferId, _range: Range<usize>) -> Option<Vec<String>> {
        None
    }
    fn viewport_range(&self, _id: BufferId) -> Option<Range<usize>> {
        None
    }
}

impl SettingsHost for NullHost {
    fn set_global_option(&mut self, _key: &str, _value: &str) -> Result<(), String> {
        Err("NullHost: set_global_option not available".into())
    }
    fn set_buffer_option(
        &mut self,
        _key: &str,
        _value: &str,
        _bid: BufferId,
    ) -> Result<(), String> {
        Err("NullHost: set_buffer_option not available".into())
    }
    fn get_option(&self, _key: &str, _bid: BufferId) -> Result<OptionValue, String> {
        Err("NullHost: get_option not available".into())
    }
    fn configure_statusline(
        &mut self,
        _l: Vec<String>,
        _c: Vec<String>,
        _r: Vec<String>,
    ) -> Result<(), String> {
        Err("NullHost: configure_statusline not available".into())
    }
    fn steel_command_budget_ms(&self) -> u64 {
        10_000
    }
}

impl LanguageHost for NullHost {
    fn attach_grammar(
        &mut self,
        _name: &str,
        _gp: &Path,
        _sym: &str,
        _hl: &Path,
        _inj: Option<&Path>,
    ) -> Result<(), String> {
        Err("NullHost: attach_grammar not available".into())
    }
    fn has_grammar(&self, _language: &str) -> bool {
        false
    }
    fn register_trigger_chars(&mut self, _source: String, _language: String, _chars: Vec<char>) {}
}

impl CommandHost for NullHost {
    fn is_valid_register_name(&self, _ch: char) -> bool {
        false
    }
    fn command_is_native(&self, _name: &str) -> Result<bool, String> {
        // No registry — treat every command as Steel/forward-raw.
        Ok(false)
    }
    fn run_command_sync(
        &mut self,
        _name: &str,
        _count: Option<usize>,
        _extend: bool,
        _register: Option<char>,
    ) -> Result<(), String> {
        Err("stub host has no native command registry".into())
    }
    fn register_command(&mut self, _def: SteelCmdDef) -> Result<(), String> {
        Ok(())
    }
    fn unregister_command(&mut self, _name: &str) {}
    fn register_lazy_command(&mut self, _name: &str, _plugin: &PluginId) -> Result<(), String> {
        Ok(())
    }
    fn lazy_command_owner(&self, _name: &str) -> Option<PluginId> {
        None
    }
    fn unregister_lazy_stubs_of(&mut self, _plugin: &PluginId) {}
}

impl CursorHost for NullHost {
    fn current_line_number(&self) -> Option<usize> {
        None
    }
    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>> {
        None
    }
    fn char_index_to_line(&self, _idx: usize) -> Option<usize> {
        None
    }
    fn symbol_under_cursor(&self, _bid: BufferId) -> String {
        String::new()
    }
    fn selection_spans_full_line(&self, _bid: BufferId) -> bool {
        false
    }
}

/// Like [`NullHost`] but `register_command` fails.
///
/// Exercises the `define-command!` path where the editor-side registry rejects
/// the name (e.g. it shadows a native command): the builtin must propagate the
/// error *without* recording the command in `command_table`/`cmd_owners`.
#[derive(Default)]
pub(crate) struct FailingRegisterHost {
    inner: NullHost,
}

impl EditorHost for FailingRegisterHost {
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
}

impl CommandHost for FailingRegisterHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        self.inner.is_valid_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        self.inner.run_command_sync(name, count, extend, register)
    }
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String> {
        Err(format!(
            "FailingRegisterHost: '{}' rejected by the command registry",
            def.name
        ))
    }
    fn unregister_command(&mut self, name: &str) {
        self.inner.unregister_command(name)
    }
    fn register_lazy_command(&mut self, name: &str, plugin: &PluginId) -> Result<(), String> {
        self.inner.register_lazy_command(name, plugin)
    }
    fn lazy_command_owner(&self, name: &str) -> Option<PluginId> {
        self.inner.lazy_command_owner(name)
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &PluginId) {
        self.inner.unregister_lazy_stubs_of(plugin)
    }
}

/// Like [`NullHost`] but reports `is_inline_output_command() == true`, and
/// counts calls to `ensure_inline_output_screen` — lets a test assert a
/// builtin opens the inline-output bracket exactly when (and only when) it
/// has real terminal output to produce, without a real terminal. Exercises
/// the `SteelCtx::new_command` wiring that reads the flag off the host (see
/// `context.rs` tests) without pulling in the editor crate's real
/// `EditorHostImpl`.
#[derive(Default)]
pub(crate) struct RecordingInlineOutputHost {
    inner: NullHost,
    pub(crate) ensure_calls: usize,
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
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        Some(self)
    }
}

impl OutputHost for RecordingInlineOutputHost {
    fn is_inline_output_command(&self) -> bool {
        true
    }
    fn ensure_inline_output_screen(&mut self) -> Result<(), String> {
        self.ensure_calls += 1;
        Ok(())
    }
}

/// Like [`NullHost`] but tracks command-name claims like a minimal
/// `CommandRegistry`, for tests that exercise `declare-plugin`/`define-command!`
/// collision detection without a real editor.
///
/// Distinguishes two kinds of claim, mirroring the editor's registry:
/// `defined` (a `SteelBacked`/native name — permanent for the test) and `lazy`
/// (a `Lazy` stub's owning plugin, replaceable by that same plugin's own
/// `define-command!`). `NullHost::register_command`'s "always Ok" behavior
/// would make every declare-plugin collision test pass vacuously, so this
/// host is required wherever a test needs `declare-plugin`/`define-command!`
/// to actually collide.
#[derive(Default)]
pub(crate) struct LazyStubHost {
    inner: NullHost,
    defined: rustc_hash::FxHashSet<String>,
    lazy: rustc_hash::FxHashMap<String, PluginId>,
}

impl EditorHost for LazyStubHost {
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
}

impl CommandHost for LazyStubHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        self.inner.is_valid_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        self.inner.run_command_sync(name, count, extend, register)
    }
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String> {
        if self.defined.contains(&def.name) {
            return Err(format!("'{}' conflicts with existing command", def.name));
        }
        // A SteelBacked define overwrites a same-name Lazy stub — mirrors
        // CommandRegistry::register allowing Some(Lazy) | None.
        self.lazy.remove(&def.name);
        self.defined.insert(def.name);
        Ok(())
    }
    fn unregister_command(&mut self, name: &str) {
        self.defined.remove(name);
    }
    fn register_lazy_command(&mut self, name: &str, plugin: &PluginId) -> Result<(), String> {
        if self.defined.contains(name) {
            return Err(format!("'{name}' conflicts with an existing command"));
        }
        if let Some(owner) = self.lazy.get(name) {
            return if owner == plugin {
                Ok(())
            } else {
                Err(format!("'{name}' already claimed by lazy plugin '{owner}'"))
            };
        }
        self.lazy.insert(name.to_string(), plugin.clone());
        Ok(())
    }
    fn lazy_command_owner(&self, name: &str) -> Option<PluginId> {
        self.lazy.get(name).cloned()
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &PluginId) {
        self.lazy.retain(|_, p| p != plugin);
    }
}
