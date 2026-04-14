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
//! - Phase 2: ownership ledger + `CURRENT_PLUGIN` attribution stack.
//! - Phase 3: mutation builtins (`set-option!`, `bind-key!`, `define-command!`)
//!            and `load-plugin`.
//! - Phase 4: Steel-backed statusline segments via a `Send+Sync` cache proxy.
//! - Phase 5: step budget / `Ctrl-C` interruption via a watchdog `AtomicBool`.

use std::path::{Path, PathBuf};

use steel::steel_vm::engine::Engine;

/// Shared state that Steel builtins can read and mutate.
///
/// Phase 1: holds the resolved base directories used by `resolve-plugin-path`
/// and PLUM (Phase 3+). Later phases add `Rc<RefCell<…>>` handles to the
/// registry, keymap, settings, and ledger.
pub(crate) struct ScriptFacingCtx {
    /// `$XDG_DATA_HOME/hume/` — where PLUM installs user/third-party plugins.
    pub(crate) data_dir: PathBuf,
    /// The runtime directory (core plugins, themes, docs), or `None` if not
    /// found. Absent in some dev setups; the editor still works, but
    /// `core:*` plugins cannot be loaded.
    pub(crate) runtime_dir: Option<PathBuf>,
}

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
    /// Create a new scripting host with the Steel standard library loaded.
    ///
    /// Resolves base directories eagerly so Phase 3 builtins can use them
    /// without re-reading environment variables on every call.
    pub(crate) fn new() -> Self {
        let ctx = ScriptFacingCtx {
            data_dir: crate::os::dirs::data_dir(),
            runtime_dir: crate::os::dirs::runtime_dir(),
        };
        Self {
            engine: Engine::new(),
            ctx,
        }
    }

    /// Evaluate `init.scm` at `path`.
    ///
    /// - Returns `Ok(())` if the file does not exist (missing config is normal).
    /// - Returns `Err(message)` if the file exists but fails to parse or
    ///   evaluate. The caller is responsible for surfacing the error to the user.
    pub(crate) fn eval_init(&mut self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Ok(());
        }
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        // compile_and_run_raw_program requires Into<Cow<'static, str>>;
        // passing the owned String (not &source) satisfies this via Cow::Owned.
        self.engine
            .compile_and_run_raw_program(source)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
