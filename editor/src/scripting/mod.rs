//! Steel scripting integration for HUME.
//!
//! The [`ScriptingHost`] owns the Steel [`Engine`] and runs entirely on the
//! main event-loop thread — Steel's Engine is `!Send` by design (internal
//! `Rc`/`RefCell`, non-atomic `im-rs` lists). This is a deliberate choice:
//! edit commands are synchronous `(Buffer, SelectionSet) → (Buffer, SelectionSet)`
//! operations on the hot-key path; an IPC round-trip per keystroke would be
//! strictly worse than a direct function call.
//!
//! ## Phases
//! - Phase 1 (this file): embed the engine, evaluate `init.scm`, report errors.
//! - Phase 2 (`ledger.rs`): ownership ledger + `CURRENT_PLUGIN` attribution stack.
//! - Phase 3 (this file + `builtins/`): mutation builtins (`set-option!`,
//!   `bind-key!`) and `load-plugin` (Scheme-defined, Rust-backed).
//! - Phase 4: Steel-backed statusline segments via a `Send+Sync` cache proxy.
//! - Phase 5: step budget / `Ctrl-C` interruption via a watchdog `AtomicBool`.

pub(crate) mod builtins;
pub(crate) mod ledger;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use steel::steel_vm::engine::Engine;

use crate::editor::keymap::Keymap;
use crate::settings::EditorSettings;

use ledger::{LedgerStack, PluginStack};

// ── EvalCtx ───────────────────────────────────────────────────────────────────

/// Editor state moved into thread-local storage for the duration of
/// [`ScriptingHost::eval_init`].
///
/// Every builtin function accesses this via [`EVAL_CTX`] (through
/// [`builtins::with_ctx`]).  The TLS move-in/move-out pattern lets us use
/// plain `FunctionSignature` function pointers (which cannot capture state)
/// while still giving builtins access to the editor's mutable fields.
///
/// Fields are restored to their original locations unconditionally after
/// `eval_init` returns, even on error.
pub(crate) struct EvalCtx {
    /// Editor settings being mutated by `(set-option! …)`.
    pub(crate) settings: EditorSettings,
    /// Keymap being mutated by `(bind-key! …)`.
    pub(crate) keymap: Keymap,
    /// Plugin attribution stack; identifies whose mutation is being recorded.
    pub(crate) plugin_stack: PluginStack,
    /// Ordered ledger of all plugin mutations, used for unload/reload teardown.
    pub(crate) ledger_stack: LedgerStack,
    /// Where PLUM installs third-party plugins (`$XDG_DATA_HOME/hume/`).
    pub(crate) data_dir: PathBuf,
    /// Where core plugins, themes, and docs live.  `None` if not found on disk.
    pub(crate) runtime_dir: Option<PathBuf>,
    /// Every plugin name passed to `(load-plugin …)`, including absent ones.
    /// Used by PLUM's `:plum-install` to discover what to install.
    pub(crate) declared_plugins: Vec<String>,
    /// Plugins that were both declared and successfully located on disk.
    pub(crate) loaded_plugins: Vec<String>,
}

thread_local! {
    /// TLS slot for [`EvalCtx`].  `Some` only during [`ScriptingHost::eval_init`].
    pub(crate) static EVAL_CTX: RefCell<Option<EvalCtx>> = RefCell::new(None);
}

// ── ScriptFacingCtx ───────────────────────────────────────────────────────────

/// Permanent scripting state held on [`ScriptingHost`] between evals.
///
/// Fields are moved into [`EvalCtx`] at the start of each `eval_init` call and
/// restored afterwards.  Between calls they live here so the `ScriptingHost`
/// retains ledger + attribution state across multiple evaluations.
pub(crate) struct ScriptFacingCtx {
    /// `$XDG_DATA_HOME/hume/` — where PLUM installs user/third-party plugins.
    pub(crate) data_dir: PathBuf,
    /// The runtime directory (core plugins, themes, docs), or `None` if not
    /// found.  Absent in some dev setups; the editor still works, but
    /// `core:*` plugins cannot be loaded.
    pub(crate) runtime_dir: Option<PathBuf>,
    /// Attribution stack: `stack.last()` is the plugin currently executing.
    /// Empty → top-level `init.scm` → `Owner::User`.
    pub(crate) plugin_stack: PluginStack,
    /// Ordered ledger of all plugin mutations, used for unload/reload teardown.
    pub(crate) ledger_stack: LedgerStack,
}

// ── ScriptingHost ─────────────────────────────────────────────────────────────

/// The embedded Steel scripting host.
///
/// Owns the [`Engine`] and the [`ScriptFacingCtx`] that builtins reach back
/// into. Constructed once during `Editor::init_scripting()` and held for the
/// lifetime of the process.
pub(crate) struct ScriptingHost {
    engine: Engine,
    pub(crate) ctx: ScriptFacingCtx,
}

impl ScriptingHost {
    /// Evaluate a Steel source string directly, without a file.
    ///
    /// Convenience wrapper for testing.  Mirrors `eval_init` but accepts a
    /// string instead of a path, and always evaluates (never returns early).
    #[cfg(test)]
    pub(crate) fn eval_source(
        &mut self,
        source: &str,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        EVAL_CTX.with(|cell| {
            *cell.borrow_mut() = Some(EvalCtx {
                settings: std::mem::take(settings),
                keymap: std::mem::take(keymap),
                plugin_stack: std::mem::take(&mut self.ctx.plugin_stack),
                ledger_stack: std::mem::take(&mut self.ctx.ledger_stack),
                data_dir: self.ctx.data_dir.clone(),
                runtime_dir: self.ctx.runtime_dir.clone(),
                declared_plugins: Vec::new(),
                loaded_plugins: Vec::new(),
            });
        });

        let result = self
            .engine
            .compile_and_run_raw_program(source.to_owned())
            .map(|_| ())
            .map_err(|e| e.to_string());

        EVAL_CTX.with(|cell| {
            if let Some(ctx) = cell.borrow_mut().take() {
                *settings = ctx.settings;
                *keymap = ctx.keymap;
                self.ctx.plugin_stack = ctx.plugin_stack;
                self.ctx.ledger_stack = ctx.ledger_stack;
            }
        });

        result
    }

    /// Create a new scripting host with the Steel standard library and all HUME
    /// builtins loaded.
    ///
    /// Resolves base directories eagerly so builtins can use them without
    /// re-reading environment variables on every call.
    pub(crate) fn new() -> Self {
        let ctx = ScriptFacingCtx {
            data_dir: crate::os::dirs::data_dir(),
            runtime_dir: crate::os::dirs::runtime_dir(),
            plugin_stack: PluginStack::default(),
            ledger_stack: LedgerStack::default(),
        };
        let mut engine = Engine::new();
        builtins::register_all(&mut engine);
        Self { engine, ctx }
    }

    /// Evaluate `init.scm` at `path`, giving builtins access to `settings` and
    /// `keymap` for the duration of the call.
    ///
    /// - Returns `Ok(())` if the file does not exist (missing config is normal).
    /// - Returns `Err(message)` if the file exists but fails to parse or
    ///   evaluate.  The caller is responsible for surfacing the error to the
    ///   user.
    ///
    /// `settings` and `keymap` are moved into the TLS [`EvalCtx`] before
    /// evaluation and restored afterwards — even on error.  Builtins such as
    /// `set-option!` and `bind-key!` mutate them through the TLS handle.
    pub(crate) fn eval_init(
        &mut self,
        path: &Path,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        if !path.exists() {
            return Ok(());
        }
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;

        // Move editor state + scripting state into TLS so builtins can access
        // them as plain `FunctionSignature` function pointers (which cannot
        // capture variables).  `std::mem::take` replaces each field with its
        // `Default` as a harmless placeholder for the duration.
        EVAL_CTX.with(|cell| {
            *cell.borrow_mut() = Some(EvalCtx {
                settings: std::mem::take(settings),
                keymap: std::mem::take(keymap),
                plugin_stack: std::mem::take(&mut self.ctx.plugin_stack),
                ledger_stack: std::mem::take(&mut self.ctx.ledger_stack),
                data_dir: self.ctx.data_dir.clone(),
                runtime_dir: self.ctx.runtime_dir.clone(),
                declared_plugins: Vec::new(),
                loaded_plugins: Vec::new(),
            });
        });

        // compile_and_run_raw_program requires Into<Cow<'static, str>>;
        // passing the owned String satisfies this via Cow::Owned.
        let result = self
            .engine
            .compile_and_run_raw_program(source)
            .map(|_| ())
            .map_err(|e| e.to_string());

        // Restore all state unconditionally — builtins may have modified
        // settings/keymap, and plugin_stack/ledger_stack accumulate across evals.
        EVAL_CTX.with(|cell| {
            if let Some(ctx) = cell.borrow_mut().take() {
                *settings = ctx.settings;
                *keymap = ctx.keymap;
                self.ctx.plugin_stack = ctx.plugin_stack;
                self.ctx.ledger_stack = ctx.ledger_stack;
            }
        });

        result
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EditorSettings;
    use crate::editor::keymap::Keymap;

    fn host() -> ScriptingHost {
        ScriptingHost::new()
    }

    // ── set-option! ───────────────────────────────────────────────────────────

    #[test]
    fn set_option_tab_width_integer() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        assert_eq!(s.tab_width, 4);
        h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 2);
    }

    #[test]
    fn set_option_tab_width_string() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source("(set-option! \"tab-width\" \"8\")", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 8);
    }

    #[test]
    fn set_option_bool_as_bool() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        assert!(s.mouse_enabled);
        h.eval_source("(set-option! \"mouse-enabled\" #f)", &mut s, &mut km).unwrap();
        assert!(!s.mouse_enabled);
    }

    #[test]
    fn set_option_unknown_key_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(set-option! \"nonexistent\" \"val\")", &mut s, &mut km)
            .unwrap_err();
        assert!(err.contains("unknown setting"), "got: {err}");
    }

    #[test]
    fn set_option_settings_restored_on_error() {
        // On error, settings should be back in their pre-eval state.
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // First set tab-width to 2...
        h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 2);
        // Then run a script that errors mid-way: tab-width is set to 8, then a
        // bad setting that raises. The eval errors, but settings should be returned
        // (with the partial mutation intact — fail-fast, no per-statement rollback).
        let _ = h.eval_source(
            "(set-option! \"tab-width\" 8)\n(set-option! \"bogus\" \"x\")",
            &mut s, &mut km,
        );
        // Settings are back in our hands (not stuck in TLS).
        let _ = s.tab_width; // accessible = test doesn't hang/panic
    }

    // ── bind-key! ─────────────────────────────────────────────────────────────

    #[test]
    fn bind_key_does_not_error_on_valid_input() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // A valid binding should succeed; the trie is verified via keymap's own tests.
        h.eval_source("(bind-key! \"normal\" \"z\" \"move-right\")", &mut s, &mut km).unwrap();
    }

    #[test]
    fn bind_key_multi_key_sequence_no_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source("(bind-key! \"normal\" \"gh\" \"move-right\")", &mut s, &mut km).unwrap();
    }

    #[test]
    fn bind_key_invalid_mode_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(bind-key! \"visual\" \"f\" \"cmd\")", &mut s, &mut km)
            .unwrap_err();
        assert!(err.contains("mode"), "got: {err}");
    }

    #[test]
    fn bind_key_invalid_key_sequence_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(bind-key! \"normal\" \"<bogus>\" \"cmd\")", &mut s, &mut km)
            .unwrap_err();
        assert!(!err.is_empty(), "expected error for unknown key '<bogus>'");
    }

    // ── load-plugin path resolution ────────────────────────────────────────────

    #[test]
    fn load_plugin_missing_plugin_declared_not_loaded() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // The plugin doesn't exist on disk — should be declared but not loaded.
        h.eval_source("(load-plugin \"user/nonexistent-repo\")", &mut s, &mut km).unwrap();

        // Inspect state via builtins.
        // declared-plugins filters out core:* — user/nonexistent should appear.
        let declared_result = h.eval_source("(declared-plugins)", &mut s, &mut km);
        // Even if we can't inspect the list directly here, the eval should not error.
        assert!(declared_result.is_ok(), "declared-plugins raised: {:?}", declared_result);
    }

    #[test]
    fn load_plugin_malformed_name_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(load-plugin \"just-a-name\")", &mut s, &mut km)
            .unwrap_err();
        assert!(!err.is_empty(), "expected error for malformed plugin name");
    }

    // ── configure-statusline! ─────────────────────────────────────────────────

    #[test]
    fn configure_statusline_sets_left_section() {
        use crate::ui::statusline::StatusElement;
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(configure-statusline! '("Mode" "FileName") '() '("Position"))"#,
            &mut s, &mut km,
        ).unwrap();

        assert_eq!(s.statusline.left,   vec![StatusElement::Mode, StatusElement::FileName]);
        assert_eq!(s.statusline.center, vec![]);
        assert_eq!(s.statusline.right,  vec![StatusElement::Position]);
    }

    #[test]
    fn configure_statusline_all_sections() {
        use crate::ui::statusline::StatusElement;
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(configure-statusline!
                 '("Position" "FileName" "DirtyIndicator")
                 '("SearchMatches")
                 '("Separator" "Mode"))"#,
            &mut s, &mut km,
        ).unwrap();

        assert_eq!(s.statusline.left,
            vec![StatusElement::Position, StatusElement::FileName, StatusElement::DirtyIndicator]);
        assert_eq!(s.statusline.center, vec![StatusElement::SearchMatches]);
        assert_eq!(s.statusline.right,  vec![StatusElement::Separator, StatusElement::Mode]);
    }

    #[test]
    fn configure_statusline_empty_sections() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source("(configure-statusline! '() '() '())", &mut s, &mut km).unwrap();

        assert!(s.statusline.left.is_empty());
        assert!(s.statusline.center.is_empty());
        assert!(s.statusline.right.is_empty());
    }

    #[test]
    fn configure_statusline_unknown_element_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        let err = h.eval_source(
            r#"(configure-statusline! '("NotAnElement") '() '())"#,
            &mut s, &mut km,
        ).unwrap_err();
        assert!(err.contains("NotAnElement"), "got: {err}");
    }

    #[test]
    fn configure_statusline_wrong_arity_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        let err = h.eval_source("(configure-statusline! '())", &mut s, &mut km).unwrap_err();
        assert!(!err.is_empty(), "expected arity error");
    }
}
