//! [`MockHost`] — shared [`hume_scripting::host::EditorHost`] for lib unit tests and
//! integration tests.
//!
//! Holds real `EditorSettings` and `Keymap` so tests can assert on
//! `set-option!` / `bind-key!` side effects directly, without a full editor
//! session.
//!
//! Included in two ways:
//! - `editor/src/testing/mod.rs` → `mod mock_host` for lib unit tests.
//! - `editor/tests/scripting.rs` → `#[path = "../src/testing/mock_host.rs"]`
//!   for integration tests.
//!
//! Uses `hume::` paths throughout; `extern crate self as hume` in `lib.rs`
//! makes those resolve correctly in the lib-crate context too.
//!
//! # Design rule: delegate, record, or faithfully mirror — never approximate
//!
//! Every method here is (a) a thin wrapper over a *real* production
//! structure/function it holds (`self.settings`, `hume::settings::
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

use hume_engine::pipeline::{BufferId, PaneId};
use hume_scripting::host::{
    BufferHost, CommandHost, CursorHost, EditorHost, EventHost, LanguageHost, OptionValue,
    SettingsHost,
};

/// Mirrors `hume::editor::event`'s Steel-visible event names by hand — that
/// module is `pub(crate)`, unreachable from the `tests/scripting.rs`
/// integration crate this file is also spliced into via `#[path]`, so this
/// list can't delegate to the real one. Same trade-off this file already
/// accepts for every other capability: a faithful mirror, kept in sync by
/// hand, not shared code.
const MOCK_HOST_EVENT_NAMES: &[&str] = &[
    "on-buffer-open",
    "on-buffer-close",
    "on-buffer-save",
    "on-buffer-enter",
    "on-focus-gained",
    "on-mode-change",
    "on-language-set",
    "on-lsp-attach",
    "on-lsp-detach",
    "on-diagnostics-changed",
    "on-viewport-change",
    "on-trigger-char",
    "on-completion-accept",
    "on-completion-refilter",
];

pub(crate) struct MockHost {
    pub(crate) settings: hume::settings::EditorSettings,
    /// Grammar names attached via `(register-grammar! …)`.
    pub(crate) grammars: rustc_hash::FxHashSet<String>,
    /// Commands registered via `(define-command! …)` during evals.
    pub(crate) registered_cmds: Vec<hume_scripting::SteelCmdDef>,
    /// Names treated as native by `command_is_native`.  Empty by default
    /// (all commands return `Ok(false)`).  Tests populate this to exercise
    /// the `run_command_sync` path.
    pub(crate) native_names: rustc_hash::FxHashSet<String>,
    /// Record of every `run_command_sync` call: `(name, count, extend, register)`.
    /// `count` is `None` when the Steel side passed `0` ("no count typed").
    pub(crate) dispatched_native: Vec<(String, Option<usize>, bool, Option<char>)>,
    /// Lazy activation stubs registered via `register_lazy_command`.
    pub(crate) lazy_cmds: rustc_hash::FxHashMap<String, hume_scripting::attribution::PluginId>,
}

impl MockHost {
    pub(crate) fn new() -> Self {
        Self {
            settings: hume::settings::EditorSettings::default(),
            grammars: rustc_hash::FxHashSet::default(),
            registered_cmds: Vec::new(),
            native_names: rustc_hash::FxHashSet::default(),
            dispatched_native: Vec::new(),
            lazy_cmds: rustc_hash::FxHashMap::default(),
        }
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
    fn known_event_names(&self) -> Vec<&'static str> {
        MOCK_HOST_EVENT_NAMES.to_vec()
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
    fn viewport_range(&self, _id: BufferId) -> Option<(usize, usize)> {
        None
    }
}

impl SettingsHost for MockHost {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        // MockHost models no editor state to resync derived state against
        // (no history rings, no buffers, no view) — write_global is the
        // effect-free raw writer, and it's the only one that fits here.
        hume::settings::write_global(key, value, &mut self.settings)
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
        hume::settings::setting_value(key, &self.settings, None)
            .ok_or_else(|| format!("get-option: unknown setting '{key}'"))
    }
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        use hume::ui::statusline::{StatusLineConfig, parse_statusline_section};
        let left = parse_statusline_section(left, "left")?;
        let center = parse_statusline_section(center, "center")?;
        let right = parse_statusline_section(right, "right")?;
        self.settings.statusline = StatusLineConfig {
            left,
            center,
            right,
        };
        Ok(())
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
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &std::path::Path,
        _symbol: &str,
        highlights_path: &std::path::Path,
        _injections_path: Option<&std::path::Path>,
    ) -> Result<(), String> {
        if !grammar_path.exists() {
            return Err(format!(
                "register-grammar! '{name}': grammar library not found: {}",
                grammar_path.display()
            ));
        }
        if !highlights_path.exists() {
            return Err(format!(
                "register-grammar! '{name}': highlights query not found: {}",
                highlights_path.display()
            ));
        }
        self.grammars.insert(name.to_owned());
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
        // `registered_cmds` is a SteelBacked/native/typed conflict (the real
        // host's `Some(_) => Err` branch); a name only in `lazy_cmds` is a
        // `Lazy` stub, which the real `CommandRegistry::register` allows
        // overwriting (`Some(Lazy) | None => Ok`) — so clear it here too.
        if self.registered_cmds.iter().any(|d| d.name == def.name) {
            return Err(format!(
                "define-command!: '{}' conflicts with existing command",
                def.name
            ));
        }
        self.lazy_cmds.remove(&def.name);
        self.registered_cmds.push(def);
        Ok(())
    }
    fn unregister_command(&mut self, name: &str) {
        self.registered_cmds.retain(|d| d.name != name);
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
    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::attribution::PluginId> {
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
    fn selection_spans_full_line(&self, _bid: BufferId) -> bool {
        false
    }
}
