//! End-to-end wiring test for `:reload-config`.
//!
//! `Editor::reset_config_state`'s per-surface contract (each config-owned
//! piece of state reverting to its compiled-in default) is tested in the
//! portable `tests/reload_config.rs`, which never touches `config_dir()`/
//! `XDG_CONFIG_HOME` and so needs no OS-path isolation. This file has the
//! one test that proves the actual `:reload-config` typed command wires
//! that reset to a real `init.scm` reload on disk.

use super::*;

use super::super::scripting_grammar::grammar_fixture;
use crate::editor::keymap::{BindMode, Keymap};
use crate::editor::minibuf::history::HistoryKind;

/// Owns three isolated tempdirs (`config`, `data`, `runtime`) and keeps
/// `XDG_CONFIG_HOME`/`HUME_RUNTIME`/`XDG_DATA_HOME` pointed at them for its
/// whole lifetime — unlike `unix::plugins::setup_editor_with_init_scripting`,
/// which unsets the env vars right after its one `init_scripting()` call,
/// this fixture stays alive across an initial `init_scripting()` and a later
/// `:reload-config` dispatch against the *same* config path.
struct ReloadFixture {
    config_dir: std::path::PathBuf,
    _config_tmp: tempfile::TempDir,
    _data_tmp: tempfile::TempDir,
    _runtime_tmp: tempfile::TempDir,
    // Last field — released after the tempdirs above are deleted (see
    // `HumeRuntimeGuard`'s doc for why the drop order matters).
    _lock: ClaimGuard,
}

impl ReloadFixture {
    fn new(init_scm: &str) -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);
        let config_tmp = safe_tempdir();
        let data_tmp = safe_tempdir();
        let runtime_tmp = safe_tempdir();
        let config_dir = config_tmp.path().join("hume");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("init.scm"), init_scm).unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
            std::env::set_var("HUME_RUNTIME", runtime_tmp.path());
            std::env::set_var("XDG_DATA_HOME", data_tmp.path());
        }
        Self {
            config_dir,
            _config_tmp: config_tmp,
            _data_tmp: data_tmp,
            _runtime_tmp: runtime_tmp,
            _lock: lock,
        }
    }

    fn write_init(&self, init_scm: &str) {
        std::fs::write(self.config_dir.join("init.scm"), init_scm).unwrap();
    }
}

impl Drop for ReloadFixture {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HUME_RUNTIME");
            std::env::remove_var("XDG_DATA_HOME");
        }
    }
}

/// `:reload-config`, dispatched through the real minibuffer path against a
/// real `init.scm` on disk: a bound key and a global option revert to their
/// compiled-in defaults, the reloaded file's own `define-command!` for the
/// same name registers cleanly (the builtin-conflict guard `reset_config_state`'s
/// doc warns about), and no error is logged.
#[test]
fn reload_config_command_resets_state_from_a_real_init_scm() {
    let fixture = ReloadFixture::new(
        r#"(bind-key! 'normal "Q" "move-down")
           (set-option! "scrolloff" 9)
           (define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')]),
        Some(("move-down".to_string(), false)),
        "sanity: the initial init.scm must have applied"
    );
    assert_eq!(
        ed.state.settings.scrolloff, 9,
        "sanity: the initial init.scm must have applied"
    );
    assert!(
        ed.state.config.registry.contains("bar"),
        "sanity: the initial init.scm must have applied"
    );

    fixture.write_init(r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#);
    type_cmd(&mut ed, ":reload-config");

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')]),
        Keymap::default().lookup_command(BindMode::Normal, &[key('Q')]),
        "bind-key! from the old init.scm must not survive the reload"
    );
    assert_eq!(
        ed.state.settings.scrolloff,
        EditorSettings::default().scrolloff,
        "set-option! from the old init.scm must not survive the reload"
    );
    assert!(
        ed.state.config.registry.contains("bar"),
        "the reloaded init.scm's own define-command! must register cleanly, \
         not trip the builtin-conflict check against the dropped command"
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a real reload must not log any error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace),
        "sanity: init_scripting's routine 'runtime dir = …'/'data dir = …' \
         Trace lines must actually have logged something here, or the \
         assertion below (success despite non-empty Trace-only output) \
         proves nothing"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Config reloaded"),
        "a reload whose only new log output is Trace-level must still \
         report success — regression test for MessageLog::totals \
         deliberately excluding Trace/Info from the before/after \
         comparison typed_reload_config gates on"
    );
}

/// A `set-buffer-option!` written from an `on-language-set` hook — the
/// pattern `user-manual/docs/plugins.md` documents — must still be in
/// effect after `:reload-config`, not silently revert: `reset_config_state`
/// clears the buffer's language identity (see `clear_languages_all`)
/// precisely so `init_scripting`'s post-reload re-detect sweep is a real
/// `None -> Some` transition and the hook fires again, instead of hitting
/// `set_buffer_language`'s unchanged-value early return.
///
/// The fixture's `HUME_RUNTIME` points at an empty tempdir, so neither the
/// real `runtime/scheme/languages.scm` nor `prelude.scm` loads — hence the
/// raw `%define-language!` call below (the ergonomic `define-language!` is
/// a `prelude.scm` macro, unavailable here). The test's own registration is
/// what makes `"rust"` detectable at all.
#[test]
fn reload_config_reapplies_on_language_set_buffer_overrides() {
    let init_scm = r#"(%define-language! "rust" '("rs") '() '() #f)
        (register-hook! 'on-language-set (lambda (bid lang)
          (when (equal? lang "rust") (set-buffer-option! bid "tab-width" 7))))"#;
    let fixture = ReloadFixture::new(init_scm);

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    ed.init_scripting(&mut Default::default());
    ed.settle();

    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state.buffers.get(bid).overrides.tab_width,
        Some(7),
        "sanity: the initial init.scm's on-language-set hook must apply"
    );

    fixture.write_init(init_scm);
    type_cmd(&mut ed, ":reload-config");
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).overrides.tab_width,
        Some(7),
        "on-language-set's set-buffer-option! must reapply after reload — the \
         language didn't 'change' in the sense the buffer notices, but the \
         config that set it was just re-run"
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a real reload must not log any error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A buffer opened by a lazy `#:languages` plugin's activation body during
/// `:reload-config`'s own `init_scripting()` call must not get
/// `OnBufferOpen` fired twice — see the portable `tests/reload_config.rs`'s
/// `resync_does_not_refire_buffer_open_for_a_buffer_opened_by_this_reload`
/// for why `open-buffer!` is callable here at all (`EvalMode::Init` normally
/// rejects it). Here the double-fire risk is the ordinary open path
/// (`activate_and_register`'s own `apply_script_effects` →
/// `detect_pending_languages`, nested inside the same `detect_and_set_language`
/// call that activated the plugin, so it runs before `init_scripting` even
/// returns) against `resync_config_state`'s blanket re-fire loop, which runs
/// after.
///
/// `:reload-config` is dispatched as the *first* `init_scripting()` call (no
/// prior one) so the plugin's `open-buffer!` genuinely opens a new buffer
/// here rather than deduping against one from an earlier init.
///
/// Counts via a `tab-width` override incremented once per fire — a plain
/// "did it fire at all" check can't distinguish once from twice.
#[test]
fn reload_config_does_not_double_fire_buffer_open_for_a_plugin_opened_buffer() {
    let file_tmp = safe_tempdir();
    let companion = file_tmp.path().join("companion.rs");
    std::fs::write(&companion, "fn companion() {}\n").unwrap();
    let companion_str = companion.to_string_lossy().replace('\\', "/");

    let init_scm = r#"(%define-language! "rust" '("rs") '() '() #f)
        (declare-plugin "user/opener" #:languages '("rust"))"#;
    let fixture = ReloadFixture::new(init_scm);

    let plugin_dir = fixture
        ._data_tmp
        .path()
        .join("hume")
        .join("plugins")
        .join("user")
        .join("opener");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        format!(
            r#"(register-hook! 'on-buffer-open (lambda (bid)
                 (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))
               (open-buffer! "{companion_str}")"#
        ),
    )
    .unwrap();

    let file = file_tmp.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    type_cmd(&mut ed, ":reload-config");
    ed.settle();

    let companion_bid = ed
        .state
        .buffers
        .find_by_path(&companion.canonicalize().unwrap())
        .expect("the plugin-opened buffer must be open");
    assert_eq!(
        ed.state.buffers.get(companion_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-buffer-open must fire exactly once for the buffer this reload's own plugin \
         opened, not twice"
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a real reload must not log any error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

/// A `:reload-config` whose new `init.scm` fails to evaluate must not claim
/// success — regression test for `typed_reload_config` unconditionally
/// reporting `Severity::Info, "Config reloaded"` after `init_scripting()`,
/// which overwrote the `status_msg` that `init_scripting`'s own
/// `Severity::Error` report had just set.
#[test]
fn reload_config_does_not_report_success_when_init_scm_errors() {
    let fixture = ReloadFixture::new(r#"(set-option! "scrolloff" 9)"#);
    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());
    assert_eq!(
        ed.state.settings.scrolloff, 9,
        "sanity: the initial init.scm must have applied"
    );

    fixture.write_init("(this-function-does-not-exist)");
    type_cmd(&mut ed, ":reload-config");

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a broken init.scm must log an error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    assert_ne!(
        ed.state.status_msg.as_deref(),
        Some("Config reloaded"),
        "the reload must not report success when init.scm failed to evaluate"
    );
}

/// A `:set buffer language=` assertion on a buffer whose language can never
/// be auto-detected (no matching extension, glob, or shebang) must survive
/// `:reload-config` — regression test for `reset_config_state` clearing
/// `Buffer.language` and letting the post-reload re-detect sweep's plain
/// detection be the only thing that repopulates it, silently dropping an
/// explicit assertion detection could never have produced in the first place.
#[test]
fn reload_config_restores_an_explicit_buffer_language_detection_cannot_recover() {
    let init_scm = r#"(%define-language! "notes" '() '() '() #f)"#;
    let fixture = ReloadFixture::new(init_scm);

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("README"); // no extension: never auto-detected
    std::fs::write(&file, "hello\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    ed.init_scripting(&mut Default::default());

    let bid = ed.focused_buffer_id();
    type_cmd(&mut ed, ":set buffer language=notes");
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .language
            .map(|id| ed.state.config.languages.name_of(id)),
        Some("notes"),
        "sanity: the explicit :set must have applied"
    );

    fixture.write_init(init_scm);
    type_cmd(&mut ed, ":reload-config");

    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .language
            .map(|id| ed.state.config.languages.name_of(id)),
        Some("notes"),
        "an explicit :set buffer language= assertion must survive :reload-config even \
         though detection alone can never recover it for this file"
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a real reload must not log any error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// :reload-config must not drop a startup-registered grammar
// ---------------------------------------------------------------------------

/// `init_scripting`'s unconditional `scheme/grammars.scm` eval is the only
/// place a grammar gets registered, and `:reload-config` re-enters it after
/// `reset_config_state` tears down the previous `LanguageRegistry`
/// (`clear_languages_all`). Regression coverage for `130d0e4e`, which fixed
/// `clear_languages_all` leaving `Buffer.syntax` pointed at an
/// `Arc<GrammarBundle>` from the discarded registry when a buffer's
/// language failed to re-detect after reload.
///
/// Uses a real compiled JSON grammar staged under `StagedGrammarFixture`
/// (real `HUME_RUNTIME`, so the real `grammar-sources.scm` catalog and
/// `grammars.scm` registrar run both times) with no `core:plum` in
/// `init.scm` at all — proving the survival is core's doing, not a reload
/// re-running an install command.
///
/// Flip: revert `130d0e4e`'s `buf.syntax = None` addition to
/// `clear_languages_all` and the final `syntax.is_some()` assertion fails —
/// the buffer keeps reparsing forever against a registry that no longer
/// exists.
#[test]
fn reload_config_keeps_a_startup_grammar_registered() {
    if hume_test_fixtures::skip_unless_grammars(&["json"]) {
        return;
    }
    let (parser, hl) = grammar_fixture("json");
    let fixture = StagedGrammarFixture::new("json", &parser, &hl, "");

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("data.json");
    std::fs::write(&file, "{\"x\": 1}\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    ed.init_scripting(&mut Default::default());

    let bid = ed.focused_buffer_id();
    assert!(
        ed.state.config.languages.has_grammar("json"),
        "sanity: the initial init_scripting must have registered the grammar"
    );
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "sanity: the buffer must be highlighted before reload"
    );

    fixture.write_init("");
    type_cmd(&mut ed, ":reload-config");
    drop(fixture);

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a real reload must not log any error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    assert!(
        ed.state.config.languages.has_grammar("json"),
        "the grammar must still be registered after :reload-config re-enters init_scripting"
    );
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "the buffer must still be highlighted after :reload-config"
    );
}

// ---------------------------------------------------------------------------
// :reload-config with no resolvable config directory fails fast
// ---------------------------------------------------------------------------

/// RAII guard: unsets `XDG_CONFIG_HOME` and `HOME` for its lifetime (the
/// only two env vars `hume_platform::dirs::config_dir()` ever consults on
/// Unix), restoring each to its original value on drop rather than just
/// removing it — other tests serialized behind the same `TEST_GLOBALS` claim
/// depend on `HOME` being set again afterward.
struct NoConfigDirGuard {
    _xdg_config_home: EnvVarGuard,
    _home: EnvVarGuard,
    // Last field — released after both vars above are restored (fields drop
    // in declaration order; see `HumeRuntimeGuard`'s doc for why the order
    // matters here too).
    _lock: ClaimGuard,
}

impl NoConfigDirGuard {
    fn new() -> Self {
        let lock = TEST_GLOBALS.claim(Global::Env);
        // `capture` (not `set`) — the mutation here is `remove_var`, not a
        // new value, so only the restore-on-drop half applies.
        let xdg_config_home = EnvVarGuard::capture("XDG_CONFIG_HOME");
        let home = EnvVarGuard::capture("HOME");
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
        Self {
            _xdg_config_home: xdg_config_home,
            _home: home,
            _lock: lock,
        }
    }
}

/// Regression test for the half-reset `init_scripting` used to leave behind
/// when `config_dir()` resolved to `None` mid-reload: `reset_config_state`
/// would already have wiped languages/keymap/theme/highlighting before
/// `init_scripting`'s own `None`-directory early return, permanently
/// degrading the editor with `scripting` left `None`. `typed_reload_config`
/// now checks `config_dir()` *before* touching anything, so this asserts
/// nothing was touched at all: the error is reported, and a live config
/// override (a bound key) survives untouched.
#[test]
fn reload_config_with_no_config_dir_fails_fast_and_resets_nothing() {
    let _guard = NoConfigDirGuard::new();

    let mut ed = editor_from("-[a]>b\n");
    // No scripting host at all — mirrors what a real editor looks like when
    // `Editor::open`'s caller never resolved a config dir either. Still has
    // a live keymap override from `Editor::open`'s own `ConfigState::new`,
    // which the failed reload must leave untouched.
    ed.state.config.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[key('Q')],
        "move-down".into(),
        false,
    );
    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')]),
        Some(("move-down".to_string(), false)),
        "sanity: the override must be live before the failed reload"
    );

    type_cmd(&mut ed, ":reload-config");

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')]),
        Some(("move-down".to_string(), false)),
        "a reload that fails before touching anything must leave the live \
         keymap override in place, not reset it and then fail to reload"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "the failure must be reported, not silent"
    );
    assert_ne!(
        ed.state.status_msg.as_deref(),
        Some("Config reloaded"),
        "a failed reload must not report success"
    );
}

// ---------------------------------------------------------------------------
// :reload-config twice in a row
// ---------------------------------------------------------------------------

/// Two reloads back to back must both fully apply the (changing) config on
/// disk — regression coverage for state that's `mem::take`n exactly once
/// per reload (`ReloadSnapshot::take_explicit_languages`) or recomputed
/// fresh each call (`pre_reload_bids`/`buffer_stamps`): either forgetting to
/// reset between calls, or a snapshot silently going stale on the second
/// pass, would surface here.
#[test]
fn reload_config_twice_in_a_row_both_apply_cleanly() {
    let fixture = ReloadFixture::new(r#"(set-option! "scrolloff" 3)"#);
    let mut ed = editor_from("-[a]>b\n");
    ed.init_scripting(&mut Default::default());
    assert_eq!(
        ed.state.settings.scrolloff, 3,
        "sanity: first init must apply"
    );

    fixture.write_init(r#"(set-option! "scrolloff" 5)"#);
    type_cmd(&mut ed, ":reload-config");
    assert_eq!(
        ed.state.settings.scrolloff, 5,
        "first reload must apply the updated value"
    );

    fixture.write_init(r#"(set-option! "scrolloff" 9)"#);
    type_cmd(&mut ed, ":reload-config");
    assert_eq!(
        ed.state.settings.scrolloff, 9,
        "second reload must apply the value again, not reuse stale state \
         from the first reload"
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "neither reload may log an error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// :reload-config's explicit-language restore vs. an in-place buffer swap
// ---------------------------------------------------------------------------

/// Regression test: `close_buffer`'s last-buffer branch reuses the closed
/// buffer's `BufferId` in place for a fresh scratch buffer — a versioned
/// slotmap key alone can't tell that apart from "the same buffer the
/// snapshot meant" (see `Buffer::replace_stamp`'s doc). Before that stamp
/// existed, the explicit-language snapshot's liveness check was a bare
/// `buffers.try_get(bid)`, which the fresh scratch buffer also passes — so
/// a reload landing after such a swap would apply the *closed* buffer's
/// language onto unrelated scratch content.
///
/// Drives `reset_config_state`/`init_scripting` directly (not the full
/// `:reload-config` command) so the in-place swap can be simulated
/// deterministically — bumping `replace_stamp` by hand — rather than
/// depending on exact Steel plugin-activation reentrancy timing to land a
/// real `close-buffer!` inside the sweep's window.
#[test]
fn reload_config_explicit_language_restore_skips_a_bid_whose_buffer_was_swapped_in_place() {
    let init_scm = r#"(%define-language! "notes" '() '() '() #f)"#;
    let fixture = ReloadFixture::new(init_scm);

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("README"); // no extension: never auto-detected
    std::fs::write(&file, "hello\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    ed.init_scripting(&mut Default::default());
    let bid = ed.focused_buffer_id();
    type_cmd(&mut ed, ":set buffer language=notes");
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .language
            .map(|id| ed.state.config.languages.name_of(id)),
        Some("notes"),
        "sanity: the explicit :set must have applied"
    );

    let mut snapshot = ed.reset_config_state();
    ed.scripting = None;

    // Simulate `close_buffer`'s last-buffer in-place swap landing between
    // the snapshot and the sweep that would otherwise restore onto `bid` —
    // same effect on `replace_stamp` as a real `replace_buffer_in_place`
    // call, without needing to land a real reentrant `close-buffer!` inside
    // the sweep's window.
    ed.state.buffers.get_mut(bid).replace_stamp += 1;

    ed.init_scripting(&mut snapshot);

    assert_eq!(
        ed.state.buffers.get(bid).language,
        None,
        "a bid whose buffer was swapped in place after the snapshot must \
         not have the old buffer's explicit language restored onto it"
    );
    assert!(
        !ed.state.buffers.get(bid).language_explicit,
        "nor should it be left marked explicit"
    );

    drop(fixture);
}

// ---------------------------------------------------------------------------
// :reload-config preserves editing state — undo, jump list, minibuf
// history, registers, mode/selection, and pane focus
// ---------------------------------------------------------------------------

/// A real `:reload-config` dispatch must leave every piece of *editing*
/// state exactly as `typed_reload_config`'s doc comment promises — "buffers,
/// panes, undo history, registers, and running LSP server processes are
/// untouched — only config resets". Drives each piece of state through the
/// same key/command DSL a user would (not direct field pokes), then
/// verifies each one still *works* after the reload — not just that a
/// field happens to hold the same value, but that undo actually undoes,
/// jump-backward actually navigates, and the recalled history entry is the
/// one that was typed.
#[test]
fn reload_config_preserves_undo_jumplist_history_registers_mode_and_focus() {
    let fixture = ReloadFixture::new("");
    let mut ed = editor_from("-[o]>ne\ntwo\nthree\nfour\nfive\n");
    ed.init_scripting(&mut Default::default());

    let pre_reload_bid = ed.focused_buffer_id();
    let pre_reload_pid = ed.state.focused_pane_id;
    let text_before_edit = ed.doc().text().to_string();

    // Jump list: a large motion (goto-last-line) records the pre-jump
    // position (line 0) — `ge`'s own binding is verified in defaults.rs;
    // see `goto_last_line_records_jump` for the same pattern.
    ed.feed_key(key('g'));
    ed.feed_key(key('e'));

    // Undo history: one real edit.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    assert!(
        ed.doc().text().to_string().contains('X'),
        "sanity: the edit must have landed"
    );

    // Registers: written directly (not via a yank keybinding) so this test
    // doesn't also depend on knowing which register a plain `yy` targets.
    ed.state
        .registers
        .write_text('5', vec!["preserved-register-text".to_string()]);

    // Minibuf command history: a real command dispatch, not a buffer- or
    // focus-changing one (`:messages`/`:ls` would open a new view and
    // confuse the focused-buffer assertion below).
    type_cmd(&mut ed, ":set global scrolloff=5");

    let mode_before = ed.state.mode();
    let selections_before = ed.current_selections().clone();

    fixture.write_init("");
    type_cmd(&mut ed, ":reload-config");

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a real reload must not log any error; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );

    // Pane/buffer focus, mode, and selection: unchanged by a config reset.
    assert_eq!(
        ed.focused_buffer_id(),
        pre_reload_bid,
        "the focused buffer must not change"
    );
    assert_eq!(
        ed.state.focused_pane_id, pre_reload_pid,
        "the focused pane must not change"
    );
    assert_eq!(
        ed.state.mode(),
        mode_before,
        "the editing mode must not change"
    );
    assert_eq!(
        ed.current_selections(),
        &selections_before,
        "the cursor/selection must not move"
    );

    // Registers: still readable.
    assert_eq!(
        reg(&ed, '5'),
        vec!["preserved-register-text".to_string()],
        "a written register must survive the reload"
    );

    // Minibuf command history: the pre-reload command is still there —
    // `:reload-config` itself is pushed onto the same ring right before it
    // dispatches (`handle_command` records raw input before `execute_command`
    // runs), so it's expected as the newest entry alongside it.
    assert_eq!(
        ed.state
            .history
            .get(HistoryKind::Command)
            .entries()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "set global scrolloff=5".to_string(),
            "reload-config".to_string(),
        ],
        "command history must survive the reload"
    );

    // Undo history: the pre-reload edit must still be undoable.
    ed.feed_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        text_before_edit,
        "undo must still be able to reach the pre-edit, pre-reload text"
    );

    // Jump list: jump-backward must still reach the pre-jump position.
    ed.feed_key(key_ctrl('o'));
    assert_eq!(
        ed.current_selections().primary().head(),
        0,
        "jump-backward must still return to the position recorded before \
         the reload (start of the buffer, before the goto-last-line jump)"
    );
}
