// Editor-level tests for tree-sitter structural text objects (`m i f`, `m a
// c`, …) and structural navigation (`goto-next-<kind>`, `goto-prev-<kind>`) —
// the `SelectionBody::Structural` wiring in `commands/structural.rs`,
// `registry/defaults/structural.rs`, and `keymap/defaults.rs`.
//
// Requires the `rust` grammar fixture plus its Helix-maintained
// `textobjects.scm`: run scripts/fetch-test-grammars.sh.

use super::*;

use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::Selection;
use hume_test_fixtures::{
    grammar_query_path, helix_textobjects_path, helix_textobjects_path_unchecked,
    require_fixture_file, require_grammars,
};
use hume_treesitter::registry::QueryPaths;

use crate::editor::dispatch::ArgSource;

/// Require this file's grammar fixture — see
/// `hume_test_fixtures::require_grammars`/`require_fixture_file`.
fn require_fixtures() {
    require_grammars(&["rust"]);
    require_fixture_file(
        &helix_textobjects_path_unchecked("rust"),
        "rust helix-textobjects.scm",
    );
}

/// Build an editor from the marker DSL with the `rust` grammar (plus its
/// real Helix-maintained `textobjects.scm` — what `:plum-install-grammar`
/// actually fetches) attached, but the initial parse *not yet* drained —
/// the window `ensure_syntax_current`'s no-committed-tree gate covers.
fn rust_editor_undrained(source: &str) -> Editor {
    require_fixtures();
    let to_path = helix_textobjects_path("rust")
        .expect("rust helix-textobjects.scm fixture — run scripts/fetch-test-grammars.sh");
    let mut ed = editor_from(source);
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar_with(
        &mut ed,
        "rust",
        "tree_sitter_rust",
        QueryPaths {
            highlights: &grammar_query_path("rust"),
            injections: None,
            textobjects: Some(&to_path),
        },
    );
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed
}

/// Build an editor from the marker DSL with the `rust` grammar (plus its
/// real Helix-maintained `textobjects.scm` — what `:plum-install-grammar`
/// actually fetches) attached and the initial parse drained.
fn rust_editor(source: &str) -> Editor {
    let mut ed = rust_editor_undrained(source);
    ed.reparse_stale_buffers(); // drains the initial full parse
    ed
}

/// The text a selection covers (inclusive of the grapheme at `end()`).
fn text_of(text: &BufferText, sel: Selection) -> String {
    let end = next_grapheme_boundary(text, sel.end_inclusive(text));
    text.slice(sel.start()..end).to_string()
}

/// The text covered by the focused buffer's primary selection. A slice
/// rather than the marker-annotated `state()` string: Rust source is
/// multi-line, and hand-computing exact marker offsets across a whole
/// function body is more error-prone than checking the selected substring
/// itself.
fn selected_text(ed: &Editor) -> String {
    text_of(ed.doc().text(), ed.current_selections().primary())
}

/// The text covered by every selection, sorted by position — for
/// multi-cursor assertions.
fn selection_texts(ed: &Editor) -> Vec<String> {
    let text = ed.doc().text();
    ed.current_selections()
        .iter_sorted()
        .map(|&sel| text_of(text, sel))
        .collect()
}

// ── Select: `m i f` / `m a f` ────────────────────────────────────────────────

#[test]
fn inner_function_selects_the_body_inside_a_function() {
    let mut ed = rust_editor("fn target() {\n    -[l]>et y = 2;\n}\n");
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "{\n    let y = 2;\n}");
}

#[test]
fn inner_function_outside_any_function_is_unchanged() {
    let mut ed = rust_editor("-[/]>/ nothing here\nfn f() {}\n");
    let before = state(&ed);
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(before, state(&ed));
}

/// Cursor inside a closure nested in a function: the closure (the smallest
/// enclosing `function`-kind object) wins over the outer function.
#[test]
fn inner_function_inside_a_nested_closure_selects_the_closure() {
    let mut ed = rust_editor("fn outer_fn() {\n    let c = || {\n        -[4]>2;\n    };\n}\n");
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "{\n        42;\n    }");
}

/// `function.around`'s hull includes any immediately preceding
/// `attribute_item`s (`textobjects.rs`'s grouped-capture hull).
#[test]
fn around_function_on_an_attributed_function_includes_the_attribute() {
    let mut ed = rust_editor("#[inline]\nfn attributed() {\n    -[1]>;\n}\n");
    for ch in "maf".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        selected_text(&ed),
        "#[inline]\nfn attributed() {\n    1;\n}"
    );
}

// ── Extend ───────────────────────────────────────────────────────────────────

/// `e` (toggle sticky Extend) then `m i f` unions the current selection with
/// the smallest enclosing function; a second `m i f` — selection already
/// equal to that object — grows outward to the next enclosing one
/// (`apply_text_object_extend`'s past-end retry).
#[test]
fn inner_function_extend_unions_then_second_press_grows_outward() {
    let mut ed = rust_editor("fn outer_extend() {\n    let c = || {\n        -[4]>2;\n    };\n}\n");
    ed.handle_key(key('e')); // toggle sticky extend
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "{\n        42;\n    }");

    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        selected_text(&ed),
        "{\n    let c = || {\n        42;\n    };\n}"
    );
}

// ── Navigation: `goto-next-function` / `goto-prev-function` /
//    `goto-next-argument` ─────────────────────────────────────────────────────

const NAV_SRC: &str =
    "-[/]>/ c\nfn alpha() {\n    let c = || {\n        1;\n    };\n}\n\nfn beta() {\n    2;\n}\n";

#[test]
fn goto_next_function_selects_the_whole_next_function_head_at_start() {
    let mut ed = rust_editor(NAV_SRC);
    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    assert_eq!(
        selected_text(&ed),
        "fn alpha() {\n    let c = || {\n        1;\n    };\n}"
    );
    // Head at the object's start (first char of "fn").
    let sel = ed.current_selections().primary();
    assert_eq!(sel.head(), sel.start());
}

/// A second press, searching forward from the current selection's end,
/// cannot re-select the closure nested inside it — the object just
/// selected is skipped, as Helix does.
#[test]
fn goto_next_function_a_second_press_skips_a_nested_closure() {
    let mut ed = rust_editor(NAV_SRC);
    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "fn beta() {\n    2;\n}");
}

#[test]
fn goto_prev_function_from_inside_a_function_selects_that_function() {
    let mut ed = rust_editor("fn alpha() {\n    1;\n}\n\nfn beta() {\n    -[2]>;\n}\n");
    ed.execute_keymap_command("goto-prev-function".into(), None, false, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "fn beta() {\n    2;\n}");
}

/// `count` 2 backward: the first step lands on the enclosing function (the
/// cursor's own, per the start-keyed backward rule above), the second step
/// re-searches from *that* function's start and so does not re-select it.
#[test]
fn goto_prev_function_with_count_two_advances_two_objects_backward() {
    let mut ed = rust_editor(
        "fn first() {\n    1;\n}\n\nfn second() {\n    2;\n}\n\nfn third() {\n    -[3]>;\n}\n",
    );
    ed.execute_keymap_command(
        "goto-prev-function".into(),
        Some(2),
        false,
        ArgSource::Keymap,
    );
    assert_eq!(selected_text(&ed), "fn second() {\n    2;\n}");
}

/// Backward search from inside a function lands on that function's own
/// start (Helix's start-keyed backward rule), so the union with the current
/// (collapsed, already-inside) selection is the whole function — the same
/// "target is the anchor's own unit" case `apply_word_select_extend`
/// selects wholly rather than partially.
#[test]
fn goto_prev_function_extend_from_inside_selects_the_whole_enclosing_function() {
    let mut ed = rust_editor("fn alpha() {\n    1;\n}\n\nfn beta() {\n    -[2]>;\n}\n");
    ed.execute_keymap_command("goto-prev-function".into(), None, true, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "fn beta() {\n    2;\n}");
}

/// Extending after a Move-mode result keeps the just-selected function fully
/// covered while growing onto the next one — the union with the previous
/// selection, not a plain replacement, since a Move result's own anchor sits
/// at the object's far edge (see `apply_object_motion`'s doc in `hume-ops`).
/// No nested closure here (unlike `NAV_SRC`): that case only ever absorbs
/// silently into the current selection on one press, covered instead by
/// `hume-ops`'s own `extend_forward_into_a_nested_object_does_not_shrink_the_selection`.
#[test]
fn goto_next_function_extend_after_move_keeps_both_functions_selected() {
    let mut ed = rust_editor("-[/]>/ c\nfn first() {\n    1;\n}\n\nfn second() {\n    2;\n}\n");
    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    ed.execute_keymap_command("goto-next-function".into(), None, true, ArgSource::Keymap);
    assert_eq!(
        selected_text(&ed),
        "fn first() {\n    1;\n}\n\nfn second() {\n    2;\n}"
    );
}

/// `count` 2 from before the first function lands on the second, not the
/// third — each count step re-searches from the previous step's own end.
#[test]
fn goto_next_function_with_count_two_advances_two_objects() {
    let mut ed = rust_editor(
        "-[/]>/ c\nfn first() {\n    1;\n}\n\nfn second() {\n    2;\n}\n\nfn third() {\n    3;\n}\n",
    );
    ed.execute_keymap_command(
        "goto-next-function".into(),
        Some(2),
        false,
        ArgSource::Keymap,
    );
    assert_eq!(selected_text(&ed), "fn second() {\n    2;\n}");
}

/// No function follows the cursor: the selection is left unchanged.
#[test]
fn goto_next_function_at_the_last_function_is_unchanged() {
    let mut ed = rust_editor("fn only() {\n    -[3]>;\n}\n");
    let before = state(&ed);
    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    assert_eq!(before, state(&ed));
}

#[test]
fn goto_next_function_records_a_jump_list_entry() {
    let mut ed = rust_editor(NAV_SRC);
    let before = state(&ed);
    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    assert_ne!(before, state(&ed));
    ed.handle_key(key_ctrl('o')); // jump-backward
    assert_eq!(before, state(&ed));
}

/// Extend mode keeps the anchor pinned to the selection's original anchor —
/// only the head moves to the found object's edge.
#[test]
fn goto_next_function_extend_keeps_the_anchor() {
    let mut ed = rust_editor(NAV_SRC);
    let anchor_before = ed.current_selections().primary().anchor();
    ed.execute_keymap_command("goto-next-function".into(), None, true, ArgSource::Keymap);
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor(), anchor_before);
    assert_ne!(sel.head(), anchor_before);
}

/// `goto-next-argument` navigates `parameter.inside` (not `.around`, whose
/// hull includes the trailing comma) — the trimmed span, no comma.
#[test]
fn goto_next_argument_selects_the_trimmed_argument_with_no_comma() {
    let mut ed = rust_editor("fn call_site() {\n    foo(-[a]>aa, bbb, ccc);\n}\n");
    ed.execute_keymap_command("goto-next-argument".into(), None, false, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "bbb");
}

// ── Multi-cursor ─────────────────────────────────────────────────────────────

#[test]
fn inner_function_two_cursors_in_two_functions_selects_both() {
    let mut ed = rust_editor("fn one() {\n    -[1]>;\n}\n\nfn two() {\n    -[2]>;\n}\n");
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(ed.current_selections().len(), 2);
    assert_eq!(
        selection_texts(&ed),
        vec!["{\n    1;\n}".to_string(), "{\n    2;\n}".to_string()]
    );
}

#[test]
fn inner_function_two_cursors_in_one_function_merge() {
    let mut ed = rust_editor("fn merge_target() {\n    let -[a]> = 1;\n    let -[b]> = 2;\n}\n");
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(ed.current_selections().len(), 1);
    assert_eq!(selected_text(&ed), "{\n    let a = 1;\n    let b = 2;\n}");
}

// ── No grammar ───────────────────────────────────────────────────────────────

#[test]
fn structural_commands_are_no_ops_without_a_grammar() {
    let mut ed = editor_from("fn f() {\n    -[1]>;\n}\n");
    let before = state(&ed);

    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(before, state(&ed), "m i f without a grammar");

    for ch in "maf".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(before, state(&ed), "m a f without a grammar");

    ed.execute_keymap_command("goto-next-function".into(), None, false, ArgSource::Keymap);
    assert_eq!(before, state(&ed), "goto-next-function without a grammar");
}

/// Before the background worker's first parse lands, `ensure_syntax_current`
/// no-ops rather than blocking the UI thread on a full synchronous parse of
/// the whole buffer — the command reads as the same "no grammar" no-op until
/// the next `reparse_stale_buffers` installs the worker's result.
#[test]
fn structural_command_before_the_first_parse_lands_is_a_no_op() {
    let mut ed = rust_editor_undrained("fn target() {\n    -[l]>et y = 2;\n}\n");
    let before = state(&ed);
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        before,
        state(&ed),
        "m i f before the initial parse is drained"
    );

    // Draining now (as the next frame would) makes the same keys work.
    ed.reparse_stale_buffers();
    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "{\n    let y = 2;\n}");
}

/// A parse that *failed* for the current generation advances `parsed_gen`
/// without advancing `tree_gen` or replacing `layers`. `ensure_syntax_current`
/// must still reparse in that state: the committed layers describe the
/// pre-edit text, so reading them would resolve spans against byte offsets
/// the buffer no longer has.
///
/// Flip: gate `ensure_syntax_current` on `parsed_gen() == Some(text_gen)`
/// instead of `Syntax::is_current` and it returns early here, leaving `m i f`
/// to collect from the stale gen-0 tree — which selects the pre-edit body
/// text (or trips `collect_hulls`'s `end_byte <= len_bytes` debug assert).
#[test]
fn structural_command_after_a_failed_parse_reparses_instead_of_reading_stale_layers() {
    use hume_treesitter::parse_worker::{ParseDone, ParseOutcome};

    let mut ed = rust_editor("fn target() {\n    -[l]>et y = 2;\n}\n");
    let bid = ed.focused_buffer_id();

    // `x d` deletes the `let` line, so the committed gen-0 layers no longer
    // describe the buffer. `feed_key` bypasses `settle`, so nothing drains.
    ed.feed_key(key('x'));
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "fn target() {\n}\n");

    // That generation's parse fails: parsed_gen advances, layers do not.
    let text_gen = ed.state.buffers.get(bid).text_gen;
    let bundle = {
        let syn = ed.state.buffers.get(bid).syntax.as_ref().expect("syntax");
        std::sync::Arc::clone(syn.bundle())
    };
    ed.state
        .buffers
        .get_mut(bid)
        .syntax
        .as_mut()
        .expect("syntax")
        .install(
            ParseDone {
                bid,
                text_gen,
                bundle,
                outcome: ParseOutcome::ParseFailed,
            },
            text_gen,
        );
    let syn = ed.state.buffers.get(bid).syntax.as_ref().expect("syntax");
    assert_eq!(syn.parsed_gen(), Some(text_gen));
    assert!(
        !syn.is_current(text_gen),
        "layers must still predate the edit — otherwise this test proves nothing"
    );

    for ch in "mif".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        selected_text(&ed),
        "{\n}",
        "m i f must reflect the post-edit tree, not the stale gen-0 layers"
    );
}

// ── Dot-repeat ───────────────────────────────────────────────────────────────

/// `m a f` `d` deletes the function under the cursor; `.` replays both steps
/// from a fresh cursor, deleting whatever function is under it now.
#[test]
fn dot_repeat_of_around_function_deletes_the_function_under_the_new_cursor() {
    let mut ed = rust_editor("fn del_one() {\n    -[1]>;\n}\n\nfn del_two() {\n    2;\n}\n");
    for ch in "maf".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key('d'));
    let after_first = ed.doc().text().to_string();
    assert_eq!(after_first, "\n\nfn del_two() {\n    2;\n}\n");

    // Move the cursor into del_two's body, then replay `m a f` + `d`.
    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    let pos = after_first.find('2').expect("del_two's body");
    ed.state.panes.state[pid][bid].selections = SelectionSet::single(Selection::collapsed(pos));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "\n\n\n");
}

// ── Macro-replay freshness ───────────────────────────────────────────────────

/// A macro that deletes a line inside a function (`x d`) and then selects
/// and deletes that function's now-current body (`m i f d`), replayed in one
/// `drain_replay_queue()` batch, must see the post-edit tree for its second
/// step — not the pre-edit byte ranges the first step invalidated.
///
/// Fail oracle: if `SelectionBody::Structural`'s dispatch arm omitted its
/// `ensure_syntax_current` call, the `m i f` step would compute its span
/// from the stale pre-`x d` tree, either mismatching `direct`'s result or
/// tripping the `end_byte <= text.len_bytes()` debug_assert in
/// `hume-treesitter`'s hull collector.
#[test]
fn macro_replay_reparses_before_each_structural_step() {
    const SRC: &str = "fn macro_fn() {\n    -[l]>et a = 1;\n    let b = 2;\n}\n";
    let macro_keys: Vec<_> = "xdmifd".chars().map(key).collect();

    let mut direct = rust_editor(SRC);
    for k in macro_keys.clone() {
        direct.feed_event(k);
    }

    let mut recorded = rust_editor(SRC);
    recorded.state.registers.write_macro('q', macro_keys);
    recorded.handle_key(key('q'));
    recorded.handle_key(key('q'));
    recorded.drain_replay_queue();

    assert_eq!(state(&recorded), state(&direct));
    assert_eq!(direct.doc().text().to_string(), "fn macro_fn() \n");
}

// ── Unified argument (`m i a` / `m a a`) ─────────────────────────────────────

/// A sibling argument's `,` inside a string literal must not fool the
/// separator scan — the tree span for the whole string-literal argument
/// wins outright.
#[test]
fn around_argument_ignores_a_comma_inside_a_sibling_string_literal() {
    let mut ed = rust_editor("fn call_site() {\n    foo(\"-[a]>,b\", second);\n}\n");
    for ch in "maa".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "\"a,b\", ");
}

/// `m a a` on the last parameter eats the preceding `, ` rather than leaving
/// a dangling separator.
#[test]
fn around_argument_on_the_last_parameter_eats_the_preceding_separator() {
    let mut ed = rust_editor("fn call_site2() {\n    bar(first, -[s]>econd);\n}\n");
    for ch in "maa".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), ", second");
}

/// A top-level array literal has no `parameter` capture (its elements are
/// `entry` objects) — `m i a` falls back to the lexical scan.
#[test]
fn inner_argument_in_a_top_level_array_falls_back_to_the_lexical_scan() {
    let mut ed = rust_editor("fn holds_array() {\n    let arr = [-[1]>, 2, 3];\n}\n");
    for ch in "mia".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "1");
}

/// An array literal that IS a call argument is itself the `parameter` (its
/// elements are not) — the tree span wins over the lexical scan, selecting
/// the whole array rather than the element under the cursor.
#[test]
fn inner_argument_on_an_array_literal_argument_selects_the_whole_array() {
    let mut ed = rust_editor("fn call_with_array() {\n    foo([1, -[2]>, 3]);\n}\n");
    for ch in "mia".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "[1, 2, 3]");
}

/// The lexical fallback still works with no grammar attached at all.
#[test]
fn inner_argument_in_a_scratch_buffer_with_no_grammar_still_works() {
    let mut ed = editor_from("foo(-[a]>aa, bbb);\n");
    for ch in "mia".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "foo(-[aaa]>, bbb);\n");
}

/// A grammar attached but with no `textobjects.scm` at all (`QueryPaths`
/// leaves `textobjects: None`) — `object_spans` finds nothing, so `m i a`
/// falls back to the lexical scan exactly as the no-grammar case does, and
/// `ensure_syntax_current`'s no-textobjects-anywhere gate must not break
/// that fallback along the way.
#[test]
fn inner_argument_with_a_grammar_but_no_textobjects_query_falls_back_to_the_lexical_scan() {
    require_fixtures();
    let mut ed = editor_from("fn f() {\n    foo(-[a]>aa, bbb);\n}\n");
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar(&mut ed, "rust", "tree_sitter_rust");
    let lang = ed.state.config.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();

    for ch in "mia".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "aaa");
}

// ── Other kinds: class / comment / unit test / value ────────────────────────
//
// Only `Function` and `Parameter` get exercised above; these cover the
// remaining four `STRUCTURAL_OBJECTS` rows through the real Helix rust
// `textobjects.scm`, one representative select + navigate case per kind.

#[test]
fn inner_class_selects_the_body_inside_a_struct() {
    let mut ed = rust_editor("struct Widget {\n    -[n]>ame: String,\n}\n");
    for ch in "mit".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "{\n    name: String,\n}");
}

#[test]
fn around_class_on_a_struct_includes_the_declaration() {
    let mut ed = rust_editor("struct Widget {\n    -[n]>ame: String,\n}\n");
    for ch in "mat".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "struct Widget {\n    name: String,\n}");
}

#[test]
fn goto_next_class_selects_the_whole_next_struct() {
    let mut ed =
        rust_editor("-[/]>/ c\nstruct Alpha {\n    a: i32,\n}\n\nstruct Beta {\n    b: i32,\n}\n");
    ed.execute_keymap_command("goto-next-class".into(), None, false, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "struct Alpha {\n    a: i32,\n}");
    let sel = ed.current_selections().primary();
    assert_eq!(sel.head(), sel.start());
}

/// `comment.inside` captures a single `line_comment` node with no grouping
/// quantifier (unlike `comment.around`'s `(line_comment)+`) — `m i c` selects
/// only the line under the cursor, never the whole contiguous block.
#[test]
fn inner_comment_selects_only_the_line_at_the_cursor() {
    let mut ed = rust_editor("// line -[o]>ne\n// line two\nstruct S {}\n");
    for ch in "mic".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "// line one");
}

#[test]
fn around_comment_selects_the_whole_contiguous_block() {
    let mut ed = rust_editor("// line -[o]>ne\n// line two\nstruct S {}\n");
    for ch in "mac".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "// line one\n// line two");
}

#[test]
fn goto_next_comment_selects_the_whole_block() {
    let mut ed = rust_editor("-[s]>truct S {}\n\n// line one\n// line two\n");
    ed.execute_keymap_command("goto-next-comment".into(), None, false, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "// line one\n// line two");
}

#[test]
fn inner_unit_test_selects_the_functions_body() {
    let mut ed = rust_editor("#[test]\nfn it_works() {\n    -[a]>ssert!(true);\n}\n");
    for ch in "miu".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "{\n    assert!(true);\n}");
}

#[test]
fn around_unit_test_includes_the_test_attribute() {
    let mut ed = rust_editor("#[test]\nfn it_works() {\n    -[a]>ssert!(true);\n}\n");
    for ch in "mau".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        selected_text(&ed),
        "#[test]\nfn it_works() {\n    assert!(true);\n}"
    );
}

/// `test.around`'s `(#eq? @_test_attribute "test")` predicate must actually
/// filter — a plain function ahead of the real `#[test]` one is skipped.
#[test]
fn goto_next_unit_test_skips_a_plain_function_and_lands_on_the_test() {
    let mut ed = rust_editor(
        "-[/]>/ c\nfn plain() {\n    1;\n}\n\n#[test]\nfn it_works() {\n    assert!(true);\n}\n",
    );
    ed.execute_keymap_command("goto-next-test".into(), None, false, ArgSource::Keymap);
    assert_eq!(
        selected_text(&ed),
        "#[test]\nfn it_works() {\n    assert!(true);\n}"
    );
}

/// `entry.inside`/`entry.around` on a struct's `field_declaration` — a
/// different pattern from the array/tuple case below (one match per named
/// child, each its own inside span; the same enclosing `field_declaration`
/// as the around span). Command names/key say "value" — see
/// `STRUCTURAL_OBJECTS`'s entry row for why the capture stays `entry`.
#[test]
fn inner_value_selects_a_struct_fields_name() {
    let mut ed = rust_editor("struct Point {\n    -[x]>: i32,\n}\n");
    for ch in "miv".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "x");
}

#[test]
fn around_value_selects_the_whole_field_declaration() {
    let mut ed = rust_editor("struct Point {\n    -[x]>: i32,\n}\n");
    for ch in "mav".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "x: i32");
}

/// `(array_expression (_) @entry.around)` captures each element on its own —
/// no grouping, no separator, unlike the `parameter` family.
#[test]
fn around_value_on_an_array_element_selects_just_that_element() {
    let mut ed = rust_editor("fn make_arr() {\n    let arr = [10, -[2]>0, 30];\n}\n");
    for ch in "mav".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), "20");
}

#[test]
fn goto_next_value_walks_array_elements() {
    let mut ed = rust_editor("fn make_arr2() {\n    let arr = [-[1]>0, 20, 30];\n}\n");
    ed.execute_keymap_command("goto-next-value".into(), None, false, ArgSource::Keymap);
    assert_eq!(selected_text(&ed), "20");
}

/// Reproduces the `gu`/`gU` bug: with two `#[test]` fns in the buffer, the
/// underlying tree-sitter query for `test.around` also reports a spurious
/// match spanning from the first test's `#[test]` attribute through the
/// *last* test's closing brace (see
/// `test_around_two_sequential_tests_has_no_spurious_merged_span` in
/// `hume-treesitter`). `goto-next-test` from before the first test lands on
/// that merged span — selecting every test at once — instead of just the
/// first one. This is the mechanism behind `gu` extending toward the end of
/// a file like `hume-editing/src/lines/tests.rs`, which has many
/// `#[test]` fns.
#[test]
fn goto_next_test_with_two_tests_lands_on_the_first_test_only() {
    let mut ed = rust_editor(
        "-[u]>se super::*;\n\n#[test]\nfn one() {\n    assert!(true);\n}\n\n#[test]\nfn two() {\n    assert!(true);\n}\n",
    );
    ed.execute_keymap_command("goto-next-test".into(), None, false, ArgSource::Keymap);
    assert_eq!(
        selected_text(&ed),
        "#[test]\nfn one() {\n    assert!(true);\n}"
    );
}

/// `gU`'s half of the same bug: `adjacent`'s backward tie-break also picks
/// the spurious merged span when the cursor sits in the gap right after the
/// first test. From the blank line between the two tests, `goto-prev-test`
/// must land on the first test alone.
#[test]
fn goto_prev_test_from_the_gap_between_two_tests_lands_on_the_first_test_only() {
    let mut ed = rust_editor(
        "#[test]\nfn one() {\n    assert!(true);\n}\n-[\n]>#[test]\nfn two() {\n    assert!(true);\n}\n",
    );
    ed.execute_keymap_command("goto-prev-test".into(), None, false, ArgSource::Keymap);
    assert_eq!(
        selected_text(&ed),
        "#[test]\nfn one() {\n    assert!(true);\n}"
    );
}

/// `enclosing`'s half of the same bug: from the gap between two tests,
/// `around-test` (`m a u`) must find no enclosing test at all, not the
/// spurious merged span that (wrongly) contains the gap position.
#[test]
fn around_test_from_the_gap_between_two_tests_is_a_no_op() {
    let mut ed = rust_editor(
        "#[test]\nfn one() {\n    assert!(true);\n}\n-[\n]>#[test]\nfn two() {\n    assert!(true);\n}\n",
    );
    let before = selected_text(&ed);
    for ch in "mau".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(selected_text(&ed), before);
}
