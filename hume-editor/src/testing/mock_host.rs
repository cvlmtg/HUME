//! [`MockHost`] — shared [`hume_scripting::EditorHost`] for lib unit tests and
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
//! structure/function it holds (`self.settings`, `self.keymap`, `hume::
//! settings::setting_value`, `hume::ops::register::is_valid_register_name`),
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

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_scripting::host::{
    BindMode, BufferHost, CommandHost, CursorHost, EditorHost, KeymapHost, LanguageHost,
    OptionValue, SettingsHost,
};

pub(crate) struct MockHost {
    pub(crate) settings: hume::settings::EditorSettings,
    pub(crate) keymap: hume::Keymap,
    /// Grammar names attached via `(register-grammar! …)`.
    pub(crate) grammars: std::collections::HashSet<String>,
    /// Commands registered via `(define-command! …)` during evals.
    pub(crate) registered_cmds: Vec<hume_scripting::SteelCmdDef>,
    /// Names treated as native by `command_is_native`.  Empty by default
    /// (all commands return `Ok(false)`).  Tests populate this to exercise
    /// the `run_command_sync` path.
    pub(crate) native_names: std::collections::HashSet<String>,
    /// Record of every `run_command_sync` call: `(name, count, extend, register)`.
    /// `count` is `None` when the Steel side passed `0` ("no count typed").
    pub(crate) dispatched_native: Vec<(String, Option<usize>, bool, Option<char>)>,
    /// Lazy activation stubs registered via `register_lazy_command`.
    pub(crate) lazy_cmds: std::collections::HashMap<String, hume_scripting::PluginId>,
}

impl MockHost {
    pub(crate) fn new() -> Self {
        Self {
            settings: hume::settings::EditorSettings::default(),
            keymap: hume::Keymap::default(),
            grammars: std::collections::HashSet::new(),
            registered_cmds: Vec::new(),
            native_names: std::collections::HashSet::new(),
            dispatched_native: Vec::new(),
            lazy_cmds: std::collections::HashMap::new(),
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
    fn keymap(&mut self) -> &mut dyn KeymapHost {
        self
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        self
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
        use hume::settings::{BufferOverrides, SettingScope, apply_setting};
        let mut dummy = BufferOverrides::default();
        apply_setting(
            SettingScope::Global,
            key,
            value,
            &mut self.settings,
            &mut dummy,
        )
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
        use hume::ui::statusline::{StatusElement, StatusLineConfig};
        let parse = |list: Vec<String>, section: &str| -> Result<Vec<StatusElement>, String> {
            list.iter()
                .map(|s| {
                    s.parse::<StatusElement>()
                        .map_err(|e| format!("configure-statusline! {section}: {e}"))
                })
                .collect()
        };
        let left = parse(left, "left")?;
        let center = parse(center, "center")?;
        let right = parse(right, "right")?;
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

impl KeymapHost for MockHost {
    fn bind_key(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
        force_extend: bool,
    ) -> Result<(), String> {
        self.keymap.bind_user_with_extend(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
            force_extend,
        );
        Ok(())
    }
    fn bind_wait_char(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
    ) -> Result<(), String> {
        self.keymap.bind_wait_char_user(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
        );
        Ok(())
    }
    fn unbind_key(&mut self, mode: BindMode, keys: &[KeyEvent]) -> Result<(), String> {
        self.keymap.unbind_user(to_editor_bind_mode(mode), keys);
        Ok(())
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
        hume::ops::register::is_valid_register_name(ch)
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
        plugin: &hume_scripting::PluginId,
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
    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::PluginId> {
        self.lazy_cmds.get(name).cloned()
    }
    fn unregister_lazy_stubs_of(&mut self, plugin: &hume_scripting::PluginId) {
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

fn to_editor_bind_mode(mode: BindMode) -> hume::KeymapBindMode {
    match mode {
        BindMode::Normal => hume::KeymapBindMode::Normal,
        BindMode::Extend => hume::KeymapBindMode::Extend,
        BindMode::Insert => hume::KeymapBindMode::Insert,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_def(name: &str) -> hume_scripting::SteelCmdDef {
        hume_scripting::SteelCmdDef {
            name: name.to_string(),
            doc: String::new(),
            arity: 0,
            is_variadic: false,
            inline_output: false,
            repeatable: false,
        }
    }

    #[test]
    fn register_command_rejects_duplicate_name() {
        let mut mock = MockHost::new();
        mock.register_command(cmd_def("dup"))
            .expect("first registration must succeed");

        let err = mock
            .register_command(cmd_def("dup"))
            .expect_err("second registration of the same name must be rejected");
        assert!(
            err.contains("dup") && err.contains("conflicts with existing command"),
            "unexpected message: {err}"
        );
        assert_eq!(
            mock.registered_cmds.len(),
            1,
            "the rejected redefinition must not be recorded"
        );
    }

    #[test]
    fn register_command_overwrites_lazy_stub() {
        let mut mock = MockHost::new();
        let plugin = hume_scripting::PluginId::parse("core:test").unwrap();
        mock.register_lazy_command("bar", &plugin).unwrap();
        assert!(mock.lazy_command_owner("bar").is_some());

        mock.register_command(cmd_def("bar"))
            .expect("defining over a Lazy stub must succeed");

        assert!(
            mock.lazy_command_owner("bar").is_none(),
            "the Lazy stub must be cleared once the real command is defined"
        );
    }

    #[test]
    fn attach_grammar_rejects_missing_grammar_path() {
        let mut mock = MockHost::new();
        let dir = tempfile::tempdir().unwrap();
        let highlights = dir.path().join("highlights.scm");
        std::fs::write(&highlights, "").unwrap();

        let err = mock
            .attach_grammar(
                "rust",
                std::path::Path::new("/no/such/lib.dylib"),
                "rust_language",
                &highlights,
                None,
            )
            .expect_err("missing grammar path must be rejected");
        assert!(
            err.contains("grammar library not found"),
            "unexpected message: {err}"
        );
        assert!(
            !mock.has_grammar("rust"),
            "a failed attach must not be recorded"
        );
    }

    #[test]
    fn attach_grammar_rejects_missing_highlights_path() {
        let mut mock = MockHost::new();
        let dir = tempfile::tempdir().unwrap();
        let grammar = dir.path().join("lib.dylib");
        std::fs::write(&grammar, "").unwrap();

        let err = mock
            .attach_grammar(
                "rust",
                &grammar,
                "rust_language",
                std::path::Path::new("/no/such/highlights.scm"),
                None,
            )
            .expect_err("missing highlights path must be rejected");
        assert!(
            err.contains("highlights query not found"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn attach_grammar_succeeds_when_both_paths_exist() {
        let mut mock = MockHost::new();
        let dir = tempfile::tempdir().unwrap();
        let grammar = dir.path().join("lib.dylib");
        let highlights = dir.path().join("highlights.scm");
        std::fs::write(&grammar, "").unwrap();
        std::fs::write(&highlights, "").unwrap();

        mock.attach_grammar("rust", &grammar, "rust_language", &highlights, None)
            .expect("both paths existing must succeed");
        assert!(mock.has_grammar("rust"));
    }
}
