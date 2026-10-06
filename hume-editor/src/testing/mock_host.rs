//! [`MockHost`]: shared [`hume_scripting::host::EditorHost`] for lib unit tests and
//! integration tests.
//!
//! Holds real `EditorSettings` and `Keymap` so tests can assert on
//! `set-option!`/`bind-key!` side effects without a full editor session.
//! Lib unit tests get it under `#[cfg(test)]`; integration tests link it as
//! `hume::testing::MockHost` via the `test-util` feature. `extern crate self
//! as hume` in `lib.rs` makes its `hume::` paths resolve in both.
//!
//! # Design rule: never approximate
//!
//! Every method either delegates to a real production structure or function,
//! records what the test told it, or mirrors a specific real rule (cited at
//! the call site), such as `CommandRegistry`'s name-collision check. A test
//! needing finer behavior (native/typed collisions, real grammar parsing)
//! uses a real `Editor` + `EditorHostImpl` instead (see
//! `editor/tests/plugins.rs`).

use hume_engine::pipeline::BufferId;
use hume_scripting::PaneHandle;
use hume_scripting::host::{
    BufferHost, CommandHost, CursorHost, EditorHost, EventHost, LanguageHost, OptionValue,
    SelectionInfo, SettingsHost,
};

/// One recorded `run_command_sync` call: `(name, pane, count, extend,
/// register)`; see [`MockHost::dispatched_native`]'s own doc.
pub type DispatchedNativeCall = (String, PaneHandle, Option<usize>, bool, Option<char>);

pub struct MockHost {
    pub settings: hume::editor::settings::EditorSettings,
    /// Grammar names attached via `(register-grammar! …)`.
    pub grammars: rustc_hash::FxHashSet<String>,
    /// Commands registered via `(define-command! …)` during evals.
    pub registered_cmds: Vec<hume_scripting::SteelCmdDef>,
    /// Typed commands registered via `(define-typed-command! …)` during evals.
    pub registered_typed_cmds: Vec<hume_scripting::SteelTypedCmdDef>,
    /// Names treated as native by `command_is_native`.  Empty by default
    /// (all commands return `Ok(false)`).  Tests populate this to exercise
    /// the `run_command_sync` path.
    pub native_names: rustc_hash::FxHashSet<String>,
    /// Record of every `run_command_sync` call. `count` is `None`
    /// when the Steel side passed `0` ("no count typed"). Unlike
    /// `EditorHostImpl`, this mock has no pane model to resolve `pane`
    /// against, so every call is recorded regardless of `pane`. See
    /// `run_command_sync`'s own doc.
    pub dispatched_native: Vec<DispatchedNativeCall>,
    /// Lazy activation stubs registered via `register_lazy_command`.
    pub lazy_cmds: rustc_hash::FxHashMap<String, hume_scripting::attribution::EntryId>,
    /// Buffer ids `buffer_exists` answers `true` for. Empty by default, so
    /// every id (including `focused_pane()`'s own `BufferId::default()`)
    /// is "stale" from an explicit-`pane` builtin's point of view unless a
    /// test opts one in. A test whose scenario needs a buffer to read as
    /// live (e.g. `get-buffer-option` reaching the host at all) inserts it
    /// here first.
    pub live_buffer_ids: rustc_hash::FxHashSet<BufferId>,
}

impl MockHost {
    pub fn new() -> Self {
        Self {
            settings: hume::editor::settings::EditorSettings::default(),
            grammars: rustc_hash::FxHashSet::default(),
            registered_cmds: Vec::new(),
            registered_typed_cmds: Vec::new(),
            native_names: rustc_hash::FxHashSet::default(),
            dispatched_native: Vec::new(),
            lazy_cmds: rustc_hash::FxHashMap::default(),
            live_buffer_ids: rustc_hash::FxHashSet::default(),
        }
    }

    /// Whether `name` is already claimed by a defined (non-Lazy) command,
    /// mappable or typed: the two vectors are one namespace in the real
    /// registry.
    fn is_registered(&self, name: &str) -> bool {
        self.registered_cmds.iter().any(|d| d.name == name)
            || self.registered_typed_cmds.iter().any(|d| d.name == name)
    }
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorHost for MockHost {
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

impl EventHost for MockHost {
    fn known_event_names(&self) -> &'static [&'static str] {
        // MockHost is a real part of this crate now, not text spliced into a
        // foreign test crate, so it can name `crate::editor::event` directly
        // instead of keeping a hand-written mirror of its event list in sync.
        crate::editor::event::known_event_names()
    }
}

impl BufferHost for MockHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        Vec::new()
    }
    fn panes(&self) -> Vec<PaneHandle> {
        Vec::new()
    }
    fn focused_pane(&self) -> PaneHandle {
        PaneHandle::buffer_only(BufferId::default())
    }
    fn buffer_panes(&self, pane: PaneHandle) -> Vec<PaneHandle> {
        vec![pane]
    }
    fn require_focused_pane(&self, _pane: PaneHandle) -> Result<(), String> {
        Err("MockHost: require_focused_pane not available".into())
    }
    fn pane_live(&self, _pane: PaneHandle) -> bool {
        false
    }
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.live_buffer_ids.contains(&id)
    }
    fn buffer_path(&self, _id: BufferId) -> Option<std::path::PathBuf> {
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
    fn open_buffer(&mut self, _path: &std::path::Path) -> Result<BufferId, String> {
        Err("MockHost: open_buffer not available".into())
    }
    fn close_buffer(&mut self, _id: BufferId) -> Result<(), String> {
        Err("MockHost: close_buffer not available".into())
    }
    fn switch_to_buffer(&mut self, _pane: PaneHandle, _target: BufferId) -> Result<(), String> {
        Err("MockHost: switch_to_buffer not available".into())
    }
    fn buffer_undo_tree(&self, _id: BufferId) -> Option<Vec<hume_scripting::host::UndoNode>> {
        None
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
    fn buffer_lines(
        &self,
        _id: BufferId,
        _range: hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
    ) -> Option<Vec<String>> {
        None
    }
    fn line_to_offset(&self, _id: BufferId, _line: hume_rope::line::ContentLine) -> Option<usize> {
        None
    }
    fn viewport_range(
        &self,
        _pane: PaneHandle,
    ) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>, String> {
        Err("MockHost: viewport_range not available".into())
    }
}

impl SettingsHost for MockHost {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        // MockHost models no editor state to resync derived state against
        // (no history rings, no buffers, no view). write_global is the
        // effect-free raw writer, and it's the only one that fits here.
        hume::editor::settings::ops::write_global_for_test(key, value, &mut self.settings)
    }
    fn set_buffer_option(
        &mut self,
        _key: &str,
        _value: &str,
        _bid: BufferId,
    ) -> Result<(), String> {
        // MockHost models no buffers, so no per-buffer override to write to.
        Err("MockHost: set_buffer_option not available".into())
    }
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        hume::editor::settings::setting_value(key, &self.settings, None)
            .ok_or_else(|| format!("get-option: unknown setting '{key}'"))
    }
    fn get_buffer_option(&self, key: &str, _bid: BufferId) -> Result<OptionValue, String> {
        // MockHost models no buffers, so there is no per-buffer override to
        // resolve: every key reads its global value.
        hume::editor::settings::setting_value(key, &self.settings, None)
            .ok_or_else(|| format!("get-buffer-option: unknown setting '{key}'"))
    }
    fn steel_command_budget_ms(&self) -> u64 {
        self.settings.steel_command_budget_ms as u64
    }
}

impl LanguageHost for MockHost {
    // Checks the same bad-path failure mode `attach_grammar_errs_for_bad_path`
    // (host_impl.rs) pins on the real host, without doing real tree-sitter
    // grammar/query compilation: that's expensive and this lightweight mock
    // has no reason to perform it. A path that exists but doesn't actually
    // parse as a valid grammar/query still succeeds here; no test needs that
    // finer-grained failure through `MockHost` today.
    //
    // Error prefixes mirror `RegisterError`'s `Display` (`hume-treesitter`'s
    // `registry.rs`: `"grammar load failed: ..."` / `"highlights.scm read
    // failed: ..."`) rather than inventing separate wording. A caller that
    // only ever sees `MockHost`'s errors should still learn the real vocabulary.
    // The detail past the prefix is this mock's own (the path), not a
    // reproduction of the OS-specific `io::Error`/dlopen text the real host
    // would show: that text is platform- and locale-dependent and not worth
    // faking byte-for-byte for an existence check.
    fn attach_grammar(&mut self, reg: &hume_scripting::GrammarReg) -> Result<(), String> {
        if !reg.grammar_path.exists() {
            return Err(format!(
                "register-grammar! '{}': grammar load failed: failed to open grammar library: {}",
                reg.name,
                reg.grammar_path.display()
            ));
        }
        if !reg.highlights_path.exists() {
            return Err(format!(
                "register-grammar! '{}': highlights.scm read failed: {}",
                reg.name,
                reg.highlights_path.display()
            ));
        }
        self.grammars.insert(reg.name.clone());
        Ok(())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.grammars.contains(language)
    }
}

impl CommandHost for MockHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        hume_ops::register::is_valid_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        Ok(self.native_names.contains(name))
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        pane: PaneHandle,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<bool, String> {
        // Unlike `EditorHostImpl::run_command_sync`, this mock has no pane
        // model at all, so it can't resolve `pane` against a command's own
        // target requirement the way the real host does. It accepts and
        // records any `pane` unconditionally. Resolution (whether `pane`
        // names a live pane, the focused pane, or is out of reach) is the
        // real host's job; a test that needs to assert on a resolution
        // failure exercises `EditorHostImpl` directly, not this mock.
        self.dispatched_native
            .push((name.to_owned(), pane, count, extend, register));
        Ok(true)
    }
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        // Mirrors `EditorHostImpl::register_command` (host_impl.rs), reduced
        // to what this mock actually tracks: a name already in
        // `registered_cmds`/`registered_typed_cmds` is a SteelBacked/native/
        // typed conflict (the real host's `Some(_) => Err` branch); a name
        // only in `lazy_cmds` is a `Lazy` stub, which the real
        // `CommandRegistry::register` allows overwriting (`Some(Lazy) | None
        // => Ok`), so clear it here too.
        if self.is_registered(&def.name) {
            return Err(format!(
                "define-command!: '{}' conflicts with existing command",
                def.name
            ));
        }
        self.lazy_cmds.remove(&def.name);
        self.registered_cmds.push(def);
        Ok(())
    }
    fn register_typed_command(
        &mut self,
        def: hume_scripting::SteelTypedCmdDef,
    ) -> Result<(), String> {
        // Mirrors `register_command` above: mappable and typed names share
        // one namespace in the real registry, so the collision check covers
        // both vectors (see `is_registered`).
        if self.is_registered(&def.name) {
            return Err(format!(
                "define-typed-command!: '{}' conflicts with existing command",
                def.name
            ));
        }
        self.lazy_cmds.remove(&def.name);
        self.registered_typed_cmds.push(def);
        Ok(())
    }
    fn unregister_command(&mut self, name: &str) {
        self.registered_cmds.retain(|d| d.name != name);
        self.registered_typed_cmds.retain(|d| d.name != name);
    }
    fn register_lazy_command(
        &mut self,
        name: &str,
        plugin: &hume_scripting::attribution::EntryId,
    ) -> Result<(), String> {
        // Permissive, like `register_command` above. Collision
        // detection is `CommandRegistry`'s decision; testing it here would be
        // a second copy of the same rules that can silently drift from the
        // real behavior it's meant to prove. Tests that need real collision
        // semantics use a real `Editor` + `EditorHostImpl` instead (see
        // `hume-editor/src/editor/tests/plugins.rs`).
        self.lazy_cmds.insert(name.to_owned(), plugin.clone());
        Ok(())
    }
    fn register_lazy_typed_command(
        &mut self,
        name: &str,
        plugin: &hume_scripting::attribution::EntryId,
    ) -> Result<(), String> {
        self.lazy_cmds.insert(name.to_owned(), plugin.clone());
        Ok(())
    }
    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::attribution::EntryId> {
        self.lazy_cmds.get(name).cloned()
    }
    // `lazy_cmds` tracks no kind, so this mock can't tell a typed-only stub
    // from a mappable one, same answer as `lazy_command_owner`. A test
    // needing the real mappable-only distinction (`call!` on a typed-only
    // lazy name) uses a real `Editor` + `EditorHostImpl` instead, per this
    // struct's own doc.
    fn lazy_mappable_command_owner(
        &self,
        name: &str,
    ) -> Option<hume_scripting::attribution::EntryId> {
        self.lazy_cmds.get(name).cloned()
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &hume_scripting::attribution::EntryId) {
        self.lazy_cmds.retain(|_, p| p != plugin);
    }
}

impl CursorHost for MockHost {
    fn buffer_cursor_line(&self, _pane: PaneHandle) -> Result<usize, String> {
        Err("MockHost: buffer_cursor_line not available".into())
    }
    fn buffer_selections(&self, _pane: PaneHandle) -> Result<Vec<SelectionInfo>, String> {
        Err("MockHost: buffer_selections not available".into())
    }
    fn offset_to_line(&self, _bid: BufferId, _idx: usize) -> Option<usize> {
        None
    }
    fn symbol_under_cursor(&self, _pane: PaneHandle) -> Result<String, String> {
        Err("MockHost: symbol_under_cursor not available".into())
    }
    fn selections_linewise(&self, _pane: PaneHandle) -> Result<bool, String> {
        Err("MockHost: selections_linewise not available".into())
    }
    fn selections_charwise(&self, _pane: PaneHandle) -> Result<bool, String> {
        Err("MockHost: selections_charwise not available".into())
    }
}
