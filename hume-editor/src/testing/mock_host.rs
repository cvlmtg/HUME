//! [`MockHost`] — shared [`hume_scripting::host::EditorHost`] for lib unit tests and
//! integration tests.
//!
//! Holds real `EditorSettings` and `Keymap` so tests can assert on
//! `set-option!` / `bind-key!` side effects directly, without a full editor
//! session.
//!
//! Reached two ways, both through the one real module (no `#[path]`
//! duplication): lib unit tests get it under plain `#[cfg(test)]`;
//! `editor/tests/scripting.rs` and `editor/tests/unix/main.rs` link against
//! it as `hume::testing::MockHost` via the `test-util` feature their
//! `Cargo.toml` dev-dependency enables on this crate.
//!
//! Uses `hume::` paths throughout; `extern crate self as hume` in `lib.rs`
//! makes those resolve correctly in the lib-crate context too.
//!
//! # Design rule: delegate, record, or faithfully mirror — never approximate
//!
//! Every method here is (a) a thin wrapper over a *real* production
//! structure/function it holds (`self.settings`, `hume::editor::settings::
//! setting_value`, `hume_ops::register::is_valid_register_name`),
//! (b) pure recording of whatever the test already told it (`dispatched_
//! native`, `native_names`), or (c) a reduced but faithful mirror of a real
//! decision, restated in the exact terms this mock actually tracks
//! (`register_command`/`register_lazy_command` reject a name already present
//! in `registered_cmds`/`lazy_cmds`, matching `CommandRegistry`'s real
//! collision rule one-for-one; `attach_grammar` checks the same bad-path
//! failure the real host hits, without doing real tree-sitter compilation).
//! What must never happen is an *invented* approximation that only
//! coincidentally agrees with the real decision today — every check here
//! traces back to a specific real rule it mirrors, cited at the call site.
//! A test whose scenario needs behavior finer-grained than what's mirrored
//! (e.g. native/typed-command collisions, real grammar parsing) uses a real
//! `Editor` + `EditorHostImpl` instead (see
//! `hume-editor/src/editor/tests/plugins.rs`).

use std::ops::Range;

use hume_engine::pipeline::{BufferId, PaneId};
use hume_scripting::host::{
    BufferHost, CommandHost, CursorHost, EditorHost, EventHost, LanguageHost, OptionValue,
    SettingsHost,
};

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
    /// Record of every `run_command_sync` call: `(name, count, extend, register)`.
    /// `count` is `None` when the Steel side passed `0` ("no count typed").
    pub dispatched_native: Vec<(String, Option<usize>, bool, Option<char>)>,
    /// Lazy activation stubs registered via `register_lazy_command`.
    pub lazy_cmds: rustc_hash::FxHashMap<String, hume_scripting::attribution::PluginId>,
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
        }
    }

    /// Whether `name` is already claimed by a defined (non-Lazy) command,
    /// mappable or typed — the two vectors are one namespace in the real
    /// registry. Shared by `register_command`/`register_typed_command`.
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
    fn pane_ids(&self) -> Vec<PaneId> {
        Vec::new()
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        false
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
    fn close_buffer(&mut self, _id: BufferId) -> Result<BufferId, String> {
        Err("MockHost: close_buffer not available".into())
    }
    fn switch_to_buffer(&mut self, _current: BufferId, _target: BufferId) -> Result<(), String> {
        Err("MockHost: switch_to_buffer not available".into())
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
    fn line_to_offset(&self, _id: BufferId, _line: usize) -> Option<usize> {
        None
    }
    fn viewport_range(
        &self,
        _id: BufferId,
    ) -> Option<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>> {
        None
    }
}

impl SettingsHost for MockHost {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        // MockHost models no editor state to resync derived state against
        // (no history rings, no buffers, no view) — write_global is the
        // effect-free raw writer, and it's the only one that fits here.
        hume::editor::settings::ops::write_global_for_test(key, value, &mut self.settings)
    }
    fn set_buffer_option(
        &mut self,
        _key: &str,
        _value: &str,
        _bid: BufferId,
    ) -> Result<(), String> {
        // MockHost models no buffers — no per-buffer override to write to.
        Err("MockHost: set_buffer_option not available".into())
    }
    fn get_option(&self, key: &str, _bid: BufferId) -> Result<OptionValue, String> {
        // MockHost models no buffers, so there is no per-buffer override to
        // resolve — every key reads its global value.
        hume::editor::settings::setting_value(key, &self.settings, None)
            .ok_or_else(|| format!("get-option: unknown setting '{key}'"))
    }
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        // `EditorSettings.statusline` is private outside `settings.rs`, so
        // this re-serializes to the wire format and writes through
        // `write_global` — the same path `EditorHostImpl::configure_statusline`
        // (`host_impl.rs`) uses, rather than a second, mock-only writer.
        use hume::statusline::{StatusLineConfig, parse_statusline_section};
        let cfg = StatusLineConfig {
            left: parse_statusline_section(left, "left")?,
            center: parse_statusline_section(center, "center")?,
            right: parse_statusline_section(right, "right")?,
        };
        let wire = hume::editor::settings::format_statusline(&cfg);
        hume::editor::settings::ops::write_global_for_test("statusline", &wire, &mut self.settings)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        self.settings.steel_command_budget_ms as u64
    }
}

impl LanguageHost for MockHost {
    // Checks the same bad-path failure mode `attach_grammar_errs_for_bad_path`
    // (host_impl.rs) pins on the real host, without doing real tree-sitter
    // grammar/query compilation — that's expensive and this lightweight mock
    // has no reason to perform it. A path that exists but doesn't actually
    // parse as a valid grammar/query still succeeds here; no test needs that
    // finer-grained failure through `MockHost` today.
    //
    // Error prefixes mirror `RegisterError`'s `Display` (`hume-treesitter`'s
    // `registry.rs`: `"grammar load failed: ..."` / `"highlights.scm read
    // failed: ..."`) rather than inventing separate wording — a caller that
    // only ever sees `MockHost`'s errors should still learn the real vocabulary.
    // The detail past the prefix is this mock's own (the path), not a
    // reproduction of the OS-specific `io::Error`/dlopen text the real host
    // would show — that text is platform- and locale-dependent and not worth
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
    fn register_trigger_chars(&mut self, _source: String, _language: String, _chars: Vec<char>) {}
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
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        self.dispatched_native
            .push((name.to_owned(), count, extend, register));
        Ok(())
    }
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        // Mirrors `EditorHostImpl::register_command` (host_impl.rs), reduced
        // to what this mock actually tracks: a name already in
        // `registered_cmds`/`registered_typed_cmds` is a SteelBacked/native/
        // typed conflict (the real host's `Some(_) => Err` branch); a name
        // only in `lazy_cmds` is a `Lazy` stub, which the real
        // `CommandRegistry::register` allows overwriting (`Some(Lazy) | None
        // => Ok`) — so clear it here too.
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
        // Mirrors `register_command` above — mappable and typed names share
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
        plugin: &hume_scripting::attribution::PluginId,
    ) -> Result<(), String> {
        // Deliberately permissive, like `register_command` above — collision
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
        plugin: &hume_scripting::attribution::PluginId,
    ) -> Result<(), String> {
        self.lazy_cmds.insert(name.to_owned(), plugin.clone());
        Ok(())
    }
    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::attribution::PluginId> {
        self.lazy_cmds.get(name).cloned()
    }
    // `lazy_cmds` tracks no kind, so this mock can't tell a typed-only stub
    // from a mappable one — same answer as `lazy_command_owner`. A test
    // needing the real mappable-only distinction (`call!` on a typed-only
    // lazy name) uses a real `Editor` + `EditorHostImpl` instead, per this
    // struct's own doc.
    fn lazy_mappable_command_owner(
        &self,
        name: &str,
    ) -> Option<hume_scripting::attribution::PluginId> {
        self.lazy_cmds.get(name).cloned()
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &hume_scripting::attribution::PluginId) {
        self.lazy_cmds.retain(|_, p| p != plugin);
    }
}

impl CursorHost for MockHost {
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
    fn selections_linewise(&self, _bid: BufferId) -> bool {
        false
    }
    fn selections_charwise(&self, _bid: BufferId) -> bool {
        false
    }
}
