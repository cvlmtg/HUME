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

use std::path::Path;
use std::time::{Duration, Instant};

// ── Shared async-drain helpers ────────────────────────────────────────────────
//
// Every unix test that waits on a spawned child or streaming source (a
// picker source, a `spawn-async!` job) polls in a bounded loop instead of a
// single drain call — a background thread's result can land on any frame,
// so CI scheduling jitter would flake a "drain once and assert" test.

/// Drains async sources and their queued Steel callbacks/events in a bounded
/// loop until `until` returns true. `settle()` already covers both (see its
/// doc), so this is a single call, not two.
fn drain_until(ed: &mut Editor, mut until: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        ed.settle();
        if until(ed) {
            return;
        }
        assert!(Instant::now() < deadline, "condition never became true");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Same loop as [`drain_until`], but calls `drain_async_sources` directly
/// instead of `settle()` — for tests that drive the Rust-level registry
/// directly, with no Steel VM in play, where settling the (empty) work queue
/// on top would be pointless.
fn drain_sources_until(ed: &mut Editor, mut until: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        ed.drain_async_sources();
        if until(ed) {
            return;
        }
        assert!(Instant::now() < deadline, "condition never became true");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until the open picker's `total_len()` reaches exactly `n` — the
/// row-count wait repeated across every `picker-source-spawn!`/
/// `spawn-async!` picker test. No picker open never satisfies it.
fn drain_until_picker_total(ed: &mut Editor, n: usize) {
    drain_until(ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .map(|p| p.total_len())
            .unwrap_or(0)
            == n
    });
}

/// Whether the open picker no longer has an attached source — the
/// respawn/stop/natural-exit convergence point every
/// `picker-source-spawn!`/`picker-source-stop!` test polls for. A plain
/// predicate rather than its own `drain_*_until` wrapper since callers pass
/// it to either `drain_until` (Steel end-to-end) or `drain_sources_until`
/// (Rust-only, no Steel VM in play) depending on which they're already
/// using.
fn source_detached(ed: &Editor) -> bool {
    ed.state
        .config
        .picker
        .as_ref()
        .is_some_and(|p| !p.has_source())
}

// ── Shared process-liveness helper ──────────────────────────────────────────

/// `kill -0` against the real OS as an independent liveness oracle — never
/// asks the handle itself whether it thinks the child is alive.
fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn kill -0")
        .success()
}

// ── Shared unix-only guards and fixtures ─────────────────────────────────────

/// Claim `Global::Env`, create isolated `runtime` and `tmp` tempdirs, set
/// `HUME_RUNTIME` and `TMPDIR`, and restore both on drop.
///
/// The claim is acquired BEFORE the tempdirs are created so that a concurrent
/// guarded test's TMPDIR does not cause our tempdirs to be nested inside it —
/// which would make them disappear when that test's guard drops and deletes its
/// tree.
struct HumeRuntimeGuard {
    runtime: tempfile::TempDir,
    tmp: tempfile::TempDir,
    // Last field — released after runtime/tmp dirs are deleted.
    _lock: ClaimGuard,
}

impl HumeRuntimeGuard {
    fn new() -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);
        let runtime = safe_tempdir();
        let tmp = safe_tempdir();
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
    _data_tmp: tempfile::TempDir,
    prev_xdg_data_home: Option<String>,
    // Last field — released after `_data_tmp` is deleted (see
    // `HumeRuntimeGuard`'s doc for why the drop order matters).
    _lock: ClaimGuard,
}

impl RealRuntimeGuard {
    fn new() -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);
        let real_runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime");
        let data_tmp = safe_tempdir();
        let prev_xdg_data_home = std::env::var("XDG_DATA_HOME").ok();
        unsafe {
            std::env::set_var("HUME_RUNTIME", real_runtime);
            std::env::set_var("XDG_DATA_HOME", data_tmp.path());
        }
        RealRuntimeGuard {
            _data_tmp: data_tmp,
            prev_xdg_data_home,
            _lock: lock,
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

// ── Shared diagnostics fixture ───────────────────────────────────────────────
//
// `DiagFixture`/`publish_diagnostics_notification` are shared with
// `lsp_diagnostics_nav.rs` too (via `use super::*`). `setup_diagnostics`
// itself is only for `lsp_diagnostics_inline.rs` (EOL summary) and
// `lsp_diagnostic_signs.rs` (gutter signs) — both need `core:lsp` loaded
// *before* `drain_lsp()`, since both decorations are driven by the queued
// `on-diagnostics-changed` hook (see its own doc below).
// `lsp_diagnostics_nav.rs` keeps its own near-identical `setup`: its tests
// pull `diagnostics-for-buffer` fresh at call time, independent of the hook,
// so it loads the plugin *after* `drain_lsp()` instead — a genuine ordering
// difference, not incidental duplication.

use crate::editor::lsp::LspState;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// `((start_line, start_char), (end_line, end_char), severity, message)`.
type DiagFixture<'a> = ((u32, u32), (u32, u32), i64, &'a str);

fn publish_diagnostics_notification(uri: &str, diags: &[DiagFixture]) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = diags
        .iter()
        .map(|((sl, sc), (el, ec), sev, msg)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": sev,
                "message": msg,
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({"uri": uri, "diagnostics": diagnostics}),
    }
}

/// Everything [`setup_diagnostics`] builds and keeps alive for the test's
/// duration. `_dirs` bundles the two owned `TempDir`s (init-eval dir, the
/// on-disk file's dir) into one field so every call site names exactly one
/// keep-alive field, not two — a struct pattern's `..` drops any field it
/// doesn't bind *immediately*, at the `let`, not at the end of the caller's
/// scope, so `_dirs` must always be bound explicitly (`let DiagSetup { mut
/// ed, _guard, _dirs, .. } = setup_diagnostics(...)`), never swallowed by
/// `..`, or the directories `ed`'s buffer and a later `run`/
/// `eval_with_real_host` call still reference vanish out from under them.
/// `file`/`tmp` are plain owned paths (not borrows into `_dirs`) so a caller
/// that needs one doesn't also have to hold a borrow of the whole struct.
struct DiagSetup {
    ed: Editor,
    /// The on-disk path `ed` opened — only
    /// `lsp_diagnostic_signs.rs`'s reload test writes new content to this
    /// and `:e!`s it; every other caller lets it go unread after setup.
    file: std::path::PathBuf,
    /// The Steel init-eval directory — some tests `run` a second plugin
    /// sign source after setup and need this again.
    tmp: std::path::PathBuf,
    _guard: RealRuntimeGuard,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// Plugin load happens *before* `drain_lsp()` (unlike `lsp_diagnostics_nav.rs`'s
/// otherwise-identical `setup`) — both the EOL summary and the gutter signs
/// are driven by `on-diagnostics-changed`, which is a queued hook
/// (`queue_event` → `pending_work`, actually invoked by `settle()`): the
/// handler must be registered by `(load-plugin "core:lsp")` before that
/// queued hook is drained, or the first batch's decorations never render.
fn setup_diagnostics(content: &str, diags: &[DiagFixture]) -> DiagSetup {
    let guard = RealRuntimeGuard::new();
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, content).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    if !diags.is_empty() {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, file.parent().unwrap().to_path_buf()));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib") (load-plugin "core:lsp")"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    ed.drain_lsp();
    ed.settle();

    DiagSetup {
        ed,
        file,
        tmp: tmp.path().to_path_buf(),
        _guard: guard,
        _dirs: (tmp, file_dir),
    }
}

/// Points `XDG_CONFIG_HOME`/`HUME_RUNTIME`/`XDG_DATA_HOME` at a config
/// tempdir (holding a caller-chosen `init.scm`), the real repo `runtime/`
/// dir, and a data tempdir staged with a real compiled grammar at the exact
/// paths core's `grammar-output-path`/`grammar-highlights-path` expect — so
/// `init_scripting`'s unconditional `scheme/grammars.scm` eval (see
/// `scripting_setup.rs`) registers it against the real source catalog.
///
/// Held for the fixture's whole lifetime (unlike `RealRuntimeGuard`, which
/// has no config dir at all) so a later `:reload-config` dispatch can
/// re-enter `init_scripting` against the same paths after `write_init`
/// swaps in a new `init.scm`.
struct StagedGrammarFixture {
    config_dir: PathBuf,
    _config_tmp: tempfile::TempDir,
    _data_tmp: tempfile::TempDir,
    // Last field — released after the tempdirs above are deleted (see
    // `HumeRuntimeGuard`'s doc for why the drop order matters).
    _lock: ClaimGuard,
}

impl StagedGrammarFixture {
    /// `grammar_name`'s compiled fixture library and `highlights.scm` staged
    /// under a fresh `<data>/grammars/`; `init_scm` written to a fresh
    /// `init.scm`. Caller supplies `grammar_name`'s own fixture files —
    /// callers call `require_grammars` first.
    fn new(grammar_name: &str, parser: &Path, highlights: &Path, init_scm: &str) -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);
        let repo_runtime_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime");

        let config_tmp = safe_tempdir();
        let config_dir = config_tmp.path().join("hume");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("init.scm"), init_scm).unwrap();

        let data_tmp = safe_tempdir();
        let grammars_dir = data_tmp.path().join("hume").join("grammars");
        let hl_dir = grammars_dir.join("sources").join(grammar_name);
        std::fs::create_dir_all(&hl_dir).unwrap();
        std::fs::copy(
            parser,
            grammars_dir.join(format!(
                "{grammar_name}.{}",
                hume_test_fixtures::grammar_platform_ext()
            )),
        )
        .unwrap();
        std::fs::copy(highlights, hl_dir.join("highlights.scm")).unwrap();

        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
            std::env::set_var("HUME_RUNTIME", repo_runtime_dir);
            std::env::set_var("XDG_DATA_HOME", data_tmp.path());
        }

        Self {
            config_dir,
            _config_tmp: config_tmp,
            _data_tmp: data_tmp,
            _lock: lock,
        }
    }

    fn write_init(&self, init_scm: &str) {
        std::fs::write(self.config_dir.join("init.scm"), init_scm).unwrap();
    }
}

impl Drop for StagedGrammarFixture {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HUME_RUNTIME");
            std::env::remove_var("XDG_DATA_HOME");
        }
    }
}

/// Runs `git <args>` in `dir`, asserting success — shared by every test
/// fixture that needs a real git repository (`core:pickers`'s git-branch
/// picker, `core:git-diff`'s ref fetch).
fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

/// `git init -q` plus a local commit identity — a fresh sandbox has neither,
/// and `git commit` fails without one.
fn git_init(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
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
    _lock: ClaimGuard,
}

impl CwdSandbox {
    fn new() -> Self {
        let _lock = TEST_GLOBALS.claim(Global::Cwd);
        let saved = std::env::current_dir().expect("current_dir");
        let dir = safe_tempdir();
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
        // Must not swallow a failure here: `dir` deletes unconditionally right
        // after this returns, and a silently-ignored restore would leave cwd
        // dangling in the very directory that's about to disappear.
        std::env::set_current_dir(&self.saved).expect("CwdSandbox restore must not fail");
    }
}

mod alternate;
mod async_job;
mod async_job_steel;
mod buffer;
mod buffer_store;
mod cd;
mod column_display_agreement;
mod command_mode;
mod completion;
mod dot_repeat;
mod file_io;
mod git_diff_plugin;
mod injections_editor;
mod language;
mod list_buffers;
mod lsp_actions;
mod lsp_bridge;
mod lsp_completion_feature;
mod lsp_diagnostic_signs;
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
mod picker_source_steel;
mod pickers_plugin;
mod plugins;
mod reload_config;
mod scripting_effects;
mod scripting_grammar;
mod scripting_lsp_install;
mod scripting_theme_install;
mod sync_dispatch;
mod theme_dirs;
mod tutor;
mod vim_keybind;
