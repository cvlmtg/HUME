// `Editor::reset_config_state` — the full-reset contract `:reload-config`
// relies on. Each test writes one config-owned surface via the real
// `EditorHostImpl` (`eval_with_real_host`, same eval path `init_scripting`
// uses), confirms the write landed, calls `reset_config_state` directly,
// then asserts the surface is back to its compiled-in default. Every
// assertion is written so it fails if `reset_config_state` did nothing —
// see each test's oracle.
//
// `Editor::resync_config_state` — the repopulation half, exercised in the
// `-- Resync --` section below: it repopulates state `reset_config_state`
// clears that is normally repopulated by a hook fired on a transition
// (server attach, buffer open, diagnostics published) which a bare reload
// never causes.
//
// The end-to-end wiring test (real `init.scm` on disk, `:reload-config`
// dispatched through the minibuffer) lives in `unix/reload_config.rs`.

use std::path::Path;

use super::*;
use hume_engine::types::Scope;
use hume_scripting::ScriptingHost;

use crate::editor::commands::open_pane;
use crate::editor::keymap::{BindMode, Keymap, WalkResult};
use crate::editor::lsp::LspState;
use crate::editor::reload::ReloadSnapshot;
use crate::ui::statusline::StatusElement;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;

// ── Keymap ───────────────────────────────────────────────────────────────────

/// A `bind-key!` override must revert to the compiled-in default binding —
/// not stay overridden, not end up unbound. Compared against a fresh
/// `Keymap::default()` (independent oracle) rather than a hardcoded
/// expectation for 'Q', which has no default binding at all.
#[test]
fn reset_reverts_bind_key_to_default() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(bind-key! 'normal "Q" "move-down")"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')]),
        Some(("move-down".to_string(), false)),
        "sanity: the override must be live before reset"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('Q')]),
        Keymap::default().lookup_command(BindMode::Normal, &[key('Q')]),
        "must revert to the compiled-in default for 'Q', not stay overridden"
    );
}

/// `unbind-key!` on a key with a compiled-in default (`x` → `select-line`)
/// must also revert — the default trie is rebuilt wholesale, not patched.
#[test]
fn reset_reverts_unbind_key_to_default() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(unbind-key! 'normal "x")"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('x')]),
        None,
        "sanity: 'x' must be unbound before reset"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('x')]),
        Some(("select-line".to_string(), false)),
        "must revert to the compiled-in default binding for 'x'"
    );
}

/// `bind-wait-char!` writes into the same trie `bind-key!` does, so the same
/// wholesale-rebuild reset must undo it too.
#[test]
fn reset_reverts_bind_wait_char_to_default() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");

    // Sanity: 'g' 'W' is not a WaitChar node in the compiled-in default —
    // otherwise the final assertion below would pass even if reset did nothing.
    assert!(
        !matches!(
            Keymap::default().normal.walk(&[key('g'), key('W')]),
            WalkResult::WaitChar(_)
        ),
        "test setup invalid: 'gW' must not default to a wait-char binding"
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(bind-wait-char! 'normal "g W" "some-cmd")"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert!(
        matches!(
            ed.state.config.keymap.normal.walk(&[key('g'), key('W')]),
            WalkResult::WaitChar(_)
        ),
        "sanity: the wait-char binding must be live before reset"
    );

    ed.reset_config_state();

    assert!(
        !matches!(
            ed.state.config.keymap.normal.walk(&[key('g'), key('W')]),
            WalkResult::WaitChar(_)
        ),
        "wait-char binding must not survive the reset"
    );
}

/// Kitty-only default binds (installed by `apply_kitty_defaults`, outside
/// `Keymap::default()`) must be reinstalled after a config override —
/// `reset_config_state` must not just rebuild the plain default trie and
/// stop there.
#[test]
fn reset_reinstalls_kitty_defaults_after_a_config_override() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    ed.set_kitty_support(true);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(bind-key! 'normal "ctrl-;" "move-down")"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key_ctrl(';')]),
        Some(("move-down".to_string(), false)),
        "sanity: the config override must win before reset"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key_ctrl(';')]),
        Some(("collapse-to-anchor-and-exit-extend".to_string(), false)),
        "kitty-only default must be reinstalled, not left overridden or unbound"
    );
}

// ── Settings ─────────────────────────────────────────────────────────────────

/// `set-option!` (as `init.scm` or `:set global` would call it) must revert
/// to `EditorSettings::default()` — not the previous session's value.
#[test]
fn reset_reverts_set_option_to_compiled_in_default() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(set-option! "scrolloff" 42)"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert_eq!(
        ed.state.settings.scrolloff, 42,
        "sanity: the write must land"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state.settings.scrolloff,
        EditorSettings::default().scrolloff,
        "runtime overrides must not survive a reset any more than init.scm ones do"
    );
}

/// A runtime `:set global` change — never written by any `init.scm` — is
/// discarded too: "from scratch" resets *every* global, not just the ones a
/// script wrote this session.
#[test]
fn reset_reverts_runtime_set_command_too() {
    let mut ed = editor_from("-[a]>b\n");
    type_cmd(&mut ed, ":set global scrolloff=7");

    assert_eq!(
        ed.state.settings.scrolloff, 7,
        "sanity: the write must land"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state.settings.scrolloff,
        EditorSettings::default().scrolloff
    );
}

/// `configure-statusline!` reverts to `StatusLineConfig::default()` — same
/// global-setting reset path as `scrolloff`, just a richer value.
#[test]
fn reset_reverts_statusline_config_to_default() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(configure-statusline! (list "Cwd") (list) (list))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert_eq!(
        ed.state.settings.statusline.left,
        vec![StatusElement::Cwd],
        "sanity: the write must land"
    );

    ed.reset_config_state();

    let default = crate::ui::statusline::StatusLineConfig::default();
    assert_eq!(ed.state.settings.statusline.left, default.left);
    assert_eq!(ed.state.settings.statusline.center, default.center);
    assert_eq!(ed.state.settings.statusline.right, default.right);
}

/// The loaded theme reverts to the compiled-in default (`sand.toml`) — not
/// via `resync_derived_state`'s ordinary "theme" arm (which deliberately
/// no-ops when `settings.theme` is empty, the reset value), but via the
/// explicit `view.theme = build_default_theme()` write `reset_globals` makes.
///
/// Swaps in a synthetic theme directly (no Steel eval, no dependency on a
/// real bundled theme file) so the test is self-contained; only the
/// synthetic swap's arbitrary color needs to differ from the default's, not
/// match any real theme's palette.
#[test]
fn reset_reverts_theme_to_compiled_in_default() {
    let mut ed = editor_from("-[a]>b\n");
    let default_style = ed.view.theme.resolve_by_name(Scope("ui.statusline"));

    let synthetic = hume_engine::theme::loader::parse_theme(
        "[palette]\ncustom = \"#123456\"\n\n\"ui.statusline\" = { fg = \"custom\" }\n",
    )
    .expect("synthetic theme must parse");
    ed.view.theme = synthetic;
    ed.state.settings.theme = "synthetic".to_string();

    assert_ne!(
        ed.view.theme.resolve_by_name(Scope("ui.statusline")),
        default_style,
        "sanity: the synthetic theme must actually differ from the default"
    );

    ed.reset_config_state();

    assert_eq!(ed.state.settings.theme, "");
    assert_eq!(
        ed.view.theme.resolve_by_name(Scope("ui.statusline")),
        default_style
    );
}

// ── Buffers ──────────────────────────────────────────────────────────────────

/// A `set-buffer-option!` override (e.g. from an `on-language-set` hook)
/// must not survive a reset — `BufferOverrides` goes back to "inherit from
/// global" for every open buffer.
#[test]
fn reset_clears_buffer_overrides() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "widen-tabs" "" (lambda () (set-buffer-option! (current-buffer) "tab-width" 8)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":widen-tabs");

    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state.buffers.get(bid).overrides.tab_width,
        Some(8),
        "sanity: the override must land"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state.buffers.get(bid).overrides.tab_width,
        None,
        "buffer-local override must be cleared, not survive the reset"
    );
}

/// `Buffer.language` is a `LanguageId` — an index into `state.config.languages`,
/// which the reset replaces with a fresh, empty registry right after this
/// clear. Left alone, the old index would dangle (and, worse, silently
/// alias whatever the new registry happens to intern at the same slot);
/// it must go back to `None` in the same reset that swaps the registry.
#[test]
fn reset_clears_stale_buffer_language_ids() {
    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));

    assert_eq!(
        ed.state.buffers.get(bid).language,
        Some(lang),
        "sanity: the language must be set before reset"
    );

    ed.reset_config_state();

    assert_eq!(
        ed.state.buffers.get(bid).language,
        None,
        "buffer language must not survive the reset — it would dangle once \
         state.config.languages is replaced, and its survival is what keeps \
         set_buffer_language's unchanged-value guard from firing OnLanguageSet again"
    );
}

// ── LSP ──────────────────────────────────────────────────────────────────────

/// `register-lsp-server!` clears from `lsp.configs` on reset — a language a
/// plugin registered but the new `init.scm` no longer does must not keep its
/// stale config lying around (`lsp.servers`, the running processes
/// themselves, are untouched — see `LspState::reset_config`'s doc).
#[test]
fn reset_clears_lsp_server_configs() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '())"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert_eq!(
        ed.lsp.config_command_for_test("rust"),
        Some("rust-analyzer".to_string()),
        "sanity: the registration must land"
    );

    ed.reset_config_state();

    assert_eq!(ed.lsp.config_command_for_test("rust"), None);
}

// ── Decorations ──────────────────────────────────────────────────────────────

/// Plugin-set decorations (signs, inlay hints, virtual lines, extra
/// highlights, inline diagnostics) don't linger past a reset — the plugin
/// that set them may not even be loaded by the new config.
#[test]
fn reset_clears_plugin_decorations() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "mark" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope" 7)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":mark");

    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state.config.decorations.signs_for("linter", bid).len(),
        1,
        "sanity: the sign must land"
    );

    ed.reset_config_state();

    assert!(
        ed.state
            .config
            .decorations
            .signs_for("linter", bid)
            .is_empty()
    );
}

// ── Timers ───────────────────────────────────────────────────────────────────

/// A scheduled `(after ms thunk)` must not fire against the *new* engine
/// after a reset — its `SteelVal` thunk is rooted in the outgoing one.
#[test]
fn reset_cancels_pending_steel_timers() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "start" "" (lambda () (after 100000 (lambda () (car '())))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":start");

    assert_eq!(
        ed.timer_payloads.len(),
        1,
        "sanity: the timer must be scheduled"
    );

    ed.reset_config_state();

    assert!(
        ed.timer_payloads.is_empty(),
        "a Steel `after` thunk must not survive the reset"
    );
}

// ── Overlays ─────────────────────────────────────────────────────────────────

/// Regression test: `reset_config_state` clears `state.config.drawer`
/// directly (not through `close-drawer!`, which would queue a callback the
/// reset already drops), with no paired view sync at that call site — unlike
/// popup/menu/picker, whose views re-resolve from the model every frame, the
/// drawer's view previously synced only on-mutation, so nothing ever told it
/// the model had changed. Fixed by making `prepare_frame` sync the drawer
/// view unconditionally every frame, like the other three overlays; this
/// drives one frame after the reset and confirms the view catches up.
#[test]
fn reset_reload_drawer_view_self_heals_on_the_next_frame() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_drawer_list(
        vec!["one".to_string(), "two".to_string()],
        steel::rvals::SteelVal::Void,
    )
    .unwrap();
    assert!(
        ed.state.drawer_view.read().unwrap().is_some(),
        "sanity: the view must be populated on open"
    );

    ed.reset_config_state();
    assert!(
        ed.state.config.drawer.is_none(),
        "sanity: the model must be cleared by the reset"
    );

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(40, 10, &mut ctx);

    assert!(
        ed.state.drawer_view.read().unwrap().is_none(),
        "the drawer view must self-heal on the very next frame after a \
         reset clears the model, not stay stale (and uncloseable — key \
         routing gates on state.config.drawer.is_some()) forever"
    );
}

/// Regression test: `reset_config_state` used to `take()`
/// `steel_prompt_callback` without the rest of the prompt session's
/// teardown (mode + minibuf + history session) — leaving the editor parked
/// in `Mode::Command` with an open minibuf and no callback wired up, so an
/// abandoned prompt's half-typed answer would be misread as an ordinary `:`
/// command on the very next Enter.
#[test]
fn reset_tears_down_an_open_prompt_session_completely() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":go");

    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Command,
        "sanity: prompt! must open Command mode"
    );
    assert!(
        ed.state.minibuf.is_some(),
        "sanity: the minibuf must be open"
    );
    assert!(
        ed.state.config.steel_prompt_callback.is_some(),
        "sanity: the callback must be armed"
    );

    ed.reset_config_state();

    assert!(
        ed.state.config.steel_prompt_callback.is_none(),
        "the callback (rooted in the outgoing engine) must be dropped"
    );
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Normal,
        "an open prompt session must exit back to Normal mode, not leave \
         Command mode behind with no callback wired up"
    );
    assert!(
        ed.state.minibuf.is_none(),
        "an open prompt session's minibuf must close, or its half-typed \
         answer would be misread as an ordinary : command on the next Enter"
    );
}

/// An open picker session must be gone after a reset, and — unlike `Esc`/
/// `picker-close!` — its `on_select` callback must never fire: it belongs to
/// the outgoing engine, which is seconds from being dropped (see
/// `picker::close_picker`'s doc for why `reset_config_state` deliberately
/// bypasses that chokepoint). Checked via `pending_steel_calls` staying
/// empty, not just `picker.is_none()` — the latter alone can't tell "dropped
/// silently" apart from "closed normally", since `close_picker` also clears
/// the field.
#[test]
fn reset_tears_down_an_open_picker_session_without_firing_its_callback() {
    let mut ed = editor_from("-[a]>b\n");
    let mut session = crate::editor::picker::PickerSession::new(
        steel::rvals::SteelVal::StringV("cb".into()),
        String::new(),
        false,
    );
    let token = session.token();
    session.push(
        token,
        vec![crate::editor::picker::PickerItem {
            display: "one".to_string(),
            payload: steel::rvals::SteelVal::StringV("one".into()),
        }],
    );
    crate::editor::picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);
    assert!(
        ed.state.config.picker.is_some(),
        "sanity: the picker must be open"
    );

    ed.reset_config_state();

    assert!(
        ed.state.config.picker.is_none(),
        "the picker session must not survive a reset"
    );
    assert!(
        ed.state.config.pending_steel_calls.is_empty(),
        "the picker's on_select callback (rooted in the outgoing engine) must \
         be discarded, not queued for firing"
    );
}

// ── Dynamic commands ─────────────────────────────────────────────────────────

/// A `define-command!`-registered command is gone after a reset —
/// `ConfigState::new`'s `CommandRegistry::with_defaults()` rebuild is total,
/// not incremental. Leaving it in `registry.names()` would make the
/// reloaded `init.scm`'s re-`(define-command! …)` trip the
/// builtin-conflict check.
#[test]
fn reset_clears_dynamic_commands() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert!(
        ed.state.config.registry.contains("bar"),
        "sanity: the command must register"
    );

    ed.reset_config_state();

    assert!(
        !ed.state.config.registry.contains("bar"),
        "dynamic command must be gone after reset"
    );
}

// ── Resync (Editor::resync_config_state) ────────────────────────────────────
//
// `reset_config_state` clears state a hook normally repopulates on a
// transition (server attach, buffer open, diagnostics published) that a bare
// reload never causes. These tests exercise `resync_config_state` directly —
// the replay that fires those hooks — rather than the full reset+rebuild
// dance, since the hook hand-off is the part under test.

/// Wires a scripted server attached to the focused buffer under `language`,
/// handshake not yet driven (client is `Starting`).
fn wire_starting_server(ed: &mut Editor, language: &str) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    let sid = backend
        .start("test-server", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test(language.to_string(), std::path::PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    sid
}

/// Drives the queued `initialize` response through to `BecameRunning`.
fn complete_handshake(ed: &mut Editor, sid: ServerId) {
    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    assert_eq!(sid2, sid);
}

/// A `Running` server's attachment must re-fire `OnLspAttach` on resync —
/// this is what makes `register-trigger-chars!` (called from `core:lsp`'s
/// `on-lsp-attach` handler) take effect again after a reload, without any
/// LSP wire traffic: the server was never detached.
#[test]
fn resync_refires_lsp_attach_for_a_running_server() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let sid = wire_starting_server(&mut ed, "rust");
    complete_handshake(&mut ed, sid);
    // `complete_handshake`'s `BecameRunning` arm already queued an
    // `OnLspAttach` for this attachment, with no scripting host yet to
    // handle it — drop it, mirroring what `reset_config_state`'s
    // `pending_hooks.clear()` does to any hook queued before a reload, so
    // only `resync_config_state`'s own fire is under test below.
    ed.state.config.pending_hooks.clear();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-attach (lambda (bid server-name)
             (when (equal? server-name "rust") (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    let before = state(&ed);

    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        before,
        "on-lsp-attach must re-fire for a still-Running server on resync"
    );
}

/// A `Starting` server's attachment must NOT re-fire here — it fires its own
/// `OnLspAttach` once `BecameRunning` runs (`dispatch_lsp_action`), and
/// firing it again from resync would double it.
#[test]
fn resync_does_not_refire_attach_for_a_starting_server() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    wire_starting_server(&mut ed, "rust");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-attach (lambda (bid server-name)
             (when (equal? server-name "rust") (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    let before = state(&ed);

    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        state(&ed),
        before,
        "a Starting server's attach must not be re-fired by resync"
    );
}

/// Every already-open buffer gets `OnBufferOpen` re-fired on resync — the
/// same replay that covers a plugin's decorations set from that hook
/// (signs, virtual lines) which `reset_config_state`'s `decorations.clear_all()`
/// wipes and nothing else would bring back, since buffers aren't reopened.
#[test]
fn resync_refires_buffer_open_for_every_open_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let first_bid = ed.focused_buffer_id();

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("second.txt");
    std::fs::write(&file, "hi\n").unwrap();
    let (second_bid, is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
        &mut ed.view,
        &mut ed.state,
        &file.canonicalize().unwrap(),
    )
    .unwrap();
    assert!(is_new, "sanity: this must be a genuinely new buffer");
    ed.detect_pending_languages();
    ed.drain_hooks();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-open (lambda (bid)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Counts fires via a per-buffer `tab-width` override, incremented once
    // per `on-buffer-open` call for that buffer — a plain "did state change"
    // check can't distinguish "every open buffer fired once" from "only one
    // of them fired" (the bug a `.take(1)` mutation to the resync loop would
    // leave undetected with a single-buffer fixture).
    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        ed.state.buffers.get(first_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-buffer-open must re-fire exactly once for the first pre-reload buffer"
    );
    assert_eq!(
        ed.state.buffers.get(second_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-buffer-open must re-fire exactly once for the second pre-reload buffer too"
    );
}

/// A buffer opened during this reload's `init_scripting()` call — e.g. by a
/// lazy language plugin's activation body, the one context besides a plain
/// command where `open-buffer!` is actually callable (`init.scm`'s own
/// top-level is `EvalMode::Init`, which the builtin rejects; opening the
/// buffer directly here mirrors what that activation call does once it
/// reaches `EditorHostImpl::open_buffer`, without fighting the eval-mode
/// gate to get there) — must NOT get `OnBufferOpen` re-fired by resync. It
/// already got one fire from the ordinary open path
/// (`detect_pending_languages`, run by `apply_script_effects`'s tail — here,
/// called directly to mirror that same tail call — before
/// `resync_config_state` is ever invoked). Only a buffer that predates the
/// reload (excluded here by never appearing in `snapshot`, taken before the
/// new buffer opens) should be re-fired.
///
/// Counts fires via a per-buffer `tab-width` override, incremented once per
/// `on-buffer-open` call for that buffer — a plain "did state change" check
/// (as `resync_refires_buffer_open_for_every_open_buffer` above uses) can't
/// distinguish "fired once" from "fired twice", which is exactly the
/// distinction this test needs.
#[test]
fn resync_does_not_refire_buffer_open_for_a_buffer_opened_by_this_reload() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let old_bid = ed.focused_buffer_id();

    // Snapshotted before the new buffer opens — mirrors `reset_config_state`
    // capturing its `ReloadSnapshot` before `init_scripting` runs.
    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-open (lambda (bid)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("new.txt");
    std::fs::write(&file, "hi\n").unwrap();
    let (new_bid, is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
        &mut ed.view,
        &mut ed.state,
        &file.canonicalize().unwrap(),
    )
    .unwrap();
    assert!(is_new, "sanity: this must be a genuinely new buffer");
    // Mirrors `apply_script_effects`'s own tail call — the ordinary open
    // path's `OnBufferOpen` fire, enqueued (not yet executed: `fire_hook_silent`
    // only pushes onto `pending_hooks`) here rather than via a real eval.
    ed.detect_pending_languages();
    ed.drain_hooks();

    assert_eq!(
        ed.state.buffers.get(new_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "sanity: the ordinary open path must have fired on-buffer-open once for the new buffer"
    );
    assert_eq!(
        ed.state.buffers.get(old_bid).overrides.tab_width,
        None,
        "sanity: the pre-existing buffer must not have fired yet"
    );

    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        ed.state.buffers.get(old_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-buffer-open must fire exactly once for a buffer that predates the reload"
    );
    assert_eq!(
        ed.state.buffers.get(new_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-buffer-open must NOT fire again for a buffer this reload's init.scm itself opened"
    );
}

/// `on-diagnostics-changed` re-fires from the surviving `LspState::diagnostics`
/// cache (`LspState::reset_config` deliberately never touches it), not from
/// a fresh wire publish — exactly the reported symptom: decorations empty
/// after a reload while the underlying diagnostics data is still there.
#[test]
fn resync_refires_diagnostics_changed_from_the_surviving_cache() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("diag.rs");
    std::fs::write(&file, "aa\nbb\n").unwrap();
    let canonical = file.canonicalize().unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(canonical.clone()));
    let sid = wire_starting_server(&mut ed, "rust");
    complete_handshake(&mut ed, sid);
    ed.state.config.pending_hooks.clear(); // see resync_refires_lsp_attach_for_a_running_server's comment

    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();
    let parsed: lsp_types::PublishDiagnosticsParams = serde_json::from_value(serde_json::json!({
        "uri": uri,
        "diagnostics": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
            "severity": 1,
            "message": "boom",
        }],
    }))
    .expect("well-formed PublishDiagnosticsParams");
    assert_eq!(
        ed.ingest_publish_diagnostics(sid, parsed),
        Some(bid),
        "sanity: diagnostics must ingest into the buffer"
    );

    // The state a real `reset_config_state` would leave behind: rendered
    // decorations wiped, `LspState::diagnostics` untouched.
    ed.state.config.decorations = crate::editor::decorations::DecorationStores::reset(
        ed.state.config.decorations.virtual_lines_generation(),
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-diagnostics-changed (lambda (bid)
             (set-signs! "diag" bid (list (list 0 "!" "diag" (length (diagnostics-for-buffer bid)))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert!(
        ed.state
            .config
            .decorations
            .signs_for("diag", bid)
            .is_empty(),
        "sanity: decorations must actually be empty before resync"
    );

    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        ed.state.config.decorations.signs_for("diag", bid).len(),
        1,
        "on-diagnostics-changed must re-fire from the surviving diagnostics cache"
    );
}

/// `on-diagnostics-changed` must still re-fire from the surviving cache when
/// the server that published it has since crashed — `running_attached_buffers`
/// excludes `Crashed` servers by design (it drives `OnLspAttach`, which must
/// not fire for a dead server), but `LspState::reset_config` never clears
/// `diagnostics` for a crash either, so the buffer's last-known diagnostics
/// are still there to replay. Filtering the resync's diagnostics loop through
/// `running_attached_buffers` (rather than the diagnostics cache itself)
/// would silently skip exactly this buffer.
#[test]
fn resync_refires_diagnostics_changed_for_a_crashed_servers_surviving_cache() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("diag.rs");
    std::fs::write(&file, "aa\nbb\n").unwrap();
    let canonical = file.canonicalize().unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(canonical.clone()));
    let sid = wire_starting_server(&mut ed, "rust");
    complete_handshake(&mut ed, sid);
    ed.state.config.pending_hooks.clear(); // see resync_refires_lsp_attach_for_a_running_server's comment

    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();
    let parsed: lsp_types::PublishDiagnosticsParams = serde_json::from_value(serde_json::json!({
        "uri": uri,
        "diagnostics": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
            "severity": 1,
            "message": "boom",
        }],
    }))
    .expect("well-formed PublishDiagnosticsParams");
    assert_eq!(
        ed.ingest_publish_diagnostics(sid, parsed),
        Some(bid),
        "sanity: diagnostics must ingest into the buffer"
    );

    // Crash the server via the same path a real transport failure takes
    // (`LspClient::on_event` transitions its internal state to `Crashed` and
    // returns the action) — `dispatch_lsp_action`'s `Crashed` arm clears its
    // progress/pending requests but deliberately never touches `diagnostics`
    // or `buf.lsp_server`.
    let actions =
        ed.lsp
            .client_for_test(sid)
            .unwrap()
            .on_event(hume_lsp::transport::InboundEvent::Eof {
                error: Some("boom".to_string()),
            });
    for action in actions {
        ed.dispatch_lsp_action(sid, action);
    }
    assert!(
        ed.lsp
            .running_attached_buffers(&ed.state.buffers)
            .is_empty(),
        "sanity: a crashed server must not appear in running_attached_buffers"
    );

    // The state a real `reset_config_state` would leave behind: rendered
    // decorations wiped, `LspState::diagnostics` untouched.
    ed.state.config.decorations = crate::editor::decorations::DecorationStores::reset(
        ed.state.config.decorations.virtual_lines_generation(),
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-diagnostics-changed (lambda (bid)
             (set-signs! "diag" bid (list (list 0 "!" "diag" (length (diagnostics-for-buffer bid)))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    assert!(
        ed.state
            .config
            .decorations
            .signs_for("diag", bid)
            .is_empty(),
        "sanity: decorations must actually be empty before resync"
    );

    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        ed.state.config.decorations.signs_for("diag", bid).len(),
        1,
        "on-diagnostics-changed must re-fire from the surviving cache even though \
         the server that published it has crashed"
    );
}

/// Every pane showing a surviving buffer gets `OnViewportChange` re-fired on
/// resync — the replay that brings back inlay hints and anything else
/// `on-viewport-change`-gated without the user having to scroll. Two panes on
/// two different buffers, both counted via a per-buffer `tab-width` bump
/// (same technique as `resync_refires_buffer_open_for_every_open_buffer` —
/// a plain "did state change" check can't distinguish "each pane fired once"
/// from "only one pane fired, or one fired twice for the wrong buffer").
/// The exact `(first, last)` bounds this hook reports are covered separately
/// by `viewport_range_matches_the_on_viewport_change_hooks_own_computation`
/// (`lsp_introspect.rs`) — this test is about the resync replay's fan-out
/// across panes, not `pane_visible_range`'s own math.
#[test]
fn resync_refires_viewport_change_once_per_pane_on_a_surviving_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let first_bid = ed.focused_buffer_id();

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("second.txt");
    std::fs::write(&file, "hi\n").unwrap();
    let (second_bid, is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
        &mut ed.view,
        &mut ed.state,
        &file.canonicalize().unwrap(),
    )
    .unwrap();
    assert!(is_new, "sanity: this must be a genuinely new buffer");
    ed.detect_pending_languages();
    ed.drain_hooks();
    open_pane(&mut ed.state, &mut ed.view, second_bid);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first last)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let snapshot =
        ReloadSnapshot::for_test(ed.state.buffers.iter().map(|(id, _)| id), &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        ed.state.buffers.get(first_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-viewport-change must re-fire exactly once for the first pane's buffer"
    );
    assert_eq!(
        ed.state.buffers.get(second_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "on-viewport-change must re-fire exactly once for the second pane's buffer too"
    );
}

/// A pane whose buffer is absent from the snapshot (mirrors a buffer opened
/// during this same reload, per `resync_does_not_refire_buffer_open_for_a_
/// buffer_opened_by_this_reload`) must not get `OnViewportChange` re-fired —
/// there's no pre-reload state to restore for it, and its own open path
/// already covers whatever it needs.
#[test]
fn resync_does_not_refire_viewport_change_for_a_pane_on_a_buffer_absent_from_the_snapshot() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let first_bid = ed.focused_buffer_id();

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("second.txt");
    std::fs::write(&file, "hi\n").unwrap();
    let (second_bid, _) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
        &mut ed.view,
        &mut ed.state,
        &file.canonicalize().unwrap(),
    )
    .unwrap();
    ed.detect_pending_languages();
    ed.drain_hooks();
    open_pane(&mut ed.state, &mut ed.view, second_bid);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first last)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Snapshot covers only `first_bid` — `second_bid` is treated as opened
    // during this same reload.
    let snapshot = ReloadSnapshot::for_test([first_bid], &ed.state.buffers);
    ed.resync_config_state(&snapshot);
    ed.drain_hooks();

    assert_eq!(
        ed.state.buffers.get(first_bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "sanity: the surviving pane's buffer must still fire"
    );
    assert_eq!(
        ed.state.buffers.get(second_bid).overrides.tab_width,
        None,
        "a pane on a buffer absent from the snapshot must not get \
         on-viewport-change re-fired"
    );
}
