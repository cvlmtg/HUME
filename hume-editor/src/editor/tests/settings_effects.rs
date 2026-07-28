use super::*;

use crate::editor::buffer::Buffer;
use crate::editor::message_log::Severity;
use crate::editor::minibuf::history::HistoryKind;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pipeline::RenderContext;

/// Drive `(set-option! ...)` through the real Steel path
/// (`EditorHostImpl::set_global_option`) — mirrors the harness in
/// `editor/tests/undo_levels.rs`'s `steel_set_option_applies_undo_levels`.
fn eval_set_option(ed: &mut Editor, source: &str) -> Result<(), String> {
    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = hume_scripting::ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(source.to_owned(), Default::default(), &mut init_host)
}

// ── set-option! resyncs derived state (the gap this closes) ────────────────

#[test]
fn set_option_applies_history_capacity() {
    // Fail oracle: revert set_global_option to call crate::settings::write_global
    // directly (bypassing settings_ops::apply_global's resync step).
    // settings.history_capacity still updates — write_global's own job — so
    // that assertion stays green; state.history's actual capacity never
    // resyncs, so it's the post-push assertion at the bottom (verified
    // empirically: fails "left: 4, right: 2") that goes red.
    let mut ed = editor_from("-[h]>ello\n");
    for cmd in ["a", "b", "c"] {
        ed.state
            .history
            .get_mut(HistoryKind::Command)
            .push(cmd.into());
    }
    assert_eq!(
        ed.state.history.get(HistoryKind::Command).entries().len(),
        3
    );

    let result = eval_set_option(&mut ed, r#"(set-option! "history-capacity" 2)"#);
    assert!(result.is_ok(), "eval must succeed: {result:?}");

    assert_eq!(ed.state.settings.history_capacity, 2);
    assert_eq!(
        ed.state.history.get(HistoryKind::Command).entries().len(),
        3,
        "lowering the cap must not retroactively trim existing entries"
    );

    // The next push converges to the new cap in one shot.
    ed.state
        .history
        .get_mut(HistoryKind::Command)
        .push("d".into());
    assert_eq!(
        ed.state.history.get(HistoryKind::Command).entries().len(),
        2,
        "the next push must apply the resynced capacity with no manual pickup"
    );
}

#[test]
fn set_option_applies_jump_list_capacity() {
    // Fail oracle: revert set_global_option to call crate::settings::write_global
    // directly (bypassing settings_ops::apply_global's resync step).
    // settings.jump_list_capacity still updates — write_global's own job —
    // so that assertion stays green; the live jump list's actual capacity
    // never resyncs, so it's the post-push assertion at the bottom (verified
    // empirically: fails "left: 6, right: 2") that goes red.
    let mut ed = editor_from("-[h]>ello\n");
    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    for i in 0..5 {
        ed.state.panes.jumps[pid].push(crate::editor::jump_list::JumpEntry {
            buffer_id: bid,
            selections: hume_editing::selection::SelectionSet::single(
                hume_editing::selection::Selection::collapsed(0),
            ),
            primary_line: i,
        });
    }
    assert_eq!(ed.state.panes.jumps[pid].len(), 5);

    let result = eval_set_option(&mut ed, r#"(set-option! "jump-list-capacity" 2)"#);
    assert!(result.is_ok(), "eval must succeed: {result:?}");

    assert_eq!(ed.state.settings.jump_list_capacity, 2);
    assert_eq!(
        ed.state.panes.jumps[pid].len(),
        5,
        "lowering the cap must not retroactively trim existing entries"
    );

    // The overshoot (5 -> new cap 2) is more than one entry — the next push
    // must still converge to the cap in this one call.
    ed.state.panes.jumps[pid].push(crate::editor::jump_list::JumpEntry {
        buffer_id: bid,
        selections: hume_editing::selection::SelectionSet::single(
            hume_editing::selection::Selection::collapsed(0),
        ),
        primary_line: 5,
    });
    assert_eq!(
        ed.state.panes.jumps[pid].len(),
        2,
        "the next push must apply the resynced capacity with no manual pickup"
    );
}

// ── :set global mouse-enabled / mouse-select resync per frame ─────────────

/// `mouse-enabled`/`mouse-select` are terminal modes applied once at startup
/// (`hume_platform::terminal::init`, called from `hume-editor/src/lib.rs`
/// before entering the event loop) — there is no other write side. Before
/// `resync_mouse_mode` existed, `:set global mouse-enabled=false` changed
/// `EditorSettings` but the terminal kept reporting mouse events until
/// restart. `prepare_frame` now calls `resync_mouse_mode` every frame, which
/// re-applies the terminal mode whenever it drifts from `state.settings`.
///
/// No `SharedTerm` exists in test `Editor`s (`Editor::for_testing`/`open`
/// both seed `terminal: None`), so this can't assert on emitted escape
/// bytes — that path is covered by `hume-platform`'s
/// `mouse_enable_without_select_emits_1000_and_1006_only` and friends
/// (`hume-platform/src/terminal/tests.rs`). This asserts the terminal-mode
/// tracking state itself (`Editor::applied_mouse_mode`) resyncs to the new
/// setting on the next `prepare_frame`, which is the part `resync_mouse_mode`
/// can do headless — the `if let Some(term)` write is a one-line guard
/// around the same comparison, exercised whenever a real terminal is
/// attached.
///
/// Fail oracle: remove the `self.resync_mouse_mode()` call from
/// `prepare_frame` — `applied_mouse_mode` stays `(true, false)` (the
/// constructor default) even after this `:set`, and the assertion fails.
#[test]
fn set_global_mouse_enabled_resyncs_applied_mode_next_frame() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_eq!(
        ed.applied_mouse_mode,
        (true, false),
        "default EditorSettings has mouse_enabled=true, mouse_select=false"
    );

    crate::editor::commands::typed_set(&mut ed, Some("global mouse-enabled=false"), false)
        .expect("set mouse-enabled");
    assert_eq!(
        ed.applied_mouse_mode,
        (true, false),
        "the setting write itself must not resync — only prepare_frame does"
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 8, &mut ctx);

    assert_eq!(
        ed.applied_mouse_mode,
        (false, false),
        "prepare_frame must resync the terminal mouse mode from the new setting"
    );
}

// ── statusline.mode-colors gates whole-row tinting ─────────────────────────

#[test]
fn set_option_statusline_mode_colors_gates_whole_row_tint() {
    // Fail oracle: drop the `statusline_mode_colors` check from
    // `HumeStatusline::render` (always resolve the real mode) and the
    // "colors off" assertion below fails — the row would still tint cyan in
    // Insert mode.
    //
    // The fixture theme gives `ui.statusline` and `ui.statusline.normal`
    // *different* backgrounds — every bundled theme makes them equal, which
    // would let the off-state assertion pass whether the opt-out reads the
    // base scope (correct) or silently substitutes `EditorMode::Normal`
    // (the bug: an imported theme with a distinct Normal-mode accent, e.g.
    // Helix's old pill idiom, would still tint the row).
    use hume_engine::types::{ResolvedStyle, Scope};

    let mut ed = editor_from("-[h]>ello\n");
    let rect = ratatui::layout::Rect::new(0, 0, 40, 8);
    let row = rect.bottom() - 1;

    let mut styles = std::collections::HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            bg: Some(ratatui::style::Color::DarkGray),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.normal",
        ResolvedStyle {
            bg: Some(ratatui::style::Color::Red),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.insert",
        ResolvedStyle {
            bg: Some(ratatui::style::Color::Cyan),
            ..Default::default()
        },
    );
    ed.view.theme = hume_engine::theme::Theme::new(styles, ResolvedStyle::default());

    let base_bg = ed.view.theme.resolve_by_name(Scope("ui.statusline")).bg;
    let normal_bg = ed
        .view
        .theme
        .resolve_by_name(Scope("ui.statusline.normal"))
        .bg;
    let insert_bg = ed
        .view
        .theme
        .resolve_by_name(Scope("ui.statusline.insert"))
        .bg;
    assert_ne!(
        base_bg, normal_bg,
        "sanity: fixture theme must give the base row and Normal distinct colors"
    );
    assert_ne!(
        normal_bg, insert_bg,
        "fixture theme must give Normal and Insert distinct row colors"
    );

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert);

    // Default: statusline.mode-colors is on — the row tints for Insert.
    let buf = ed.render_to_buf(rect);
    assert_eq!(buf[(0, row)].style().bg, insert_bg);

    eval_set_option(&mut ed, r#"(set-option! "statusline.mode-colors" #f)"#)
        .expect("eval must succeed");

    // Off: the row reads the theme's base style, not the Normal-mode scope,
    // even while still in Insert mode.
    let buf = ed.render_to_buf(rect);
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "toggling the option must not change the mode"
    );
    assert_eq!(buf[(0, row)].style().bg, base_bg);
}

#[test]
fn set_option_applies_undo_levels() {
    // Fail oracle: same as above, for the undo-levels arm — without the
    // inline resync, the second edit below would stay undoable.
    let mut ed = editor_from("-[h]>ello\n");
    let result = eval_set_option(&mut ed, r#"(set-option! "undo-levels" 1)"#);
    assert!(result.is_ok(), "eval must succeed: {result:?}");
    assert_eq!(ed.state.settings.undo_levels, 1);

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.feed_key(key('i'));
    ed.feed_key(key('y'));
    ed.feed_key(key_esc());
    assert!(ed.doc().can_undo());

    ed.feed_key(key('u'));
    assert!(
        !ed.doc().can_undo(),
        "cap must already apply to the open buffer"
    );
}

#[test]
fn set_option_theme_failure_does_not_persist() {
    // The real bug: set_global_option used to write the raw value with no
    // resync at all, so a bad theme name from `set-option!` (e.g. from a
    // lazily-activated plugin) would sit in settings.theme forever, later
    // reported as "current theme" even though it never loaded.
    // Fail oracle: drop the rollback in settings_ops::apply and
    // settings.theme ends up "no_such_theme_xyz" instead of empty.
    let mut ed = editor_from("-[h]>ello\n");
    let result = eval_set_option(&mut ed, r#"(set-option! "theme" "no_such_theme_xyz")"#);
    // apply_global surfaces a failed theme load as an Err (rather than
    // Ok(()) with only a message-log entry) so a plugin's own
    // (with-handler ...) around set-option! actually fires. Fail oracle
    // for *this* assertion: revert apply_global's Err return and this
    // becomes Ok, silently hiding the failure from Steel callers again.
    assert!(
        result.is_err(),
        "a failed theme load must surface as a Steel error: {result:?}"
    );

    assert!(
        ed.state.settings.theme.is_empty(),
        "a theme that failed to load must not persist, got {:?}",
        ed.state.settings.theme
    );
    assert!(
        ed.state.message_log.has_unseen(),
        "expected a warning message"
    );
}

// ── :set global theme=<bad> — the same bug via the typed path ──────────────

#[test]
fn typed_set_theme_failure_does_not_persist() {
    // Same bug, same rollback, via `:set global` instead of Steel. Before
    // the rollback fix, `:set global theme=bad` persisted "bad" into settings
    // (store-then-load), unlike `:theme bad` (load-then-store) — the two
    // entry points disagreed. Fail oracle: same as above.
    let mut ed = editor_from("-[h]>ello\n");
    let result =
        crate::editor::commands::typed_set(&mut ed, Some("global theme=no_such_theme_xyz"), false);
    assert!(
        result.is_err(),
        "a failed theme load must surface as a command error: {result:?}"
    );

    assert!(
        ed.state.settings.theme.is_empty(),
        "a theme that failed to load must not persist, got {:?}",
        ed.state.settings.theme
    );
}

// ── :theme delegates to the chokepoint ──────────────────────────────────────

#[test]
fn typed_theme_bad_name_leaves_setting() {
    // Regression guard: :theme's own load-then-store behavior must survive
    // delegating to settings_ops::apply. Mirrors
    // editor::tests::commands::load_theme_by_name_fails_gracefully, but
    // through the typed_theme entry point instead of calling the loader
    // directly.
    // Fail oracle: drop the rollback in settings_ops::apply (the same
    // mutation settings_effects.rs's theme tests above are verified
    // against) — settings.theme ends up "no_such_theme_xyz" here too, since
    // :theme now shares that code path.
    let mut ed = editor_from("-[h]>ello\n");
    let result = crate::editor::commands::typed_theme(&mut ed, Some("no_such_theme_xyz"), false);
    assert!(
        result.is_err(),
        "a failed theme load must surface as a command error: {result:?}"
    );

    assert!(ed.state.settings.theme.is_empty());
    assert!(
        ed.state.message_log.has_unseen(),
        "expected a warning message"
    );
}

/// Points `HUME_RUNTIME` at the real bundled runtime dir for the duration of
/// a test, so `theme::load_theme_by_name` can find real theme files.
/// Mirrors `editor/tests/unix/mod.rs`'s `RealRuntimeGuard`, minus the
/// `XDG_DATA_HOME` redirect (unneeded for a read-only theme load).
struct RealThemeRuntimeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl RealThemeRuntimeGuard {
    fn new() -> Self {
        let lock = super::HUME_RUNTIME_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let real_runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime");
        // SAFETY: not unsafe in the memory-safety sense — Rust 2024 requires
        // the block because env vars are process-global; HUME_RUNTIME_MUTEX
        // is what actually makes this test-safe (see its doc at tests/mod.rs).
        unsafe {
            std::env::set_var("HUME_RUNTIME", real_runtime);
        }
        Self { _lock: lock }
    }
}

impl Drop for RealThemeRuntimeGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("HUME_RUNTIME");
        }
    }
}

#[test]
fn typed_theme_sets_setting_on_success() {
    // Fail oracle: revert typed_theme to its own load-then-store path (still
    // correct on its own) with a typo in the delegated key string (e.g.
    // "themes" instead of "theme") — write_global would then return
    // Err("unknown setting"), and this test's Ok() assertion would fail.
    let _guard = RealThemeRuntimeGuard::new();
    let mut ed = editor_from("-[h]>ello\n");
    let result = crate::editor::commands::typed_theme(&mut ed, Some("gruvbox"), false);
    assert!(result.is_ok(), "command must not error: {result:?}");
    assert_eq!(ed.state.settings.theme, "gruvbox");
}

// ── set-buffer-option! from an on-language-set hook ─────────────────────────

/// `set-buffer-option!`, called from an `on-language-set` hook with the
/// hook's own `bid`, writes the target buffer's override and leaves the
/// global setting untouched — proving the write lands in `BufferOverrides`,
/// not `EditorSettings`.
///
/// Fail oracle: route the builtin through `set_global_option` instead of
/// `set_buffer_option` and the second assertion fails (global becomes 8).
#[test]
fn set_buffer_option_from_hook_writes_target_override() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "tab-width" 8)))"#,
    );
    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks();

    assert_eq!(ed.state.buffers.get(bid).overrides.tab_width, Some(8));
    assert_eq!(
        ed.state.settings.tab_width, 4,
        "global tab-width must be untouched by a buffer-scoped write"
    );
}

/// The hook's `bid` argument, not the focused buffer, is the write target —
/// pins the distinction that `drain_hooks` runs with the *focused* buffer as
/// scripting context while the hook's own `bid` may name a background
/// buffer.
///
/// Fail oracle: implement the builtin against `(current-buffer)` instead of
/// the explicit `bid` argument — both assertions below would fail (the
/// focused buffer would get the override, the background buffer would not).
#[test]
fn set_buffer_option_targets_hook_bid_not_focused_buffer() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "tab-width" 8)))"#,
    );
    let focused_bid = ed.focused_buffer_id();
    let bid2 = ed.open_buffer(Buffer::new(Text::from("x\n"), SelectionSet::default()));
    assert_ne!(bid2, focused_bid, "second buffer must not be focused");

    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid2, Some(lang));
    ed.drain_hooks();

    assert_eq!(ed.state.buffers.get(bid2).overrides.tab_width, Some(8));
    assert_eq!(
        ed.state.buffers.get(focused_bid).overrides.tab_width,
        None,
        "the focused (non-target) buffer must be untouched"
    );
}

/// `get-option`'s optional leading-bid argument, mirrored here at the host
/// layer (`SettingsHost::get_option` takes an explicit `bid`, same as
/// `set_buffer_option`), reads the *named* buffer's override — not the
/// focused buffer's — the read-side half of the same hook-bid distinction
/// `set_buffer_option_targets_hook_bid_not_focused_buffer` pins for writes.
///
/// Fail oracle: pass `ctx.focused_buffer_id` unconditionally instead of the
/// decoded bid argument — this would read the focused buffer's default (4)
/// instead of bid2's override (8).
#[test]
fn get_option_explicit_bid_reads_hook_target_not_focused_buffer() {
    use hume_scripting::host::{EditorHost, OptionValue};

    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "tab-width" 8)))"#,
    );
    let focused_bid = ed.focused_buffer_id();
    let bid2 = ed.open_buffer(Buffer::new(Text::from("x\n"), SelectionSet::default()));
    assert_ne!(bid2, focused_bid, "second buffer must not be focused");

    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid2, Some(lang));
    ed.drain_hooks();

    let mut host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert_eq!(
        host.settings().get_option("tab-width", bid2).unwrap(),
        OptionValue::Int(8),
        "explicit bid must read the hook's target buffer's override"
    );
    assert_eq!(
        host.settings()
            .get_option("tab-width", focused_bid)
            .unwrap(),
        OptionValue::Int(4),
        "the focused (non-target) buffer must still resolve to the global default"
    );
}

/// A global-only key rejected by `write_buffer`'s global-only arm is
/// reported as a hook error and leaves the global setting unchanged.
///
/// Fail oracle: drop the scope check in `write_buffer` (or bypass it) and
/// `scrolloff` would silently end up in the buffer's override slot instead
/// of erroring.
#[test]
fn set_buffer_option_global_only_key_errors_from_hook() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "scrolloff" 1)))"#,
    );
    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks();

    assert_eq!(
        ed.state.settings.scrolloff, 3,
        "global scrolloff must be untouched"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("global-only")),
        "expected a global-only-key hook error; messages: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}

/// `EditorHostImpl::set_buffer_option` returns `Err` for a stale bid instead
/// of panicking — `settings_ops::apply`'s `get_mut` panics on an unseeded
/// id, so the host method's own `try_get` guard must run first.
///
/// Fail oracle: remove the `try_get` guard added alongside this method —
/// this test panics ("unseeded BufferId") instead of observing an `Err`.
#[test]
fn host_set_buffer_option_invalid_bid_errors() {
    use hume_engine::pipeline::BufferId;
    use hume_scripting::host::EditorHost;

    let mut ed = editor_from("-[h]>ello\n");
    let mut host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host
        .settings()
        .set_buffer_option("tab-width", "8", BufferId::default());
    assert!(result.is_err(), "a stale bid must be rejected, not panic");
    let msg = result.unwrap_err();
    assert!(
        msg.contains("invalid buffer id"),
        "error must name the invalid bid; got: {msg}"
    );
}
