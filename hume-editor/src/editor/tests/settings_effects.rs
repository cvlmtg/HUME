use super::*;
use hume_grid::{Rect, Rgb};
use hume_platform::dirs::Dirs;

use crate::editor::buffer::Buffer;
use crate::editor::message_log::Severity;
use crate::editor::minibuf::history::HistoryKind;
use hume_editing::text::BufferText;
use hume_engine::pipeline::RenderContext;

/// Drive `(set-option! ...)` through the real Steel path
/// (`EditorHostImpl::set_global_option`).
fn eval_set_option(ed: &mut Editor, source: &str) -> Result<(), String> {
    let names: Vec<String> = ed
        .state
        .config
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = hume_scripting::ScriptingHost::new(&Dirs::none());
    host.register_command_names(&name_refs);
    let mut init_host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(source, &mut init_host).map(|_| ())
}

// ── set-option! resyncs derived state (the gap this closes) ────────────────

#[test]
fn set_option_applies_history_capacity() {
    // If set_global_option called crate::editor::settings::write_global
    // directly, skipping settings::ops::apply_global's resync step,
    // settings.history_capacity would still update. Only the post-push
    // assertion at the bottom checks that state.history's real capacity
    // was resynced.
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
    // As above: settings.jump_list_capacity would update even without
    // settings::ops::apply_global's resync step. The post-push assertion at
    // the bottom is the one that checks the live jump list's capacity.
    let mut ed = editor_from("-[h]>ello\n");
    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    let text = ed.doc().text();
    let at_start = test_fixtures::testing::single(text, test_fixtures::testing::cursor(text, 0));
    for i in 0..5 {
        ed.state.panes.jumps[pid].push(crate::editor::jump_list::JumpEntry {
            buffer_id: bid,
            selections: at_start.clone(),
            primary_line: hume_rope::line::ContentLine::new(i),
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

    // The overshoot (5 -> new cap 2) is more than one entry; the next push
    // must still converge to the cap in this one call.
    ed.state.panes.jumps[pid].push(crate::editor::jump_list::JumpEntry {
        buffer_id: bid,
        selections: at_start.clone(),
        primary_line: hume_rope::line::ContentLine::new(5),
    });
    assert_eq!(
        ed.state.panes.jumps[pid].len(),
        2,
        "the next push must apply the resynced capacity with no manual pickup"
    );
}

// ── :set global mouse / mouse-select resync per frame ─────────────

/// `mouse`/`mouse-select` are terminal modes applied once at startup
/// (`hume_platform::terminal::init`, called from `hume-editor/src/lib.rs`
/// before entering the event loop); there is no other write side.
/// `prepare_frame` calls `resync_mouse_mode` every frame, re-applying the
/// terminal mode whenever it drifts from `state.settings`, so `:set global
/// mouse=false` takes effect without restarting.
///
/// No `SharedTerm` exists in test `Editor`s (`Editor::for_testing`/`open`
/// both seed `terminal: None`), so this can't assert on emitted escape
/// bytes. That path is covered by `hume-platform`'s
/// `mouse_enable_without_select_emits_1000_and_1006_only` and friends
/// (`hume-platform/src/terminal/tests.rs`). This asserts the terminal-mode
/// tracking state itself (`Editor::applied_mouse_mode`) resyncs to the new
/// setting on the next `prepare_frame`, which is the part `resync_mouse_mode`
/// can do headless. The `if let Some(term)` write is a one-line guard
/// around the same comparison, exercised whenever a real terminal is
/// attached.
///
/// Without the `self.resync_mouse_mode()` call in `prepare_frame`,
/// `applied_mouse_mode` would keep its constructor default `(true, false)`
/// after this `:set`.
#[test]
fn set_global_mouse_resyncs_applied_mode_next_frame() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_eq!(
        ed.applied_mouse_mode,
        (true, false),
        "default EditorSettings has mouse=true, mouse_select=false"
    );

    let fp = FocusedPane::current(&ed.state);
    crate::editor::commands::typed_set(&mut ed, fp, Some("global mouse=false"), false)
        .expect("set mouse");
    assert_eq!(
        ed.applied_mouse_mode,
        (true, false),
        "the setting write itself must not resync; only prepare_frame does"
    );

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        ed.applied_mouse_mode,
        (false, false),
        "prepare_frame must resync the terminal mouse mode from the new setting"
    );
}

// ── statusline.mode-colors gates whole-row tinting ─────────────────────────

#[test]
fn set_option_statusline_mode_colors_gates_whole_row_tint() {
    // `HumeStatusline::render` checks `statusline_mode_colors` before
    // resolving the real mode. Without that check the row would still tint
    // cyan in Insert mode with colors off.
    //
    // The fixture theme gives `ui.statusline` and `ui.statusline.normal`
    // *different* backgrounds. Every bundled theme makes them equal, which
    // would let the off-state assertion pass whether the opt-out reads the
    // base scope (correct) or silently substitutes `EditorMode::Normal`
    // (an imported theme with a distinct Normal-mode accent, e.g. Helix's
    // old pill idiom, would then still tint the row).
    use hume_engine::types::{ResolvedStyle, Scope};

    let mut ed = editor_from("-[h]>ello\n");
    let rect = Rect::new(0, 0, 40, 8);
    let row = rect.bottom() - 1;

    let mut styles = std::collections::HashMap::new();
    styles.insert(
        "ui.statusline",
        ResolvedStyle {
            bg: Some(Rgb(64, 64, 64)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.normal",
        ResolvedStyle {
            bg: Some(Rgb(255, 0, 0)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.statusline.insert",
        ResolvedStyle {
            bg: Some(Rgb(0, 255, 255)),
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

    // Default: statusline.mode-colors is on: the row tints for Insert.
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
    // Same idea for the undo-levels arm: without the inline resync, the
    // second edit below would stay undoable.
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
    // A bad theme name from `set-option!` (e.g. from a lazily-activated
    // plugin) must not stay in settings.theme, where it would later be
    // reported as "current theme" even though it never loaded. The rollback
    // in settings::ops::apply keeps settings.theme empty here.
    let mut ed = editor_from("-[h]>ello\n");
    let result = eval_set_option(&mut ed, r#"(set-option! "theme" "no_such_theme_xyz")"#);
    // apply_global surfaces a failed theme load as an Err (rather than
    // Ok(()) with only a message-log entry) so a plugin's own
    // (with-handler ...) around set-option! actually fires. An Ok here
    // would hide the failure from Steel callers.
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
    // Same rollback, via `:set global` instead of Steel. Without it,
    // `:set global theme=bad` would persist "bad" (store-then-load) while
    // `:theme bad` would not (load-then-store).
    let mut ed = editor_from("-[h]>ello\n");
    let fp = FocusedPane::current(&ed.state);
    let result = crate::editor::commands::typed_set(
        &mut ed,
        fp,
        Some("global theme=no_such_theme_xyz"),
        false,
    );
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
    // :theme's own load-then-store behavior must hold when it delegates to
    // settings::ops::apply. Mirrors
    // editor::tests::commands::load_theme_by_name_fails_gracefully, but
    // through the typed_theme entry point instead of calling the loader
    // directly.
    // :theme shares settings::ops::apply's rollback with the theme tests
    // above, so without it settings.theme would end up "no_such_theme_xyz"
    // here too.
    let mut ed = editor_from("-[h]>ello\n");
    let fp = FocusedPane::current(&ed.state);
    let result =
        crate::editor::commands::typed_theme(&mut ed, fp, Some("no_such_theme_xyz"), false);
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

#[test]
fn typed_theme_sets_setting_on_success() {
    // A typo in the key string typed_theme delegates with (e.g. "themes")
    // would make write_global return Err("unknown setting").
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = repo_runtime_dirs();
    let fp = FocusedPane::current(&ed.state);
    let result = crate::editor::commands::typed_theme(&mut ed, fp, Some("gruvbox"), false);
    assert!(result.is_ok(), "command must not error: {result:?}");
    assert_eq!(ed.state.settings.theme, "gruvbox");
}

// ── set-buffer-option! from an on-language-set hook ─────────────────────────

/// `set-buffer-option!`, called from an `on-language-set` hook with the
/// hook's own `bid`, writes the target buffer's override and leaves the
/// global setting untouched, proving the write lands in `BufferOverrides`,
/// not `EditorSettings`.
#[test]
fn set_buffer_option_from_hook_writes_target_override() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "tab-width" 8)))"#,
    );
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    assert_eq!(ed.state.buffers.get(bid).overrides.tab_width, Some(8));
    assert_eq!(
        ed.state.settings.tab_width, 4,
        "global tab-width must be untouched by a buffer-scoped write"
    );
}

/// End-to-end version of the `word-chars` per-language recipe documented in
/// `configuration.md`: an `on-language-set` hook sets `word-chars` for one
/// language, and `w` immediately honors it in that buffer.
#[test]
fn on_language_set_hook_configures_word_chars() {
    let mut ed = editor_from("-[f]>oo-bar baz\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (when (equal? lang "css") (set-buffer-option! bid "word-chars" "-"))))"#,
    );
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("css");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).overrides.word_chars.as_deref(),
        Some("-")
    );

    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo-bar-[ baz]>\n");
}

/// `(get-buffer-option bid "word-chars")` round-trips the raw string;
/// this covers the new `option_value!` arm.
#[test]
fn get_buffer_option_round_trips_word_chars() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>b\n");
    ed.state.settings.word_chars = "-".into();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "check" "" (lambda (bid)
             (log! 'info (get-buffer-option bid "word-chars"))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "-");
}

/// The feature this scope layer exists for: setting wrap-mode per file type
/// from an `on-language-set` hook via `set-buffer-option!`. The open pane
/// (never explicitly `:wrap`'d or `:set pane`'d) picks up the change
/// immediately: resolution is lazy (pane → buffer → global), not a seed
/// applied only to panes opened afterward.
///
/// A global-only `wrap-mode` would make `set-buffer-option!` error before
/// either assertion below is reached.
#[test]
fn set_buffer_option_wrap_mode_from_hook_changes_the_open_pane() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "wrap-mode" "word")))"#,
    );
    assert_eq!(
        ed.focused_wrap_mode(),
        hume_engine::pane::WrapMode::Indent { width: 0 },
        "sanity: default before the hook fires"
    );

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    assert_eq!(
        ed.focused_wrap_mode(),
        hume_engine::pane::WrapMode::Word { width: 0 },
        "the open pane follows the hook's buffer-scoped write"
    );
}

/// The hook's `bid` argument, not the focused buffer, is the write target.
/// Pins the distinction that `settle` runs with the *focused* buffer as
/// scripting context while the hook's own `bid` may name a background
/// buffer.
#[test]
fn set_buffer_option_targets_hook_bid_not_focused_buffer() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "tab-width" 8)))"#,
    );
    let focused_bid = ed.focused_buffer_id();
    let bid2 = ed.open_buffer(Buffer::at_start(BufferText::from("x\n")));
    assert_ne!(bid2, focused_bid, "second buffer must not be focused");

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid2, Some(lang));
    ed.settle();

    assert_eq!(ed.state.buffers.get(bid2).overrides.tab_width, Some(8));
    assert_eq!(
        ed.state.buffers.get(focused_bid).overrides.tab_width,
        None,
        "the focused (non-target) buffer must be untouched"
    );
}

/// `get-buffer-option`'s explicit `bid` argument, mirrored here at the host
/// layer (`SettingsHost::get_buffer_option`, same as `set_buffer_option`),
/// reads the *named* buffer's override, not the focused buffer's: the
/// read-side half of the same hook-bid distinction
/// `set_buffer_option_targets_hook_bid_not_focused_buffer` pins for writes.
#[test]
fn get_buffer_option_explicit_bid_reads_hook_target_not_focused_buffer() {
    use hume_scripting::host::{EditorHost, OptionValue};

    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "tab-width" 8)))"#,
    );
    let focused_bid = ed.focused_buffer_id();
    let bid2 = ed.open_buffer(Buffer::at_start(BufferText::from("x\n")));
    assert_ne!(bid2, focused_bid, "second buffer must not be focused");

    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid2, Some(lang));
    ed.settle();

    let mut host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert_eq!(
        host.settings()
            .get_buffer_option("tab-width", bid2)
            .unwrap(),
        OptionValue::Int(8),
        "explicit bid must read the hook's target buffer's override"
    );
    assert_eq!(
        host.settings()
            .get_buffer_option("tab-width", focused_bid)
            .unwrap(),
        OptionValue::Int(4),
        "the focused (non-target) buffer must still resolve to the global default"
    );
}

/// `get-buffer-option` on a closed buffer id must error, matching
/// `set-buffer-option!`'s own `try_get` guard (`EditorHostImpl::set_buffer_option`)
/// (a stale bid is invalid input, not a request for "whatever the global
/// default is").
#[test]
fn get_buffer_option_closed_bid_errors() {
    use hume_scripting::host::EditorHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid2 = ed.open_buffer(Buffer::at_start(BufferText::from("x\n")));
    ed.close_buffer(bid2);

    let mut host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let err = host
        .settings()
        .get_buffer_option("tab-width", bid2)
        .expect_err("a closed bid must error, not silently read the global default");
    assert!(
        err.contains("invalid buffer id"),
        "error must name the actual problem (invalid bid), not an unrelated message; got: {err}"
    );
}

/// A global-only key rejected by `write_buffer`'s global-only arm is
/// reported as a hook error and leaves the global setting unchanged.
///
/// Without the scope check in `write_buffer`, `scroll-margin` would silently end
/// up in the buffer's override slot.
#[test]
fn set_buffer_option_global_only_key_errors_from_hook() {
    let mut ed = editor_from("-[a]>b\n");
    crate::editor::tests::language::attach_host(
        &mut ed,
        r#"(register-hook! 'on-language-set (lambda (bid lang) (set-buffer-option! bid "scroll-margin" 1)))"#,
    );
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    assert_eq!(
        ed.state.settings.scroll_margin, 3,
        "global scroll-margin must be untouched"
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
/// of panicking. `settings::ops::apply`'s `get_mut` panics on an unseeded
/// id, so the host method's own `try_get` guard must run first.
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

// ── cursor-shape-insert reaches the render settings ───────────────────────

/// `cursor-shape-insert` only ever changes the *Insert* cursor: every other
/// mode is hardwired to a block, because a terminal has one hardware cursor
/// and HUME's prompt modes park it in the minibuf instead. Reads
/// `EditorState::cursor_shape`, the single source both the terminal-cursor
/// branch in `Editor::run` and `resolve_pane_settings` consult.
#[test]
fn cursor_shape_insert_only_applies_to_insert_mode() {
    let mut ed = editor_from("-[a]>bc\n");
    eval_set_option(&mut ed, r#"(set-option! "cursor-shape-insert" "bar")"#).unwrap();

    assert_eq!(ed.state.mode(), Mode::Normal, "sanity: starts in Normal");
    assert_eq!(
        ed.state.cursor_shape(),
        crate::editor::settings::CursorShape::Block,
        "Normal is hardwired to a block regardless of the setting"
    );

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert");
    assert_eq!(
        ed.state.cursor_shape(),
        crate::editor::settings::CursorShape::Bar,
        "Insert is the one mode the setting reaches"
    );
}

/// The focused pane's `cursor_is_block` is exactly "the resolved shape for
/// the live mode is Block", so flipping the setting flips it, and with it
/// whether `style_display_line` paints either selection head at all. An *unfocused*
/// pane is always `true` regardless: no real terminal cursor sits there to
/// stand in for the painted one, so its heads must be drawn either way.
#[test]
fn cursor_shape_insert_gates_head_painting_in_the_focused_pane_only() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.execute_typed("vsplit", None).unwrap();
    let focused = ed.state.focus.id();
    let other = ed
        .view
        .panes
        .every_pane_across_all_tabs()
        .map(|(pid, _)| pid)
        .find(|&p| p != focused)
        .expect("vsplit must leave a second pane");

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert");

    for (shape, expected_focused) in [("bar", false), ("underline", false), ("block", true)] {
        eval_set_option(
            &mut ed,
            &format!(r#"(set-option! "cursor-shape-insert" "{shape}")"#),
        )
        .unwrap();
        assert_eq!(
            ed.resolve_pane_settings(focused).cursor_is_block,
            expected_focused,
            "focused pane with cursor-shape-insert={shape}"
        );
        assert!(
            ed.resolve_pane_settings(other).cursor_is_block,
            "an unfocused pane paints its heads for every shape (cursor-shape-insert={shape})"
        );
    }
}

/// With `cursor-shape-insert=block` the painted head *is* the cursor, so the
/// real terminal cursor must be hidden, the same rule the default `bar`
/// inverts (`insert_mode_hides_cursor_only_in_focused_pane` covers that side).
/// Asserts on the resolved shape rather than the escape byte: `Editor::run`
/// maps it to `CursorStyle` with no branch of its own beyond this value.
#[test]
fn insert_block_shape_suppresses_the_terminal_cursor() {
    let mut ed = editor_from("-[a]>bc\n");
    eval_set_option(&mut ed, r#"(set-option! "cursor-shape-insert" "block")"#).unwrap();
    ed.feed_key(key('i'));

    assert_eq!(
        ed.state.cursor_shape(),
        crate::editor::settings::CursorShape::Block,
        "block is what `Editor::run` tests to decide the cursor is not drawn"
    );
}
