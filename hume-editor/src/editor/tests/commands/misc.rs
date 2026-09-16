//! misc.rs — everything left: extend-mode exit rules, undo grouping, line text objects, typed-command arity, and case transforms.

use super::super::*;
use pretty_assertions::assert_eq;

// ── Extend mode exits after selection-consuming edits ────────────────────────
//
// Mirrors Vim visual-mode: any operator on a visual selection returns to Normal.
// Yank is the deliberate exception — it is non-destructive and preserves the
// selection (Helix-like).

#[test]
fn extend_exits_after_delete() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.input.set_extend(true);
    ed.handle_key(key('d'));
    assert_eq!(ed.state.mode(), Mode::Normal, "delete exits Extend");
}

#[test]
fn extend_exits_after_replace() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.input.set_extend(true);
    ed.handle_key(key('r'));
    ed.handle_key(key('x')); // replacement char completes replace
    assert_eq!(ed.state.mode(), Mode::Normal, "replace exits Extend");
}

#[test]
fn extend_exits_after_paste() {
    // Pre-populate a register so paste does real work.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('y')); // yank "h" into ring
    ed.state.input.set_extend(true);
    ed.handle_key(key('p')); // smart-paste-after
    assert_eq!(ed.state.mode(), Mode::Normal, "paste exits Extend");
}

#[test]
fn extend_preserved_after_yank() {
    // Yank must NOT exit Extend — it is non-destructive and the selection stays live.
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.input.set_extend(true);
    ed.handle_key(key('y'));
    assert_eq!(ed.state.mode(), Mode::Extend, "yank preserves Extend");
}

// ── `o`/`O` undo grouping ─────────────────────────────────────────────────────

/// `o` must group the structural newline insertion and the subsequent insert
/// session into one undo step. Without the fix, the newline would be a
/// separate `apply_edit` revision, so `u` would only undo the typed text and
/// leave behind an empty line.
#[test]
fn o_groups_newline_and_insert_session_into_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('o'));
    assert_eq!(ed.state.mode(), Mode::Insert);

    ed.handle_key(key('w'));
    ed.handle_key(key('o'));
    ed.handle_key(key('r'));
    ed.handle_key(key('l'));
    ed.handle_key(key('d'));

    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hello\nworld\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(!ed.doc().can_undo());
}

/// Same undo-grouping invariant for `O` (open line above).
#[test]
fn capital_o_groups_newline_and_insert_session_into_one_undo_step() {
    let mut ed = editor_from("foo\n-[b]>ar\n");

    ed.handle_key(key('O'));
    assert_eq!(ed.state.mode(), Mode::Insert);

    ed.handle_key(key('n'));
    ed.handle_key(key('e'));
    ed.handle_key(key('w'));

    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "foo\nnew\nbar\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "foo\n-[b]>ar\n");
    assert!(!ed.doc().can_undo());
}

// ── Plain insert session groups all chars into one undo step ──────────────

/// `i` with a non-collapsed selection must collapse to the start of the
/// selection and enter Insert — it must NOT replace the selected text.
#[test]
fn i_collapses_selection_to_start() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('i'));

    assert_eq!(ed.state.mode(), Mode::Insert);
    // Cursor collapsed to 'h' — nothing deleted.
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

/// `i` + typing + `Esc` must commit as one undo step, just like `c`. A single
/// `u` should restore the original buffer — not leave partial edits behind.
#[test]
fn i_groups_insert_session_into_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert);

    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));

    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "XYhello\n");

    // One undo restores the original state completely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");

    // Only one revision was recorded.
    assert!(!ed.doc().can_undo());
}

// ── Line text objects (mil / mal) ─────────────────────────────────────────────

#[test]
fn mil_selects_line_content_excluding_newline() {
    let mut ed = editor_from("hell-[o]> world\nsecond\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[hello world]>\nsecond\n");
}

#[test]
fn mal_selects_line_including_newline() {
    let mut ed = editor_from("hell-[o]> world\nsecond\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('a'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[hello world\n]>second\n");
}

#[test]
fn mil_on_empty_line_is_noop() {
    // An empty line has no content — selection should not change.
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

// ── Typed-command arity rule for Steel commands ───────────────────────────

/// Wire up a Steel *typed* command and return the editor + scripting host
/// ready for use.
///
/// Uses `EditorHostImpl` so `define-typed-command!` registers the command
/// directly into the editor's `CommandRegistry` inline. The `arity` /
/// `is_variadic` override re-registers with explicit values (useful when the
/// test arity differs from what Steel infers).
fn setup_typed_arity_test(src: &str, name: &str, arity: u16, is_variadic: bool) -> Editor {
    use crate::editor::registry::{TypedBody, TypedCommand};
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    {
        let mut init_host = init_host!(ed);
        host.eval_source(src, &mut init_host).unwrap();
    }
    // Override arity/is_variadic so typed dispatch uses the test-supplied values.
    ed.state.config.registry.register_typed(TypedCommand {
        name: name.to_owned().into(),
        doc: std::borrow::Cow::Borrowed(""),
        aliases: &[],
        body: TypedBody::Steel {
            arity,
            is_variadic,
            inline_output: false,
        },
        completer: None,
    });
    ed.scripting = Some(host);
    ed
}

/// arity-1 + typed arg: the arg reaches the lambda as `StringV`, queued as a
/// command name via `call!`, which runs `move-right`.
/// Oracle: state changes → cursor moved → arg was forwarded.
/// Verification: changing "move-right" in the assert to something else → fails.
#[test]
fn typed_arity_rule_forwards_string_arg_to_arity_1() {
    let mut ed = setup_typed_arity_test(
        r#"(define-typed-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd move-right<Enter>` — arity-1 rule passes "move-right" as StringV.
    ed.handle_key(key(':'));
    for ch in "echo-cmd move-right".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_ne!(
        state(&ed),
        before,
        "arity-1 rule must forward arg as StringV; cursor must have moved"
    );
}

/// arity-1 + no arg: the rule passes `#f` (Scheme's spelling of "no argument
/// typed"), not a sentinel string or a fabricated count. A string-type lambda
/// that checks `(string? x)` gets a boolean, fails the check, and does
/// nothing — cursor stays put.
#[test]
fn typed_arity_rule_passes_false_when_no_arg() {
    let mut ed = setup_typed_arity_test(
        r#"(define-typed-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd<Enter>` — no arg → arity-1 rule passes #f; string guard rejects it.
    ed.handle_key(key(':'));
    for ch in "echo-cmd".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(
        state(&ed),
        before,
        "arity-1 with no arg must not crash or move cursor"
    );
}

/// arity-2 (`arg force`, the most a typed command can receive): both values
/// reach the lambda — the arg as `StringV`, `!` as `#t`.
#[test]
fn typed_arity_rule_forwards_arg_and_force_to_arity_2() {
    let mut ed = setup_typed_arity_test(
        r#"(define-typed-command! "echo-cmd" ""
             (lambda (x force) (when (and (string? x) force) (call! x))))"#,
        "echo-cmd",
        2,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd! move-right<Enter>` — force=#t only when `!` is appended.
    ed.handle_key(key(':'));
    for ch in "echo-cmd! move-right".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_ne!(
        state(&ed),
        before,
        "arity-2 rule must forward both arg and force; cursor must have moved"
    );
}

/// arity-3 (more than a typed command can supply): the rule reports an error
/// and never invokes the command. Cursor stays; error is logged. The command
/// needs no real lambda — the early return fires before call_steel_cmd.
#[test]
fn typed_arity_rule_errors_on_arity_3() {
    use crate::editor::registry::{TypedBody, TypedCommand};

    let mut ed = editor_from("-[a]>b\n");
    ed.state.config.registry.register_typed(TypedCommand {
        name: "needs-three".to_owned().into(),
        doc: std::borrow::Cow::Borrowed(""),
        aliases: &[],
        body: TypedBody::Steel {
            arity: 3,
            is_variadic: false,
            inline_output: false,
        },
        completer: None,
    });

    let before = state(&ed);
    // `:needs-three<Enter>` — arity-3 command, typed dispatch supplies at most 2.
    ed.handle_key(key(':'));
    for ch in "needs-three".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(
        state(&ed),
        before,
        "arity rule must not dispatch the command"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("supplies at most 2")),
        "arity rule must log a user-facing error"
    );
}

// ── Extend-trie WaitChar sequence cleanup ─────────────────────────────────────

/// A multi-key wait-char sequence bound in sticky-Extend mode must clear
/// `pending_keys` (and `pending_ctrl_extend`) once the sequence resolves —
/// mirroring what the normal-trie `WaitChar` arm already does. Before the fix,
/// the Extend-trie arm left the prefix key (`g`) sitting in `pending_keys`,
/// so the very next ordinary keystroke walked the trie as `[g, <key>]`
/// instead of `[<key>]` alone — silently swallowing it.
#[test]
fn extend_trie_wait_char_sequence_clears_pending_keys() {
    use crate::editor::keymap::{BindMode, WaitCharPending};

    let mut ed = editor_from("-[h]>ello\n");
    ed.state.input.set_extend(true);

    // Two-key wait-char sequence: `g` (prefix) then `r` (wait-char leaf).
    ed.state.config.keymap.extend.bind_wait_char_sequence(
        &[key('g'), key('r')],
        WaitCharPending {
            cmd_name: "find-forward".into(),
            ctrl_extend: false,
        },
    );
    // A plain leaf on `x`, distinct from the `g`-prefixed sequence, so a
    // leftover `g` prefix would make this unreachable (NoMatch) instead of
    // executing it.
    ed.state.config.keymap.bind_user_with_extend(
        BindMode::Extend,
        &[key('x')],
        "delete-char-forward".into(),
        false,
    );

    ed.handle_key(key('g'));
    assert_eq!(
        ed.state.pending_keys,
        vec![key('g')],
        "sanity: 'g' commits as an interior prefix key"
    );

    ed.handle_key(key('r'));
    assert!(
        ed.state.pending_keys.is_empty(),
        "the completed wait-char sequence must clear pending_keys"
    );
    assert!(
        ed.state.wait_char.is_some(),
        "sanity: the sequence armed wait_char"
    );

    // Consume the wait-char argument (any key) — dispatches "find-forward".
    ed.handle_key(key('z'));
    assert!(ed.state.wait_char.is_none());

    // With pending_keys correctly cleared, this reaches the `x` leaf and
    // deletes the char under the cursor.
    ed.handle_key(key('x'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ello\n",
        "'x' must delete the char under the cursor, not get swallowed by a stale 'g' prefix"
    );
}

// ── G L / G U / G C case-transform keypath ───────────────────────────────────

#[test]
fn g_shift_l_lowercases_selection() {
    let mut ed = editor_from("-[HELLO]> world\n");
    ed.feed_keys([key('G'), key('L')]);
    assert_eq!(state(&ed), "-[hello]> world\n");
}

#[test]
fn g_shift_u_uppercases_selection() {
    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_keys([key('G'), key('U')]);
    assert_eq!(state(&ed), "-[HELLO]> world\n");
}

#[test]
fn g_shift_c_capitalizes_words_in_selection() {
    let mut ed = editor_from("-[hELLO wORLD]>\n");
    ed.feed_keys([key('G'), key('C')]);
    assert_eq!(state(&ed), "-[Hello World]>\n");
}

#[test]
fn g_shift_l_dot_repeats() {
    // Confirms the full keymap-dispatch path (not just the pure op) stamps
    // make-text-lowercase as repeatable.
    let mut ed = editor_from("-[HELLO]>\nWORLD\n");
    ed.feed_keys([key('G'), key('L')]); // "hello"
    ed.feed_key(key('j')); // move to line 2
    ed.feed_key(key('x')); // select the whole line ("WORLD\n")
    ed.feed_key(key('.')); // replay make-text-lowercase
    assert_eq!(state(&ed), "hello\n-[world\n]>");
}

// ── `>` / `<` indent / unindent ─────────────────────────────────────────────

#[test]
fn angle_bracket_indents_and_unindents() {
    // Default settings: tab-style=hard, tab-width=4.
    let mut ed = editor_from("-[f]>oo\n");
    ed.handle_key(key('>'));
    assert_eq!(state(&ed), "\t-[f]>oo\n");
    ed.handle_key(key('<'));
    assert_eq!(state(&ed), "-[f]>oo\n");
}

/// A count reaches the pure op (proving the keymap→`EditorCmd` count plumbing
/// works for `indent`, same as `cmd_align_selections`), and the whole `3>`
/// composes into a single undo step rather than three.
#[test]
fn three_indent_is_one_undo_step() {
    let mut ed = editor_from("-[f]>oo\n");
    ed.feed_keys([key('3'), key('>')]);
    assert_eq!(state(&ed), "\t\t\t-[f]>oo\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[f]>oo\n");
    assert!(!ed.doc().can_undo());
}
