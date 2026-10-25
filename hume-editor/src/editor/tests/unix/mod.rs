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

use hume_platform::process::test_support::is_alive;
use hume_platform::wake::no_op_wake;

// ── Shared async-drain helpers ────────────────────────────────────────────────
//
// Every unix test that waits on a spawned child or streaming source (a
// picker source, a `spawn-async!` job) polls in a bounded loop instead of a
// single drain call. A background thread's result can land on any frame,
// so CI scheduling jitter would flake a "drain once and assert" test.

/// `dir/a/link`, a symlink to `dir/b/c`, so `a/link/..` names `dir/b`.
fn dir_with_a_link_into_a_sibling() -> tempfile::TempDir {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::create_dir_all(dir.path().join("b/c")).unwrap();
    symlink(dir.path().join("b/c"), dir.path().join("a/link")).unwrap();
    dir
}

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

// ── Shared unix-only guards and fixtures ─────────────────────────────────────

/// An isolated, initially empty runtime directory, for a test that stages
/// plugin sources into it. [`Self::dirs`] points a session at it.
struct RuntimeDirs {
    runtime: tempfile::TempDir,
}

impl RuntimeDirs {
    fn new() -> Self {
        Self {
            runtime: tempfile::tempdir().unwrap(),
        }
    }

    fn dirs(&self) -> Dirs {
        Dirs {
            runtime: Some(self.runtime.path().to_path_buf()),
            ..process_dirs()
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

/// Evaluates `init` as an `init.scm` under `dirs` and asserts the load of
/// `core:<plugin>` is contained: `eval_init` returns `Ok`, the plugin is
/// `Failed`, and an `Error` message names it and contains each of `expected`.
/// `eval_init` queues messages on the host and only `Editor::init_scripting`
/// flushes them, so the host's queue is what this reads.
fn assert_core_plugin_load_fails(dirs: Dirs, init: &str, plugin: &str, expected: &[&str]) {
    use hume_scripting::PluginStatus;
    use hume_scripting::attribution::{EntryId, PluginId};

    let tmp = tempfile::tempdir().unwrap();
    let init_path = tmp.path().join("init.scm");
    std::fs::write(&init_path, init).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = dirs;
    let mut host = ScriptingHost::new(&ed.state.dirs);
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("a failed plugin load must be contained, not abort eval_init");

    let name = format!("core:{plugin}");
    assert!(
        host.peek_pending_messages().iter().any(|(level, msg)| {
            matches!(level, hume_scripting::LogLevel::Error)
                && msg.contains(&name)
                && expected.iter().all(|e| msg.contains(e))
        }),
        "an error must name {name} and contain {expected:?}; got {:?}",
        host.peek_pending_messages()
    );
    assert_eq!(
        host.plugin_status(&EntryId::main(PluginId::Core(plugin.to_string()))),
        Some(PluginStatus::Failed),
        "{name} must be marked Failed"
    );
}

/// The real on-disk `runtime/` directory, so multi-file core plugins
/// (`core:lsp`) are tested against the shipped files instead of hand-copied
/// ones, plus a data directory of its own: loading `core:lsp` scans
/// `<data-dir>/servers/` and would otherwise read the developer's installed
/// servers.
struct RealRuntimeDirs {
    _data_tmp: tempfile::TempDir,
    dirs: Dirs,
}

impl RealRuntimeDirs {
    fn new() -> Self {
        let data_tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            data: Some(data_tmp.path().join("hume")),
            runtime: Some(repo_runtime_dir()),
            ..process_dirs()
        };
        Self {
            _data_tmp: data_tmp,
            dirs,
        }
    }

    fn dirs(&self) -> Dirs {
        self.dirs.clone()
    }
}

/// Owns three isolated tempdirs (`config`, `data`, `runtime`) that
/// [`Self::dirs`] points a session at. Unlike
/// `unix::plugins::setup_editor_with_init_scripting`, this fixture stays
/// alive across an initial `init_scripting()` and a later `:reload-config`
/// dispatch against the *same* config path.
struct ReloadFixture {
    config_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    runtime_dir: std::path::PathBuf,
    _config_tmp: tempfile::TempDir,
    _data_tmp: tempfile::TempDir,
    _runtime_tmp: tempfile::TempDir,
}

impl ReloadFixture {
    fn new(init_scm: &str) -> Self {
        let config_tmp = tempfile::tempdir().unwrap();
        let data_tmp = tempfile::tempdir().unwrap();
        let runtime_tmp = tempfile::tempdir().unwrap();
        let config_dir = config_tmp.path().join("hume");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("init.scm"), init_scm).unwrap();
        Self {
            config_dir,
            data_dir: data_tmp.path().join("hume"),
            runtime_dir: runtime_tmp.path().to_path_buf(),
            _config_tmp: config_tmp,
            _data_tmp: data_tmp,
            _runtime_tmp: runtime_tmp,
        }
    }

    fn dirs(&self) -> Dirs {
        Dirs {
            config: Some(self.config_dir.clone()),
            data: Some(self.data_dir.clone()),
            runtime: Some(self.runtime_dir.clone()),
            ..process_dirs()
        }
    }

    fn write_init(&self, init_scm: &str) {
        std::fs::write(self.config_dir.join("init.scm"), init_scm).unwrap();
    }

    /// Write a `--config` override file *outside* `config_dir` (the tmp
    /// root, not the `hume/` subdir), so a test can prove the override (not
    /// the default `init.scm`) is what actually ran.
    fn write_override(&self, name: &str, init_scm: &str) -> std::path::PathBuf {
        let path = self._config_tmp.path().join(name);
        std::fs::write(&path, init_scm).unwrap();
        path
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

/// [`RUST_ANALYZER`], then `core:stdlib` and the real `core:lsp` plugin
/// loaded with `config` as its `#:config`: the init of every rig driving
/// `core:lsp`.
fn core_lsp_init(config: Option<&str>) -> String {
    format!(
        "{RUST_ANALYZER}\n(load-plugin! \"core:stdlib\")\n{}",
        hume_scripting::eager_load_scm("core:lsp", config)
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
    core_lsp_rig_configured(tmp, marked, initialize_result, None, configure)
}

/// [`core_lsp_rig`] with `config` passed to `core:lsp` as its `#:config`.
fn core_lsp_rig_configured(
    tmp: &Path,
    marked: &str,
    initialize_result: serde_json::Value,
    config: Option<&str>,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (LspRig, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();
    let (mut backend, _notifications, _requests) = RecordingLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    configure(&mut backend, ServerId(0));
    let init = core_lsp_init(config);
    let spec = RigSpec::rust(marked)
        .with_init(&init)
        .with_dirs(guard.dirs());
    let mut rig = LspRig::drained(tmp, spec, backend);
    rig.ed.settle();
    (rig, guard)
}

/// Settles, delivers the scripted LSP answers, and settles again: the way
/// the interactive loop drains after every keystroke.
fn pump_lsp(ed: &mut Editor) {
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

/// An LSP `Range` on `line`, from character `start` to `end`.
fn range(line: u64, start: u64, end: u64) -> serde_json::Value {
    serde_json::json!({
        "start": {"line": line, "character": start},
        "end": {"line": line, "character": end}
    })
}

/// An LSP `Location` at the empty range `line:character` of `uri`.
fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({"uri": uri, "range": range(line, character, character)})
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

/// A [`core_lsp_rig`] over `content` with the cursor at its start, answering
/// `initialize` with `capabilities`. A feature driven by server trigger
/// characters (completion, signature help) relies on `on-lsp-attach`'s
/// handler registering them when the rig's drain brings the server to
/// `Running`.
fn setup_cursor_at_start(
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
/// `scripting/init.rs`) registers it against the real language data.
///
/// Held for the whole test so a later `:reload-config` dispatch can re-enter
/// `init_scripting` against the same paths after `write_init` swaps in a new
/// `init.scm`.
struct StagedGrammarFixture {
    config_dir: PathBuf,
    data_dir: PathBuf,
    _config_tmp: tempfile::TempDir,
    _data_tmp: tempfile::TempDir,
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
        }
    }

    fn dirs(&self) -> Dirs {
        Dirs {
            config: Some(self.config_dir.clone()),
            data: Some(self.data_dir.clone()),
            runtime: Some(repo_runtime_dir()),
            ..process_dirs()
        }
    }

    fn write_init(&self, init_scm: &str) {
        std::fs::write(self.config_dir.join("init.scm"), init_scm).unwrap();
    }
}

// ── Shared plugin-loading helpers ─────────────────────────────────────────────
//
// Shared by every test file that loads a real core plugin from disk into an
// isolated data and runtime directory (`scripting_lsp_install/`,
// `scripting_theme_install.rs`, `injections_editor.rs`).

/// Canonicalizes `root` (mirrors what `hume_scripting`'s `ScriptDirs::new`
/// does internally) and returns `<root>/hume`, the actual directory `(data-dir)`
/// resolves to. macOS temp dirs are symlinks (`/var/folders` ->
/// `/private/var/folders`); comparing against the raw tempdir path would
/// mismatch what a registered command's absolute path actually contains.
fn canonical_data_dir(root: &Path) -> PathBuf {
    root.canonicalize().unwrap().join("hume")
}

/// Load the real `core:plum` plugin (plus its `core:stdlib` dependency:
/// `plum/fetch-query!` etc. call `stdlib/find`/`stdlib/write-file!`/
/// `stdlib/delete-dir!`/`stdlib/delete-file!` via `call!`): plugin/grammar/
/// theme management, no LSP awareness at all.
fn load_plum(ed: &mut Editor, data_dir: &Path) {
    load_plum_after(ed, data_dir, "");
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

/// Create a user plugin at `plugins/user/tp/plugin.scm`, write `init.scm`,
/// evaluate it and apply its effects, as startup does, and wire the host
/// into an editor over `text`. The caller keeps the `TempDir` alive.
fn setup_lazy_editor_on(
    text: &str,
    init_body: &str,
    plugin_body: &str,
) -> (Editor, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin_body).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, init_body).unwrap();

    let mut ed = editor_from(text);
    let mut host = ScriptingHost::new(&Dirs {
        data: Some(dir.path().to_path_buf()),
        ..Dirs::none()
    });
    let effects = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed in setup_lazy_editor_on");
    ed.apply_script_effects(effects);
    ed.scripting = Some(host);
    (ed, dir)
}

/// [`setup_lazy_editor_on`] over `-[a]>b`.
fn setup_lazy_editor(init_body: &str, plugin_body: &str) -> (Editor, tempfile::TempDir) {
    setup_lazy_editor_on("-[a]>b\n", init_body, plugin_body)
}

mod alternate;
mod async_job;
mod async_job_steel;
mod before_save;
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
mod language_config;
mod lsp_actions;
mod lsp_bridge;
mod lsp_completion_feature;
mod lsp_diagnostic_signs;
mod lsp_diagnostics_ingest;
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
mod lsp_symbols;
mod multi_pane;
mod picker_source;
mod picker_source_steel;
mod pickers_plugin;
mod pipe;
mod plugins;
mod reload_config;
mod scripting_effects;
mod scripting_grammar;
mod scripting_lsp_install;
mod scripting_theme_install;
mod search_path;
mod sync_dispatch;
mod theme_dirs;
mod tutor;
mod undotree_plugin;
mod vim_keybind;
