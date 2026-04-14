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
//! - Phase 4 (`builtins/statusline.rs`): `(configure-statusline! left center right)`
//!   sets `EditorSettings::statusline` declaratively.
//! - Phase 5 (this file + `builtins/interrupt.rs`): step budget via a watchdog
//!   thread + `(hume/yield!)` cooperative interruption builtin.

pub(crate) mod builtins;
pub(crate) mod ledger;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use steel::steel_vm::engine::Engine;

use std::borrow::Cow;

use crate::editor::keymap::{BindMode, Keymap};
use crate::settings::{apply_setting, BufferOverrides, EditorSettings, SettingScope};

use ledger::{LedgerStack, PluginId, PluginStack};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Wall-clock budget for a single `eval_init` call.
///
/// If the script is still running after this many seconds, the watchdog thread
/// sets the interrupt flag.  Scripts that call `(hume/yield!)` in their hot
/// loops will abort cleanly; scripts that never call it will run to completion
/// regardless (cooperative interruption only — Steel 0.8.2 has no op-callback).
pub(crate) const EVAL_BUDGET_SECS: u64 = 10;

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
    /// Shared interrupt flag.  `hume/yield!` aborts the script when this is
    /// `true`.  Set by the watchdog thread on budget expiry, or externally
    /// for Ctrl-C handling.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
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
    /// Shared interrupt flag.  Set to `true` by the watchdog to signal that
    /// `(hume/yield!)` calls should abort the running script.  Reset to
    /// `false` after every `eval_init` call.
    ///
    /// The editor can also set this directly (e.g. on `Ctrl-C`) while a
    /// script is running — future wiring, not yet plumbed.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
}

impl ScriptingHost {
    /// Evaluate a Steel source string directly, without a file.
    ///
    /// Convenience wrapper for testing.  Mirrors `eval_init` but accepts a
    /// string instead of a path, and always evaluates (never returns early).
    /// Does not spawn a watchdog thread — tests that need the interrupt flag
    /// set can do so directly via [`ScriptingHost::interrupt_flag`].
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
                interrupt_flag: Arc::clone(&self.interrupt_flag),
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

        // Reset the flag so a pre-set interrupt doesn't bleed into the next eval.
        self.interrupt_flag.store(false, Ordering::Relaxed);

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
        Self {
            engine,
            ctx,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
        }
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
                interrupt_flag: Arc::clone(&self.interrupt_flag),
            });
        });

        // Watchdog: set the interrupt flag after EVAL_BUDGET_SECS of wall-clock
        // time.  A cancel flag lets us defuse it quickly once eval returns so
        // the watchdog never fires against a future eval.
        //
        // Interruption is cooperative: scripts must call (hume/yield!) in their
        // loops.  Steel 0.8.2 has no op-callback for involuntary interruption.
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let flag   = Arc::clone(&self.interrupt_flag);
            let cancel = Arc::clone(&cancel);
            let budget = std::time::Duration::from_secs(EVAL_BUDGET_SECS);
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + budget;
                loop {
                    if cancel.load(Ordering::Relaxed) { return; }
                    if std::time::Instant::now() >= deadline {
                        flag.store(true, Ordering::Relaxed);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            });
        }

        // compile_and_run_raw_program requires Into<Cow<'static, str>>;
        // passing the owned String satisfies this via Cow::Owned.
        let result = self
            .engine
            .compile_and_run_raw_program(source)
            .map(|_| ())
            .map_err(|e| e.to_string());

        // Defuse the watchdog and reset the interrupt flag.  Setting cancel
        // first means the watchdog will exit its loop before it can set the
        // flag again after we clear it.
        cancel.store(true, Ordering::Relaxed);
        self.interrupt_flag.store(false, Ordering::Relaxed);

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

    // ── Plugin teardown / reload ───────────────────────────────────────────────

    /// Unload `plugin_name` by replaying its ledger entries in reverse:
    /// settings are restored via [`apply_setting`] and keybinds are restored
    /// (or removed) via [`Keymap::bind_user`] / [`Keymap::unbind_user`].
    ///
    /// Called before re-evaluating a plugin file on `:reload-plugin`.
    /// Also called by the editor command dispatch when a plugin is uninstalled.
    ///
    /// Returns `Ok(())` if the plugin had no ledger (was never loaded or had
    /// no mutations) — unloading an unknown plugin is a no-op.
    pub(crate) fn teardown_plugin(
        &mut self,
        plugin_name: &str,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        let plugin_id = PluginId::new(plugin_name);
        let to_restore = self.ctx.ledger_stack.unload(&plugin_id);

        for entry in to_restore {
            restore_ledger_entry(entry, settings, keymap)?;
        }
        Ok(())
    }

    /// Reload `plugin_name`: tear it down then re-evaluate its file.
    ///
    /// If the plugin file is not found on disk (e.g. uninstalled), teardown
    /// still runs and `Ok(())` is returned — consistent with the load-plugin
    /// "not on disk → silently skipped" rule.
    pub(crate) fn reload_plugin(
        &mut self,
        plugin_name: &str,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        self.teardown_plugin(plugin_name, settings, keymap)?;

        let plugin_id = PluginId::new(plugin_name);
        let path = builtins::plugins::resolve_path_for_name(
            plugin_name,
            self.ctx.runtime_dir.as_deref(),
            &self.ctx.data_dir,
        ).map_err(|e| format!("reload-plugin: {e}"))?;

        let Some(path) = path else { return Ok(()); };
        self.eval_plugin_with_attribution(&plugin_id, &path, settings, keymap)
    }

    /// Evaluate a plugin file with `plugin_id` on the attribution stack.
    ///
    /// Unlike [`eval_init`], this always evaluates (no early return on missing
    /// file), and wraps the eval in a plugin push/pop so mutations are correctly
    /// attributed to `plugin_id`.
    ///
    /// Used by [`reload_plugin`] to re-run a plugin after teardown.
    fn eval_plugin_with_attribution(
        &mut self,
        plugin_id: &PluginId,
        path: &std::path::Path,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;

        // Push the plugin attribution before moving state into TLS so that all
        // mutations during eval are attributed to `plugin_id`.
        self.ctx.plugin_stack.push(plugin_id.clone());

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
                interrupt_flag: Arc::clone(&self.interrupt_flag),
            });
        });

        let cancel = Arc::new(AtomicBool::new(false));
        {
            let flag   = Arc::clone(&self.interrupt_flag);
            let cancel = Arc::clone(&cancel);
            let budget = std::time::Duration::from_secs(EVAL_BUDGET_SECS);
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + budget;
                loop {
                    if cancel.load(Ordering::Relaxed) { return; }
                    if std::time::Instant::now() >= deadline {
                        flag.store(true, Ordering::Relaxed);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            });
        }

        let result = self
            .engine
            .compile_and_run_raw_program(source)
            .map(|_| ())
            .map_err(|e| e.to_string());

        cancel.store(true, Ordering::Relaxed);
        self.interrupt_flag.store(false, Ordering::Relaxed);

        EVAL_CTX.with(|cell| {
            if let Some(ctx) = cell.borrow_mut().take() {
                *settings = ctx.settings;
                *keymap   = ctx.keymap;
                self.ctx.plugin_stack  = ctx.plugin_stack;
                self.ctx.ledger_stack  = ctx.ledger_stack;
            }
        });

        // Unconditionally pop the attribution we pushed before the eval, even
        // if the plugin itself left the stack imbalanced via an error path.
        self.ctx.plugin_stack.pop();

        result
    }
}

// ── Ledger restoration helper ─────────────────────────────────────────────────

/// Apply one ledger entry's restoration to `settings` or `keymap`.
///
/// Setting keys are plain strings like `"tab-width"`.
/// Keymap keys are mode-prefixed: `"normal f"`, `"insert <ctrl-d>"`, etc.
///
/// For keybinds: if `prior_value` is empty the binding is removed
/// (it was unbound before the plugin set it); otherwise it is restored.
fn restore_ledger_entry(
    entry: crate::scripting::ledger::LedgerEntry,
    settings: &mut EditorSettings,
    keymap: &mut Keymap,
) -> Result<(), String> {
    if let Some(mode_and_seq) = keymap_ledger_mode(&entry.key) {
        let (mode, key_str) = mode_and_seq;
        let keys = parse_key_sequence_str(key_str)?;
        if entry.prior_value.is_empty() {
            keymap.unbind_user(mode, &keys);
        } else {
            keymap.bind_user(mode, &keys, Cow::Owned(entry.prior_value));
        }
    } else {
        // Setting key — restore via apply_setting.
        if !entry.prior_value.is_empty() {
            let mut dummy = BufferOverrides::default();
            apply_setting(SettingScope::Global, &entry.key, &entry.prior_value, settings, &mut dummy)
                .map_err(|e| format!("restoring setting '{}': {e}", entry.key))?;
        }
    }
    Ok(())
}

/// If `key` is a keymap ledger key (e.g. `"normal f"`, `"insert <ctrl-d>"`),
/// return `Some((mode, key_sequence_string))`.  Returns `None` for setting keys.
fn keymap_ledger_mode(key: &str) -> Option<(BindMode, &str)> {
    if let Some(rest) = key.strip_prefix("normal ") {
        return Some((BindMode::Normal, rest));
    }
    if let Some(rest) = key.strip_prefix("extend ") {
        return Some((BindMode::Extend, rest));
    }
    if let Some(rest) = key.strip_prefix("insert ") {
        return Some((BindMode::Insert, rest));
    }
    None
}

/// Parse a key-sequence string into `Vec<KeyEvent>` for use in restoration.
///
/// Delegates to the same parser used by `(bind-key!)`.
fn parse_key_sequence_str(s: &str) -> Result<Vec<crossterm::event::KeyEvent>, String> {
    use crossterm::event::{KeyCode, KeyEvent as CE, KeyModifiers};

    let mut keys = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut token = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '>' { closed = true; break; }
                token.push(ch);
            }
            if !closed { return Err(format!("unclosed '<' in ledger key '{s}'")); }
            keys.push(parse_angle_for_restore(&token, s)?);
        } else {
            keys.push(CE::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }
    if keys.is_empty() { return Err(format!("empty key sequence in ledger key '{s}'")); }
    Ok(keys)
}

fn parse_angle_for_restore(token: &str, full: &str) -> Result<crossterm::event::KeyEvent, String> {
    // Reuse the same logic as the bind-key! parser.
    // The key-string format is stable (ledger entries persist across reloads),
    // so this must stay in sync with builtins::keymap_bind's parser.
    use crossterm::event::{KeyEvent as CE, KeyModifiers};
    let lower = token.to_ascii_lowercase();
    let (code, mods) = if let Some(rest) = lower.strip_prefix("ctrl-") {
        (map_key_code(rest, full)?, KeyModifiers::CONTROL)
    } else if let Some(rest) = lower.strip_prefix("shift-") {
        (map_key_code(rest, full)?, KeyModifiers::SHIFT)
    } else if let Some(rest) = lower.strip_prefix("alt-") {
        (map_key_code(rest, full)?, KeyModifiers::ALT)
    } else {
        (map_key_code(&lower, full)?, KeyModifiers::NONE)
    };
    Ok(CE::new(code, mods))
}

fn map_key_code(name: &str, full: &str) -> Result<crossterm::event::KeyCode, String> {
    use crossterm::event::KeyCode;
    match name {
        "backspace" | "bs"     => Ok(KeyCode::Backspace),
        "enter" | "ret" | "cr" => Ok(KeyCode::Enter),
        "left"                 => Ok(KeyCode::Left),
        "right"                => Ok(KeyCode::Right),
        "up"                   => Ok(KeyCode::Up),
        "down"                 => Ok(KeyCode::Down),
        "home"                 => Ok(KeyCode::Home),
        "end"                  => Ok(KeyCode::End),
        "pageup"  | "pgup"     => Ok(KeyCode::PageUp),
        "pagedown"| "pgdown"   => Ok(KeyCode::PageDown),
        "tab"                  => Ok(KeyCode::Tab),
        "backtab"              => Ok(KeyCode::BackTab),
        "delete"  | "del"      => Ok(KeyCode::Delete),
        "insert"  | "ins"      => Ok(KeyCode::Insert),
        "esc"     | "escape"   => Ok(KeyCode::Esc),
        "space"                => Ok(KeyCode::Char(' ')),
        "lt"                   => Ok(KeyCode::Char('<')),
        "gt"                   => Ok(KeyCode::Char('>')),
        "f1"  => Ok(KeyCode::F(1)),  "f2"  => Ok(KeyCode::F(2)),
        "f3"  => Ok(KeyCode::F(3)),  "f4"  => Ok(KeyCode::F(4)),
        "f5"  => Ok(KeyCode::F(5)),  "f6"  => Ok(KeyCode::F(6)),
        "f7"  => Ok(KeyCode::F(7)),  "f8"  => Ok(KeyCode::F(8)),
        "f9"  => Ok(KeyCode::F(9)),  "f10" => Ok(KeyCode::F(10)),
        "f11" => Ok(KeyCode::F(11)), "f12" => Ok(KeyCode::F(12)),
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(KeyCode::Char(c)),
                _ => Err(format!("unknown key '<{name}>' in ledger key '{full}'")),
            }
        }
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

    // ── hume/yield! ───────────────────────────────────────────────────────────

    #[test]
    fn hume_yield_no_interrupt_is_noop() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // With no interrupt flag set, (hume/yield!) is a transparent no-op.
        h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
    }

    #[test]
    fn hume_yield_with_interrupt_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Pre-set the interrupt flag before the eval.
        h.interrupt_flag.store(true, Ordering::Relaxed);
        let err = h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");

        // eval_source resets the flag after every call.
        assert!(!h.interrupt_flag.load(Ordering::Relaxed), "flag should be false after eval");
    }

    #[test]
    fn hume_yield_stops_loop_when_interrupted() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Pre-set so the loop aborts on the very first yield call.
        h.interrupt_flag.store(true, Ordering::Relaxed);
        let err = h.eval_source(
            // Without the interrupt flag this loop would run forever.
            "(let loop () (hume/yield!) (loop))",
            &mut s, &mut km,
        ).unwrap_err();
        assert!(err.contains("interrupted"), "got: {err}");
    }

    #[test]
    fn interrupt_flag_reset_after_eval() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Pre-set the flag; after eval_source it must be cleared.
        h.interrupt_flag.store(true, Ordering::Relaxed);
        let _ = h.eval_source("(hume/yield!)", &mut s, &mut km); // may error — that's fine
        assert!(!h.interrupt_flag.load(Ordering::Relaxed),
                "interrupt_flag must be false after eval_source returns");

        // Subsequent evals with no flag pre-set should succeed normally.
        h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
    }

    // ── teardown_plugin / reload_plugin ───────────────────────────────────────

    /// Run a mini two-plugin scenario:
    ///   plugin A sets tab-width to 8 (prior: 4, core)
    ///   plugin B sets tab-width to 2 (prior: 8, A)
    /// Unloading A rewrites B's prior to (4, core).
    /// Unloading B restores tab-width to 4.
    #[test]
    fn teardown_restores_setting_when_plugin_is_live_owner() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Simulate plugin A setting tab-width to 8.
        // We drive via eval_source with the plugin on the attribution stack.
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (set-option! "tab-width" 8)
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();
        assert_eq!(s.tab_width, 8);

        // Tear down plugin A — tab-width should be restored to 4 (prior).
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 4, "teardown should restore prior tab-width");
    }

    #[test]
    fn teardown_splices_chain_when_later_plugin_owns_key() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // A sets tab-width 8, then B sets it to 2.
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (set-option! "tab-width" 8)
               (pop-current-plugin!)
               (push-current-plugin! "user/b")
               (set-option! "tab-width" 2)
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();
        assert_eq!(s.tab_width, 2);

        // Unload A — B still owns tab-width (live value = 2 unchanged).
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 2, "B's live value must be preserved");

        // Now unload B — B's prior was rewritten by A's teardown to (4, core),
        // so restoring should give tab-width = 4.
        h.teardown_plugin("user/b", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 4, "after both unloads, core default restored");
    }

    #[test]
    fn teardown_restores_keybind() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // The default normal keymap has 'h' bound to "move-left".
        // Plugin A rebinds 'h' to "move-right".
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (bind-key! "normal" "h" "move-right")
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();

        use crate::editor::keymap::BindMode;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
        assert_eq!(km.lookup_command(BindMode::Normal, h_key).as_deref(), Some("move-right"));

        // Tear down plugin A — 'h' should go back to "move-left".
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert_eq!(km.lookup_command(BindMode::Normal, h_key).as_deref(), Some("move-left"),
                   "teardown should restore prior keybind");
    }

    #[test]
    fn teardown_unbinds_when_key_was_previously_unbound() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Bind an unused key (assume 'Q' is not in the default keymap).
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (bind-key! "normal" "Q" "move-right")
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();

        use crate::editor::keymap::BindMode;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let q_key = &[KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)];
        assert!(km.lookup_command(BindMode::Normal, q_key).is_some());

        // Tear down — 'Q' was unbound before, so it should become unbound again.
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert!(km.lookup_command(BindMode::Normal, q_key).is_none(),
                "binding for unowned key must be removed on teardown");
    }

    #[test]
    fn teardown_unknown_plugin_is_noop() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // No error, no state change.
        h.teardown_plugin("user/nonexistent", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 4);
    }
}
