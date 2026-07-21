use super::*;

use crate::editor::minibuf::history::HistoryKind;

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
    // Fail oracle: revert set_global_option to a raw write_setting call
    // (bypassing settings_ops::apply) and the ring never resizes — the
    // trim assertion below fails (len stays 3).
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
        2,
        "ring must be trimmed to the new capacity with no manual pickup"
    );
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
    assert!(result.is_ok(), "eval must succeed: {result:?}");

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
    // this change `:set global theme=bad` persisted "bad" into settings
    // (store-then-load), unlike `:theme bad` (load-then-store) — the two
    // entry points disagreed. Fail oracle: same as above.
    let mut ed = editor_from("-[h]>ello\n");
    let result =
        crate::editor::commands::typed_set(&mut ed, Some("global theme=no_such_theme_xyz"), false);
    assert!(result.is_ok(), "command must not error: {result:?}");

    assert!(
        ed.state.settings.theme.is_empty(),
        "a theme that failed to load must not persist, got {:?}",
        ed.state.settings.theme
    );
}
