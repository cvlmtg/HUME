//! Tests that cannot run on Windows, gated once at the `mod unix;`
//! declaration in the parent — files in here need no `#[cfg]` attributes.
//!
//! Most tests here load Steel plugins from disk: Scheme `require` strings
//! embed OS paths, and backslashes are not escaped in Steel string literals.
//! The rest exercise unix-only behavior directly (e.g. `HUME_RUNTIME`
//! resolution, `set_cwd` against canonicalized paths).
//!
//! A test file with both portable and unix-only tests is split into a
//! same-named file here holding the unix-only half.

use super::*;

// ── Shared unix-only guards and fixtures ─────────────────────────────────────

/// Lock `HUME_RUNTIME_MUTEX`, create isolated `runtime` and `tmp` tempdirs,
/// set `HUME_RUNTIME` and `TMPDIR`, and restore both on drop.
///
/// The mutex is acquired BEFORE the tempdirs are created so that a concurrent
/// guarded test's TMPDIR does not cause our tempdirs to be nested inside it —
/// which would make them disappear when that test's guard drops and deletes its
/// tree.
struct HumeRuntimeGuard {
    runtime: tempfile::TempDir,
    tmp: tempfile::TempDir,
    // Last field — released after runtime/tmp dirs are deleted.
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl HumeRuntimeGuard {
    fn new() -> Self {
        let lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = tempfile::tempdir().expect("tempdir");
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("HUME_RUNTIME", runtime.path());
            std::env::set_var("TMPDIR", tmp.path());
        }
        HumeRuntimeGuard {
            runtime,
            tmp,
            _lock: lock,
        }
    }
}

impl Drop for HumeRuntimeGuard {
    fn drop(&mut self) {
        // Clear env vars before the TempDir fields delete their directories and
        // before _lock releases the mutex, so the next waiter sees a clean env.
        unsafe {
            std::env::remove_var("HUME_RUNTIME");
            std::env::remove_var("TMPDIR");
        }
    }
}

/// The real shipped `core:stdlib` plugin source, embedded so tests exercise
/// the actual file rather than a hand-rolled stand-in.
const STDLIB_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/stdlib/plugin.scm"
));

/// Stage a real shipped core plugin's source into `guard`'s isolated
/// `HUME_RUNTIME/plugins/core/<name>/plugin.scm`, so `load-plugin` resolves it
/// as a core plugin during the test.
fn write_core_plugin(guard: &HumeRuntimeGuard, name: &str, source: &str) {
    let plugin_dir = guard.runtime.path().join("plugins").join("core").join(name);
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), source).unwrap();
}

/// Points `HUME_RUNTIME` at the *real*, on-disk `runtime/` directory (a
/// sibling of the crate root, resolved once via `CARGO_MANIFEST_DIR`) for
/// the guard's lifetime — used by multi-file core plugins (`core:lsp`,
/// mirroring `core:plum`'s layout) so tests exercise the actual shipped
/// files without hand-copying every one into a temp dir and keeping that
/// list in sync as feature files are added.
///
/// Also points `XDG_DATA_HOME` at a fresh, guard-owned temp dir: loading the
/// real `core:lsp` plugin now scans `<data-dir>/servers/` at load time (see
/// `lsp/registration.scm`), so without this every `RealRuntimeGuard` test
/// would scan whatever the developer running the suite actually has
/// installed on their machine — non-hermetic, and a source of spurious
/// scan warnings in test output.
///
/// Deliberately does **not** touch `TMPDIR`, unlike [`HumeRuntimeGuard`]:
/// pointing at a persistent, never-deleted directory means there is nothing
/// for a concurrent test's cleanup to race against. `HumeRuntimeGuard`'s
/// `TMPDIR` override only protects itself from *other* `HumeRuntimeGuard`s
/// (both take the same mutex) — it does not and cannot protect unrelated
/// tests that call bare `tempfile::tempdir()`, since `TMPDIR` is a
/// process-global env var every thread's allocator reads. A slow guarded
/// test can redirect an unrelated concurrent test's `tempfile::tempdir()`
/// into its own tree and then delete that tree out from under it on drop.
/// Avoiding `TMPDIR` entirely sidesteps the hazard rather than narrowing it.
/// `XDG_DATA_HOME` doesn't need the same care — nothing outside HUME's own
/// `data-dir` resolution reads it, so there is no allocator-style hazard.
struct RealRuntimeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    _data_tmp: tempfile::TempDir,
    prev_xdg_data_home: Option<String>,
}

impl RealRuntimeGuard {
    fn new() -> Self {
        let lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let real_runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime");
        let data_tmp = tempfile::tempdir().expect("tempdir");
        let prev_xdg_data_home = std::env::var("XDG_DATA_HOME").ok();
        unsafe {
            std::env::set_var("HUME_RUNTIME", real_runtime);
            std::env::set_var("XDG_DATA_HOME", data_tmp.path());
        }
        RealRuntimeGuard {
            _lock: lock,
            _data_tmp: data_tmp,
            prev_xdg_data_home,
        }
    }
}

impl Drop for RealRuntimeGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("HUME_RUNTIME");
            match &self.prev_xdg_data_home {
                Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
    }
}

/// Like `CwdGuard`, but also owns a tempdir the test can `cd` into.
///
/// Bundling the tempdir into the same struct as the restore-on-drop logic is
/// what fixes the historical bug, not the fields' declaration order: Rust
/// always runs a struct's custom `Drop::drop` to completion *before* dropping
/// any of its own fields, regardless of their order. So restoring cwd inside
/// `CwdSandbox::drop` is guaranteed to happen before `dir` (the `TempDir`
/// field) is deleted.
///
/// A test that instead pairs a bare `CwdGuard` with a *separately-scoped*
/// `tempfile::tempdir()` local doesn't get that guarantee — independent
/// locals in a function body drop in reverse declaration order, so the
/// tempdir (declared after the guard) drops *first*, deleting the directory
/// while the process cwd still points inside it. Any concurrently-running
/// test that calls `std::env::current_dir()` in that window — e.g. Steel's
/// `Engine::new()`, which falls back to it while compiling `ALL_MODULES` —
/// gets `ENOENT` and panics. `CwdSandbox` closes that window structurally.
struct CwdSandbox {
    dir: tempfile::TempDir,
    saved: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl CwdSandbox {
    fn new() -> Self {
        let _lock = CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::current_dir().expect("current_dir");
        let dir = tempfile::tempdir().expect("tempdir");
        Self { dir, saved, _lock }
    }

    /// Raw tempdir path — build child dirs/files under this.
    fn raw(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Canonicalized tempdir path (macOS /var → /private/var) for cwd asserts.
    fn path(&self) -> PathBuf {
        std::fs::canonicalize(self.dir.path()).expect("canonicalize")
    }
}

impl Drop for CwdSandbox {
    fn drop(&mut self) {
        // Restore first; `dir` is only deleted afterwards, when the field drops.
        let _ = std::env::set_current_dir(&self.saved);
    }
}

mod alternate;
mod buffer;
mod buffer_store;
mod cd;
mod command_mode;
mod completion;
mod dot_repeat;
mod file_io;
mod injections_editor;
mod language;
mod list_buffers;
mod lsp_actions;
mod lsp_bridge;
mod lsp_completion_feature;
mod lsp_diagnostics_inline;
mod lsp_diagnostics_nav;
mod lsp_format;
mod lsp_goto;
mod lsp_hover;
mod lsp_inlay_feature;
mod lsp_packaging;
mod lsp_references;
mod lsp_rename;
mod lsp_sighelp;
mod multi_pane;
mod picker_source;
mod plugins;
mod scripting_effects;
mod scripting_grammar;
mod scripting_lsp_install;
mod sync_dispatch;
mod tutor;
mod vim_keybind;
