use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;
use pretty_assertions::assert_eq;

// ── core:vim-keybind — end-to-end plugin tests ────────────────────────────────
//
// Loads the *real* runtime/plugins/core/vim-keybind/plugin.scm (via
// `include_str!`) into an isolated HUME_RUNTIME dir, then evaluates an
// init.scm that eagerly loads it — exercising the actual shipped file rather
// than a hand-rolled stand-in.

const VIM_KEYBIND_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/vim-keybind/plugin.scm"
));

/// Build an editor with `core:vim-keybind` eagerly loaded via a real
/// `init.scm` + the real plugin file. Mirrors `setup_lazy_editor` in
/// `tests/plugins.rs`, but uses `HUME_RUNTIME` (core plugin resolution)
/// instead of a user data dir, and loads eagerly (no lazy stubs needed).
fn setup_vim_keybind_editor(input: &str) -> (Editor, HumeRuntimeGuard, tempfile::TempDir) {
    setup_vim_keybind_editor_with_config(input, None)
}

/// Like `setup_vim_keybind_editor`, but passes `config_expr` (a Scheme
/// expression, e.g. `(hash "change-to-eol" 'off)`) as `core:vim-keybind`'s
/// `#:config` — lets tests exercise `#:config` without hand-rolling the
/// plugin-dir setup or the surrounding `load-plugin` boilerplate.
fn setup_vim_keybind_editor_with_config(
    input: &str,
    config_expr: Option<&str>,
) -> (Editor, HumeRuntimeGuard, tempfile::TempDir) {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "vim-keybind", VIM_KEYBIND_PLUGIN);
    // `C`'s selection-width check dispatches to `stdlib/all-single-char?` via
    // `call!`, so vim-keybind depends on `core:stdlib` being loaded first.
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);

    let init_source = match config_expr {
        Some(cfg) => format!(
            "(load-plugin \"core:stdlib\")\n(load-plugin \"core:vim-keybind\" #:config {cfg})"
        ),
        None => "(load-plugin \"core:stdlib\")\n(load-plugin \"core:vim-keybind\")".to_string(),
    };

    let init_dir = safe_tempdir();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(&init_path, &init_source).unwrap();

    let mut ed = editor_from(input);
    let mut host = ScriptingHost::new();
    let effects = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed loading core:vim-keybind");
    // Mirror `init_scripting`: an eval's effects (here, the plugin's
    // `bind-key!` calls) only take hold once applied.
    ed.apply_script_effects(effects);
    ed.scripting = Some(host);
    (ed, guard, init_dir)
}

// ── Plugin rebinds ─────────────────────────────────────────────────────────────
//
// `$`/`^`/`0`/`G`/`Ctrl+6` each just `bind-key!` an already-tested native
// command (see `hume-ops/src/motion/tests/` and
// `tests/alternate.rs`) to a new key — one test spot-checks that the real
// plugin file's `bind-key!` lines are wired to the right command names,
// rather than one near-duplicate test per key.

#[test]
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

    let f1 = safe_named_tempfile();
    std::fs::write(f1.path(), "file1\n").unwrap();
    let f2 = safe_named_tempfile();
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

// ── `o` flip (Extend mode) ──────────────────────────────────────────────────────

/// The plugin's `(bind-key! 'extend "o" "flip-selections")` restores vim
/// visual-mode `o` — swap anchor and head — in Extend mode. Native HUME
/// already covers this via `Ctrl+e` (see `tests/commands.rs`'s `ctrl_e_*`
/// tests); this only checks the plugin's own binding wires up correctly.
#[test]
fn o_in_extend_mode_flips_selection() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key('o'));

    // anchor and head are swapped — selection is now backward.
    assert_eq!(state(&ed), "<[hell]-o\n");
    // extend mode is still active (flip doesn't exit it).
    assert_eq!(ed.state.mode, Mode::Extend);
}

// ── C / D / G ─────────────────────────────────────────────────────────────────

#[test]
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
fn shift_c_changes_to_eol_and_enters_insert() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("he-[l]>lo world\n");
    ed.handle_key(key('C'));
    assert_eq!(ed.doc().text().to_string(), "he\n");
    assert_eq!(ed.state.mode, Mode::Insert);
}

/// With a real (multi-char) selection already in place, `C` falls back to the
/// shadowed `copy-selection-on-next-line` instead of changing text — vim has
/// no bare-cursor gesture to match here, so HUME's multicursor idiom wins.
/// Mirrors `copy_next_line_range_selection` in
/// `hume-ops/src/selection_cmd/copy.rs`: a forward selection covering
/// "hello" is duplicated one line down with the same column span, buffer
/// text untouched.
#[test]
fn shift_c_with_selection_copies_to_next_line() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[hello]>\nworld\n");
    ed.handle_key(key('C'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\nworld\n",
        "buffer must be unchanged — C must not edit text when the selection spans more than one char"
    );
    assert_eq!(state(&ed), "-[hello]>\n-[world]>\n");
    assert_eq!(ed.state.mode, Mode::Normal);
}

/// A count prefix always wins over the collapsed-cursor vim gesture, even on
/// a bare cursor: `1C` duplicates the selection onto the next line instead of
/// changing to end of line.
#[test]
fn shift_c_with_count_1_copies_instead_of_changing() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\nworld\n");
    ed.handle_key(key('1'));
    ed.handle_key(key('C'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\nworld\n",
        "buffer must be unchanged — a count prefix must not trigger the change-to-eol branch"
    );
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert_eq!(heads.len(), 2, "copy-selection-on-next-line adds a cursor");
    assert!(heads.contains(&0), "original cursor stays at col 0 line 0");
    assert!(heads.contains(&6), "new cursor lands at col 0 line 1");
    assert_eq!(ed.state.mode, Mode::Normal);
}

/// `3C` on a bare cursor forwards the count to `copy-selection-on-next-line`,
/// duplicating onto all three lines below — not just gating on "count present
/// or not".
#[test]
fn shift_c_with_count_3_copies_onto_three_lines() {
    // "hello\nworld\nfoo\nbar\n": col 0 of each line is offset 0, 6, 12, 16.
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\nworld\nfoo\nbar\n");
    ed.handle_key(key('3'));
    ed.handle_key(key('C'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\nworld\nfoo\nbar\n",
        "buffer must be unchanged"
    );
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert_eq!(
        heads.len(),
        4,
        "count=3 must add one copy per line below, not just one"
    );
    assert!(heads.contains(&0), "original cursor at col 0 of line 0");
    assert!(heads.contains(&6), "copy at col 0 of line 1");
    assert!(heads.contains(&12), "copy at col 0 of line 2");
    assert!(heads.contains(&16), "copy at col 0 of line 3");
}

/// `:` invocation hands the typed count over as a string (`ArgSource::Minibuf`
/// in `hume-editor/src/editor/dispatch.rs`), not the integer that keymap
/// dispatch supplies — the command must normalize it instead of raising a
/// type error comparing a string to `0`.
#[test]
fn vim_change_to_eol_or_copy_line_typed_with_count_copies() {
    // "hello\nworld\nfoo\nbar\n": col 0 of each line is offset 0, 6, 12, 16.
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\nworld\nfoo\nbar\n");
    ed.handle_key(key(':'));
    for ch in "vim-change-to-eol-or-copy-line 3".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(
        ed.doc().text().to_string(),
        "hello\nworld\nfoo\nbar\n",
        "buffer must be unchanged"
    );
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert_eq!(
        heads.len(),
        4,
        "typed count=3 must add one copy per line below, same as keymap dispatch"
    );
    assert!(heads.contains(&0), "original cursor at col 0 of line 0");
    assert!(heads.contains(&6), "copy at col 0 of line 1");
    assert!(heads.contains(&12), "copy at col 0 of line 2");
    assert!(heads.contains(&16), "copy at col 0 of line 3");
}

/// A count prefix combined with an already-wide selection still forwards the
/// count — the wide-selection fallback and the count fallback compose rather
/// than one silently overriding the other.
#[test]
fn shift_c_with_count_and_selection_copies_with_count() {
    // "hello\nworld\nfoo\n": head col 4 ('o') of "hello" copies to "world"
    // col 4 ('d', offset 10), then to "foo" — col 4 overshoots "foo"'s 3
    // chars, clamping to its last char ('o', offset 14).
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[hello]>\nworld\nfoo\n");
    ed.handle_key(key('2'));
    ed.handle_key(key('C'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\nworld\nfoo\n",
        "buffer must be unchanged"
    );
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert_eq!(
        heads.len(),
        3,
        "count=2 must add two copies of the wide selection, not one"
    );
    assert!(heads.contains(&4), "original head at 'o' of \"hello\"");
    assert!(heads.contains(&10), "copy at 'd' of \"world\"");
    assert!(
        heads.contains(&14),
        "copy clamped to 'o', last char of \"foo\""
    );
}

/// `C` shadows the default `copy-selection-on-next-line` binding while the
/// plugin is loaded — the multicursor command stays reachable by name.
#[test]
fn shift_c_shadows_copy_selection_command() {
    let (ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\n");
    assert!(
        ed.state
            .config
            .registry
            .get_mappable("copy-selection-on-next-line")
            .is_some(),
        "the shadowed command must still be registered and callable by name"
    );
}

// ── #:config "change-to-eol" ─────────────────────────────────────────────────

/// `#:config (hash "change-to-eol" 'off)` drops the `C` binding, restoring the
/// native `copy-selection-on-next-line` (adds a second cursor on the next
/// line, leaves the buffer untouched) instead of the vim change-to-eol.
#[test]
fn shift_c_with_change_to_eol_off_restores_copy_selection() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor_with_config(
        "-[h]>ello\nworld\n",
        Some(r#"(hash "change-to-eol" 'off)"#),
    );
    ed.handle_key(key('C'));

    assert_eq!(
        ed.doc().text().to_string(),
        "hello\nworld\n",
        "buffer must be unchanged — C must not edit text when change-to-eol 'off drops the vim override"
    );
    assert_eq!(ed.state.mode, Mode::Normal);

    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert_eq!(
        heads.len(),
        2,
        "copy-selection-on-next-line adds a second cursor"
    );
    assert!(heads.contains(&0), "original cursor stays at col 0 line 0");
    assert!(heads.contains(&6), "new cursor lands at col 0 line 1");
}

/// `#:config "change-to-eol" 'off` only affects `C` — `D` and `G` (which
/// shadow nothing) stay bound to their vim behavior.
#[test]
fn change_to_eol_off_does_not_affect_non_shadowing_bindings() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor_with_config(
        "-[h]>ello\nworld\n",
        Some(r#"(hash "change-to-eol" 'off)"#),
    );
    ed.handle_key(key('D'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\nworld\n",
        "D must still delete to end of line"
    );
    ed.handle_key(key('G'));
    assert_eq!(state(&ed), "\n-[w]>orld\n", "G must still go to last line");
}

/// `#:config (hash "change-to-eol" 'on)` makes `C` change to end of line
/// unconditionally, even with a real (multi-char) selection active — unlike
/// the default `'smart` behavior (see `shift_c_with_selection_copies_to_next_line`),
/// which falls back to `copy-selection-on-next-line` in that case.
#[test]
fn shift_c_with_change_to_eol_on_ignores_selection_width() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor_with_config(
        "-[hello]>\nworld\n",
        Some(r#"(hash "change-to-eol" 'on)"#),
    );
    ed.handle_key(key('C'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\nworld\n",
        "change-to-eol 'on must change to EOL even with a real selection active"
    );
    assert_eq!(ed.state.mode, Mode::Insert);
}

// ── Dot-repeat ────────────────────────────────────────────────────────────────

/// `D` then `.` on a different line must repeat "delete to end of line" at
/// the new cursor. `vim-delete-to-eol` carries no `#:repeatable` flag — native
/// `delete` is itself repeatable and self-captures the preceding
/// `goto-line-end` (extend) step via the shared selection-recipe accumulator,
/// so the composite replays correctly with no cooperation needed from the
/// Steel wrapper.
#[test]
fn shift_d_is_dot_repeatable() {
    let (mut ed, _guard, _dir) = setup_vim_keybind_editor("-[h]>ello\nworld\n");
    ed.feed_key(key('D')); // delete "hello" → line becomes empty
    assert_eq!(ed.doc().text().to_string(), "\nworld\n");

    ed.feed_key(key('j')); // move down onto "world", column preserved at 0
    ed.feed_key(key('.')); // repeat: delete to end of line from there
    assert_eq!(ed.doc().text().to_string(), "\n\n");
}

// ── core:stdlib dependency check ───────────────────────────────────────────────

/// Loading `core:vim-keybind` with the default `'smart` `change-to-eol` but
/// without `core:stdlib` loaded first must fail `eval_init` at load time,
/// naming `core:stdlib` — not silently succeed and leave `C` picking the
/// wrong branch the first time it's pressed.
#[test]
fn smart_change_to_eol_without_stdlib_errors_at_load() {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "vim-keybind", VIM_KEYBIND_PLUGIN);
    // Deliberately no `write_core_plugin(&guard, "stdlib", ...)`.

    let init_dir = safe_tempdir();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "core:vim-keybind")"#).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    let err = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("'smart change-to-eol without core:stdlib must fail eval_init");
    assert!(
        err.message.contains("core:stdlib"),
        "error must name the missing dependency; got: {err:?}"
    );
}

/// `core:vim-keybind`'s `core:stdlib` guard is unconditional (its config
/// read always calls `stdlib/config-enum`, whatever `change-to-eol` resolves
/// to) — `'off` fails to load without `core:stdlib` too, naming it. Replaces
/// the former `change_to_eol_off_does_not_require_stdlib`, which pinned the
/// opposite contract from when only `'smart` depended on `core:stdlib`.
#[test]
fn change_to_eol_off_also_requires_stdlib() {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "vim-keybind", VIM_KEYBIND_PLUGIN);
    // Deliberately no `write_core_plugin(&guard, "stdlib", ...)`.

    let init_dir = safe_tempdir();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'off))"#,
    )
    .unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    let err = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("'off change-to-eol without core:stdlib must fail eval_init");
    assert!(
        err.message.contains("core:stdlib"),
        "error must name the missing dependency; got: {err:?}"
    );
}

/// A `change-to-eol` value outside `'on`/`'smart`/`'off` must fail the load
/// with `stdlib/config-enum`'s message — naming the plugin, the key, and the
/// offending value — rather than the dispatch `cond`'s old `else` arm (now
/// dead: `config-enum` has already rejected anything not in the allowed set).
#[test]
fn change_to_eol_bogus_value_fails_load_with_enum_message() {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "vim-keybind", VIM_KEYBIND_PLUGIN);
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);

    let init_dir = safe_tempdir();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:vim-keybind\" #:config (hash \"change-to-eol\" 'bogus))",
    )
    .unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut host = ScriptingHost::new();
    let err = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("a bogus change-to-eol value must fail eval_init");
    assert!(
        err.message.contains("core:vim-keybind")
            && err.message.contains("change-to-eol")
            && err.message.contains("bogus"),
        "error must name the plugin, the key, and the offending value; got: {:?}",
        err.message
    );
}
