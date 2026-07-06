use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;
use pretty_assertions::assert_eq;

// ── core:vim-keybind — end-to-end plugin tests ────────────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).
//
// Loads the *real* runtime/plugins/core/vim-keybind/plugin.scm (via
// `include_str!`) into an isolated HUME_RUNTIME dir, then evaluates an
// init.scm that eagerly loads it — exercising the actual shipped file rather
// than a hand-rolled stand-in.

#[cfg(not(windows))]
const VIM_KEYBIND_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/vim-keybind/plugin.scm"
));

/// Build an editor with `core:vim-keybind` eagerly loaded via a real
/// `init.scm` + the real plugin file. Mirrors `setup_lazy_editor` in
/// `tests/plugins.rs`, but uses `HUME_RUNTIME` (core plugin resolution)
/// instead of a user data dir, and loads eagerly (no lazy stubs needed).
#[cfg(not(windows))]
fn setup_vim_keybind_editor(input: &str) -> (Editor, HumeRuntimeGuard, tempfile::TempDir) {
    let guard = HumeRuntimeGuard::new();
    let plugin_dir = guard
        .runtime
        .path()
        .join("plugins")
        .join("core")
        .join("vim-keybind");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), VIM_KEYBIND_PLUGIN).unwrap();

    let init_dir = tempfile::tempdir().unwrap();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "core:vim-keybind")"#).unwrap();

    let mut ed = editor_from(input);
    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed loading core:vim-keybind");
    ed.scripting = Some(host);
    (ed, guard, init_dir)
}

// ── Plugin rebinds ─────────────────────────────────────────────────────────────
//
// `$`/`^`/`0`/`G`/`Ctrl+6` each just `bind-key!` an already-tested native
// command (see `hume-editor/src/ops/motion/tests.rs` and
// `tests/alternate.rs`) to a new key — one test spot-checks that the real
// plugin file's `bind-key!` lines are wired to the right command names,
// rather than one near-duplicate test per key.

#[test]
#[cfg(not(windows))]
fn plugin_rebinds_line_and_alternate_motions() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("  hel-[l]>o world\nfoo\nbar\n");

    ed.handle_key(key('^')); // first non-blank
    assert_eq!(state(&ed), "  -[h]>ello world\nfoo\nbar\n");

    ed.handle_key(key('$')); // end of line
    assert_eq!(state(&ed), "  hello worl-[d]>\nfoo\nbar\n");

    ed.handle_key(key('0')); // start of line
    assert_eq!(state(&ed), "-[ ]> hello world\nfoo\nbar\n");

    ed.handle_key(key('G')); // last line
    assert_eq!(state(&ed), "  hello world\nfoo\n-[b]>ar\n");

    let f1 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f1.path(), "file1\n").unwrap();
    let f2 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f2.path(), "file2\n").unwrap();
    ed.execute_typed("e", Some(f1.path().to_str().unwrap()))
        .unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(f2.path().to_str().unwrap()))
        .unwrap();

    ed.handle_key(key_ctrl('6')); // alternate buffer
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        "Ctrl+6 must switch to alternate"
    );
}

// ── C / D / G ─────────────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn shift_d_deletes_to_eol() {
    // goto-line-end(extend) turns the single-char selection on 'l' (index 2)
    // into a forward selection covering "llo world" (anchor stays at 2, head
    // moves to the last char before the trailing \n). delete_selection then
    // removes it and lands the cursor on the structural '\n' left behind —
    // same semantics as `delete_selection_multi_char_forward`.
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("he-[l]>lo world\n");
    ed.handle_key(key('D'));
    assert_eq!(state(&ed), "he-[\n]>");
    assert_eq!(ed.state.mode, Mode::Normal);
}

#[test]
#[cfg(not(windows))]
fn shift_c_changes_to_eol_and_enters_insert() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("he-[l]>lo world\n");
    ed.handle_key(key('C'));
    assert_eq!(ed.doc().text().to_string(), "he\n");
    assert_eq!(ed.state.mode, Mode::Insert);
}

/// `C` shadows the default `copy-selection-on-next-line` binding while the
/// plugin is loaded — the multicursor command stays reachable by name.
#[test]
#[cfg(not(windows))]
fn shift_c_shadows_copy_selection_command() {
    let (ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\n");
    assert!(
        ed.state
            .registry
            .get_mappable("copy-selection-on-next-line")
            .is_some(),
        "the shadowed command must still be registered and callable by name"
    );
}

// ── Dot-repeat ────────────────────────────────────────────────────────────────

/// `D` then `.` on a different line must repeat "delete to end of line" at
/// the new cursor. `vim-delete-to-eol` carries no `#:repeatable` flag — native
/// `delete` is itself repeatable and self-captures the preceding
/// `goto-line-end` (extend) step via the shared selection-recipe accumulator,
/// so the composite replays correctly with no cooperation needed from the
/// Steel wrapper.
#[test]
#[cfg(not(windows))]
fn shift_d_is_dot_repeatable() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\nworld\n");
    ed.feed_key(key('D')); // delete "hello" → line becomes empty
    assert_eq!(ed.doc().text().to_string(), "\nworld\n");

    ed.feed_key(key('j')); // move down onto "world", column preserved at 0
    ed.feed_key(key('.')); // repeat: delete to end of line from there
    assert_eq!(ed.doc().text().to_string(), "\n\n");
}

// ── Defaults no longer bind these keys without the plugin ─────────────────────

#[test]
fn without_plugin_dollar_caret_zero_ctrl6_are_noops() {
    let mut ed = editor_from("-[h]>ello world\n");
    let before = state(&ed);
    ed.handle_key(key('$'));
    assert_eq!(
        state(&ed),
        before,
        "$ must be inert without core:vim-keybind"
    );
    ed.handle_key(key('^'));
    assert_eq!(
        state(&ed),
        before,
        "^ must be inert without core:vim-keybind"
    );
    ed.handle_key(key('0'));
    assert_eq!(
        state(&ed),
        before,
        "0 must be inert without core:vim-keybind"
    );
    ed.handle_key(key_ctrl('6'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+6 must be inert without core:vim-keybind"
    );
}
