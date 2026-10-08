//! Tests that cannot run on Windows, gated once at the `mod unix;`
//! declaration in the parent: files in here need no `#[cfg]` attributes.
//!
//! Most tests here load Steel plugins from disk: Scheme `require` strings
//! embed OS paths, and backslashes are not escaped in Steel string literals.
//! The rest exercise unix-only behavior directly (e.g. `set_cwd` against
//! canonicalized paths).
//!
//! A test file with both portable and unix-only tests is split into a
//! same-named file here holding the unix-only half.

use super::*;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ── Shared async-drain helpers ────────────────────────────────────────────────
//
// Every unix test that waits on a spawned child or streaming source (a
// picker source, a `spawn-async!` job) polls in a bounded loop instead of a
// single drain call. A background thread's result can land on any frame,
// so CI scheduling jitter would flake a "drain once and assert" test.

/// Runs `step` in a bounded loop until `until` returns true.
fn poll_until(
    ed: &mut Editor,
    mut step: impl FnMut(&mut Editor),
    mut until: impl FnMut(&Editor) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        step(ed);
        if until(ed) {
            return;
        }
        assert!(Instant::now() < deadline, "condition never became true");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Drains async sources and their queued Steel callbacks/events in a bounded
/// loop until `until` returns true. `settle()` already covers both (see its
/// doc), so this is a single call, not two.
fn drain_until(ed: &mut Editor, until: impl FnMut(&Editor) -> bool) {
    poll_until(ed, Editor::settle, until);
}

/// [`drain_until`], drawing a frame each pass (`render`, which settles
/// first), for state only a frame produces, such as a lazy drawer's rows.
fn drain_frames_until(ed: &mut Editor, until: impl FnMut(&Editor) -> bool) {
    poll_until(ed, render, until);
}

/// Same loop as [`drain_until`], but calls `drain_async_sources` directly
/// instead of `settle()`, for tests that drive the Rust-level registry
/// directly, with no Steel VM in play, where settling the (empty) work queue
/// on top would be pointless.
fn drain_sources_until(ed: &mut Editor, until: impl FnMut(&Editor) -> bool) {
    poll_until(ed, Editor::drain_async_sources, until);
}

/// Waits until the open picker's `total_len()` reaches exactly `n`: the
/// row-count wait repeated across every `picker-source-spawn!`/
/// `spawn-async!` picker test. No picker open never satisfies it.
fn drain_until_picker_total(ed: &mut Editor, n: usize) {
    drain_until(ed, |ed| {
        ed.state.input.picker().map(|p| p.total_len()).unwrap_or(0) == n
    });
}

/// Whether the open picker no longer has an attached source: the
/// respawn/stop/natural-exit convergence point every
/// `picker-source-spawn!`/`picker-source-stop!` test polls for. A plain
/// predicate rather than its own `drain_*_until` wrapper since callers pass
/// it to either `drain_until` (Steel end-to-end) or `drain_sources_until`
/// (Rust-only, no Steel VM in play) depending on which they're already
/// using.
fn source_detached(ed: &Editor) -> bool {
    ed.state.input.picker().is_some_and(|p| !p.has_source())
}

// ── Shared process-liveness helper ──────────────────────────────────────────

/// `kill -0` against the real OS as an independent liveness check. It never
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

/// An isolated, initially empty runtime directory, for a test that stages
/// plugin sources into it. [`Self::dirs`] points a session at it.
struct RuntimeDirs {
    runtime: tempfile::TempDir,
    _path: PathReader,
}

impl RuntimeDirs {
    fn new() -> Self {
        Self {
            runtime: tempfile::tempdir().unwrap(),
            _path: path_reader(),
        }
    }

    fn dirs(&self) -> Dirs {
        Dirs {
            runtime: Some(self.runtime.path().to_path_buf()),
            ..Dirs::none()
        }
    }
}

/// The real shipped `core:stdlib` plugin source, embedded so tests exercise
/// the actual file rather than a hand-rolled stand-in.
const STDLIB_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/stdlib/plugin.scm"
));

/// Stage a real shipped core plugin's source into `dirs`' isolated
/// `<runtime>/plugins/core/<name>/plugin.scm`, so `load-plugin!` resolves it
/// as a core plugin during the test.
fn write_core_plugin(dirs: &RuntimeDirs, name: &str, source: &str) {
    let plugin_dir = dirs.runtime.path().join("plugins").join("core").join(name);
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), source).unwrap();
}

/// The real on-disk `runtime/` directory, so multi-file core plugins
/// (`core:lsp`) are tested against the shipped files instead of hand-copied
/// ones, plus a data directory of its own: loading `core:lsp` scans
/// `<data-dir>/servers/` and would otherwise read the developer's installed
/// servers.
struct RealRuntimeDirs {
    _data_tmp: tempfile::TempDir,
    dirs: Dirs,
    _path: PathReader,
}

impl RealRuntimeDirs {
    fn new() -> Self {
        let data_tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            data: Some(data_tmp.path().join("hume")),
            runtime: Some(repo_runtime_dir()),
            ..Dirs::none()
        };
        Self {
            _data_tmp: data_tmp,
            dirs,
            _path: path_reader(),
        }
    }

    fn dirs(&self) -> Dirs {
        self.dirs.clone()
    }
}

// ── Shared LSP fixtures ──────────────────────────────────────────────────────
//
// Every fixture here opens its file through `LspRig`, with the real
// `core:lsp` plugin loaded by the rig's init, before the file opens: the
// plugin's `on-lsp-attach` and `on-diagnostics-changed` handlers are
// installed by the time the attach and the first publish fire them.

use super::lsp_rig::{LspRig, RUST_ANALYZER, RigSpec, file_uri};
use crate::editor::lsp::LspState;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};
use hume_scripting::ScriptingHost;

/// [`RUST_ANALYZER`], then `core:stdlib` and the real `core:lsp` plugin:
/// the init of every rig driving `core:lsp`.
fn core_lsp_init() -> String {
    format!(
        "{RUST_ANALYZER}\n(load-plugin! \"core:stdlib\")\n{}",
        hume_scripting::eager_load_scm("core:lsp", None)
    )
}

/// The canonical rig root for `tmp`, the directory every extra fixture file
/// of a [`core_lsp_rig`] test lives in.
fn rig_root(tmp: &Path) -> PathBuf {
    std::fs::canonicalize(tmp).unwrap()
}

/// The URI of the file a [`RigSpec::rust`] rig rooted at `tmp` opens, for a
/// response scripted before the rig exists.
fn rust_rig_uri(tmp: &Path) -> String {
    file_uri(&rig_root(tmp).join("src/main.rs"))
}

/// A drained [`RigSpec::rust`] rig at `tmp` over the real `core:lsp`
/// plugin, its one `rust-analyzer` answering `initialize` with
/// `initialize_result`. `configure` scripts the backend before anything is
/// sent; the server it is handed is the id the rig's one server spawns as.
fn core_lsp_rig(
    tmp: &Path,
    marked: &str,
    initialize_result: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (LspRig, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();
    let (mut backend, _notifications, _requests) = RecordingLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    configure(&mut backend, ServerId(0));
    let init = core_lsp_init();
    let spec = RigSpec::rust(marked)
        .with_init(&init)
        .with_dirs(guard.dirs());
    let mut rig = LspRig::drained(tmp, spec, backend);
    rig.ed.settle();
    (rig, guard)
}

/// A trigger-char fixture's text: an `Insert` cursor at its newline sits
/// right after "foo", ready to type a trigger char.
const FOO: &str = "foo\n";

/// `content` as `editor_from`-style marked text, the cursor on its first
/// cluster.
fn marked_at_start(content: &str) -> String {
    let first = hume_rope::grapheme::next_str_boundary(content, 0);
    format!("-[{}]>{}", &content[..first], &content[first..])
}

/// A [`core_lsp_rig`] over `content` (cursor at its start), for a feature
/// driven by server trigger characters (completion, signature help):
/// `on-lsp-attach`'s handler registers them when the rig's drain brings the
/// server to `Running`.
fn setup_trigger_char_feature(
    tmp: &Path,
    content: &str,
    capabilities: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs, RequestLog) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        &marked_at_start(content),
        serde_json::json!({ "capabilities": capabilities }),
        configure,
    );
    (rig.ed, guard, rig.requests)
}

/// How many requests logged in `requests` were sent for `method`.
fn request_count(requests: &RequestLog, method: &str) -> usize {
    requests
        .borrow()
        .iter()
        .filter(|(_sid, m, _params)| m == method)
        .count()
}

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
/// duration. A struct pattern's `..` drops any field it doesn't bind at the
/// `let`, not at the end of the caller's scope, so `_root` must always be
/// bound explicitly (`let DiagSetup { mut ed, _guard, _root, .. } =
/// setup_diagnostics(...)`), or the directory holding `ed`'s file and init
/// vanishes out from under it.
struct DiagSetup {
    ed: Editor,
    /// The on-disk path `ed` opened.
    file: std::path::PathBuf,
    /// The rig root, where a later `run`/`eval_with_real_host` writes its
    /// init.
    tmp: std::path::PathBuf,
    /// The one server attached to `file`, the publisher of `diags`.
    sid: ServerId,
    _guard: RealRuntimeDirs,
    _root: tempfile::TempDir,
}

/// A [`core_lsp_rig`] over `content` (cursor at its start) whose server
/// publishes `diags` for the file once it is `Running`, settled so the
/// queued `on-diagnostics-changed` hook has drawn its decorations.
fn setup_diagnostics(content: &str, diags: &[DiagFixture]) -> DiagSetup {
    let root = tempfile::tempdir().unwrap();
    let (mut rig, guard) = core_lsp_rig(
        root.path(),
        &marked_at_start(content),
        serde_json::json!({ "capabilities": {} }),
        |_, _| {},
    );
    let sid = rig.sid("rust-analyzer");
    if !diags.is_empty() {
        let msg = publish_diagnostics_notification(&rig.uri(), diags);
        rig.push(sid, msg);
        rig.ed.settle();
    }
    let file = rig
        .ed
        .state
        .buffers
        .get(rig.bid)
        .path()
        .unwrap()
        .to_path_buf();
    DiagSetup {
        ed: rig.ed,
        file,
        tmp: rig.root,
        sid,
        _guard: guard,
        _root: root,
    }
}

/// A config tempdir (holding a caller-chosen `init.scm`), the real repo
/// `runtime/` dir, and a data tempdir staged with a real compiled grammar at
/// the exact paths core's `grammar-output-path`/`grammar-highlights-path`
/// expect, so `init_scripting`'s unconditional `scheme/grammars.scm` eval (see
/// `scripting_setup.rs`) registers it against the real source catalog.
///
/// Held for the whole test so a later `:reload-config` dispatch can re-enter
/// `init_scripting` against the same paths after `write_init` swaps in a new
/// `init.scm`.
struct StagedGrammarFixture {
    config_dir: PathBuf,
    data_dir: PathBuf,
    _config_tmp: tempfile::TempDir,
    _data_tmp: tempfile::TempDir,
    _path: PathReader,
}

impl StagedGrammarFixture {
    /// `grammar_name`'s compiled fixture library and `highlights.scm` staged
    /// under a fresh `<data>/grammars/`; `init_scm` written to a fresh
    /// `init.scm`. Caller supplies `grammar_name`'s own fixture files;
    /// callers call `require_grammars` first.
    fn new(grammar_name: &str, parser: &Path, highlights: &Path, init_scm: &str) -> Self {
        let config_tmp = tempfile::tempdir().unwrap();
        let config_dir = config_tmp.path().join("hume");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("init.scm"), init_scm).unwrap();

        let data_tmp = tempfile::tempdir().unwrap();
        let data_dir = data_tmp.path().join("hume");
        let grammars_dir = data_dir.join("grammars");
        let hl_dir = grammars_dir.join("sources").join(grammar_name);
        std::fs::create_dir_all(&hl_dir).unwrap();
        std::fs::copy(
            parser,
            grammars_dir.join(format!(
                "{grammar_name}.{}",
                test_fixtures::grammar_platform_ext()
            )),
        )
        .unwrap();
        std::fs::copy(highlights, hl_dir.join("highlights.scm")).unwrap();

        Self {
            config_dir,
            data_dir,
            _config_tmp: config_tmp,
            _data_tmp: data_tmp,
            _path: path_reader(),
        }
    }

    fn dirs(&self) -> Dirs {
        Dirs {
            config: Some(self.config_dir.clone()),
            data: Some(self.data_dir.clone()),
            runtime: Some(repo_runtime_dir()),
            tmp: None,
        }
    }

    fn write_init(&self, init_scm: &str) {
        std::fs::write(self.config_dir.join("init.scm"), init_scm).unwrap();
    }
}

/// Runs `git <args>` in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

/// `git init -q` plus a local commit identity: a fresh sandbox has neither,
/// and `git commit` fails without one.
fn git_init(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
}

// ── Shared plugin-loading helpers ─────────────────────────────────────────────
//
// Shared by every test file that loads a real core plugin from disk into an
// isolated data and runtime directory (`scripting_lsp_install.rs`,
// `scripting_theme_install.rs`, `injections_editor.rs`).

/// Canonicalizes `root` (mirrors what `hume_scripting`'s `ScriptDirs::new`
/// does internally) and returns `<root>/hume`, the actual directory `(data-dir)`
/// resolves to. macOS temp dirs are symlinks (`/var/folders` ->
/// `/private/var/folders`); comparing against the raw tempdir path would
/// mismatch what a registered command's absolute path actually contains.
fn canonical_data_dir(root: &Path) -> PathBuf {
    root.canonicalize().unwrap().join("hume")
}

/// Load `init_src` into `ed`, pointing its runtime directory at the repo's
/// real `runtime/` dir (so the real shipped plugin sources and catalogs are
/// used) and its data directory at `<data_dir>/hume`.
fn load_with_init(ed: &mut Editor, data_dir: &Path, init_src: &str) {
    load_with_init_in_runtime(ed, &repo_runtime_dir(), data_dir, init_src);
}

/// [`load_with_init`] against an explicit runtime directory, for a test
/// that fabricates part of the runtime (e.g. the `core:lsp-install` source catalog).
fn load_with_init_in_runtime(ed: &mut Editor, runtime_dir: &Path, data_dir: &Path, init_src: &str) {
    let config_tmp = tempfile::tempdir().unwrap();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_src).unwrap();
    // `(data-dir)` is canonicalized only when the directory exists, and the
    // tests compare against `canonical_data_dir`.
    std::fs::create_dir_all(data_dir.join("hume")).unwrap();

    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_dir.join("hume")),
        runtime: Some(runtime_dir.to_path_buf()),
        tmp: None,
    };
    ed.init_scripting(&mut Default::default());
    // `config_tmp` is deleted on return, so a later `:reload-config` has to
    // find no config directory rather than a dangling one.
    ed.state.dirs.config = None;
}

/// Load the real `core:plum` plugin (plus its `core:stdlib` dependency:
/// `plum/fetch-query!` etc. call `stdlib/find`/`stdlib/write-file!`/
/// `stdlib/delete-dir!`/`stdlib/delete-file!` via `call!`): plugin/grammar/
/// theme management, no LSP awareness at all (servers.scm lives entirely in
/// core:lsp now).
fn load_plum(ed: &mut Editor, data_dir: &Path) {
    load_with_init(
        ed,
        data_dir,
        &format!(
            "(load-plugin! \"core:stdlib\")\n{}\n\
             (%activate-plugin-inline! \"core:plum\" \"grammars.scm\")\n\
             (%activate-plugin-inline! \"core:plum\" \"themes.scm\")",
            hume_scripting::eager_load_scm("core:plum", None)
        ),
    );
}

/// Load the real `core:lsp-install` plugin only (plus its documented
/// `core:stdlib` dependency): the entire LSP server lifecycle (install,
/// uninstall, listing, registration of installed servers), each entry
/// activating on first use.
fn load_lsp_install(ed: &mut Editor, data_dir: &Path) {
    load_with_init(
        ed,
        data_dir,
        "(load-plugin! \"core:stdlib\")\n(load-plugin! \"core:lsp-install\")",
    );
}

/// Eagerly load `core:lsp-install`'s `plugin.scm` (plus `core:stdlib`): the
/// startup scan runs during init, and the install commands stay undefined.
fn load_lsp_install_eager(ed: &mut Editor, data_dir: &Path) {
    load_with_init(
        ed,
        data_dir,
        &format!(
            "(load-plugin! \"core:stdlib\")\n{}",
            hume_scripting::eager_load_scm("core:lsp-install", None)
        ),
    );
}

/// Load the real `core:lsp` plugin only (plus its documented `core:stdlib`
/// dependency): the language-server client, with no install lifecycle.
fn load_lsp(ed: &mut Editor, data_dir: &Path) {
    load_with_init(
        ed,
        data_dir,
        &format!(
            "(load-plugin! \"core:stdlib\")\n{}",
            hume_scripting::eager_load_scm("core:lsp", None)
        ),
    );
}

/// A tempdir and its canonical path, for tests that point the editor's cwd at it.
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// Raw tempdir path; build child dirs/files under this.
    fn raw(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Canonicalized tempdir path (macOS /var → /private/var) for cwd asserts.
    fn path(&self) -> PathBuf {
        std::fs::canonicalize(self.dir.path()).expect("canonicalize")
    }
}

mod alternate;
mod async_job;
mod async_job_steel;
mod buffer;
mod buffer_store;
mod buffer_words_plugin;
mod cd;
mod column_display_agreement;
mod command_mode;
mod completion;
mod dot_repeat;
mod editor_cwd;
mod file_io;
mod git_diff_plugin;
mod injections_editor;
mod language;
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
mod lsp_locations_refresh;
mod lsp_multi_server;
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
mod undotree_plugin;
mod vim_keybind;
