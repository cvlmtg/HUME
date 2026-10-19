// Editor-level tests for tree-sitter smart indent on Enter, `o` and `O`
// (`commands/smart_indent.rs`).
//
// Requires the rust, json, markdown and markdown.inline grammar fixtures, the
// rust `indents.scm` and the markdown `injections.scm` fixtures: run
// scripts/fetch-test-grammars.sh.

use super::*;
use hume_editing::tab_style::TabStyle;
use hume_scripting::ScriptingHost;
use hume_treesitter::registry::{OptionalQuery, QueryFiles};
use pretty_assertions::assert_eq;
use test_fixtures::{grammar_query_path, require_grammars, require_helix_query};

fn require_fixtures() {
    require_grammars(&["rust", "json", "markdown", "markdown.inline"]);
    require_helix_query("rust", OptionalQuery::Indents.file_name());
}

fn rust_indent_queries() -> QueryFiles {
    let indents = require_helix_query("rust", OptionalQuery::Indents.file_name());
    QueryFiles::new(grammar_query_path("rust")).with(OptionalQuery::Indents, indents)
}

/// An editor over the marker-DSL `source` with the rust grammar and the
/// fixture rust `indents.scm` attached and the initial parse drained.
fn rust_indent_editor(source: &str) -> Editor {
    require_fixtures();
    indent_editor("rust", &rust_indent_queries(), source)
}

/// An editor over the marker-DSL `source` with grammar `name` attached with
/// `queries`, as the buffer's language, and the initial parse drained.
fn indent_editor(name: &str, queries: &QueryFiles, source: &str) -> Editor {
    let mut ed = editor_from(source);
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar_with(&mut ed, name, &format!("tree_sitter_{name}"), queries);
    super::language::attach_host(&mut ed, "");
    let lang = ed.state.config.languages.intern(name);
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    ed
}

/// Runs `check` once per tab style with that style's one-level indent
/// string, set as the global `indent-style`.
fn each_style(check: impl Fn(TabStyle, &str)) {
    check(TabStyle::Soft, "    ");
    check(TabStyle::Hard, "\t");
}

/// `template` with each `<T>` replaced by `unit`.
fn indented(template: &str, unit: &str) -> String {
    template.replace("<T>", unit)
}

/// Starts Insert at the cursor, presses Enter, and returns the editor.
fn enter_at(source: &str, style: TabStyle, unit: &str) -> Editor {
    let mut ed = rust_indent_editor(&indented(source, unit));
    ed.state.settings.indent_style = style;
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed
}

#[test]
fn enter_between_braces_opens_a_block() {
    each_style(|style, unit| {
        let ed = enter_at("fn f() {-[}]>\n", style, unit);
        assert_eq!(state(&ed), indented("fn f() {\n<T>-[\n]>}\n", unit));
    });
}

#[test]
fn enter_inside_a_body_keeps_the_body_level() {
    each_style(|style, unit| {
        let ed = enter_at("fn f() {\n    a();-[\n]>}\n", style, unit);
        assert_eq!(
            state(&ed),
            indented("fn f() {\n    a();\n<T>-[\n]>}\n", unit)
        );
    });
}

#[test]
fn enter_in_a_nested_space_indented_file_renders_tabs_under_hard() {
    let ed = enter_at(
        "impl S {\n    fn f() {\n        a();-[\n]>    }\n}\n",
        TabStyle::Hard,
        "\t",
    );
    assert_eq!(
        state(&ed),
        "impl S {\n    fn f() {\n        a();\n\t\t-[\n]>    }\n}\n"
    );
}

#[test]
fn o_on_an_empty_body_line_indents_one_level() {
    each_style(|style, unit| {
        let mut ed = rust_indent_editor("fn f() {\n-[\n]>}\n");
        ed.state.settings.indent_style = style;
        ed.feed_key(key('o'));
        assert_eq!(state(&ed), indented("fn f() {\n\n<T>-[\n]>}\n", unit));
    });
}

#[test]
fn big_o_below_the_first_line_indents_from_the_previous_line() {
    each_style(|style, unit| {
        let mut ed = rust_indent_editor("fn f() {\n-[}]>\n");
        ed.state.settings.indent_style = style;
        ed.feed_key(key('O'));
        assert_eq!(state(&ed), indented("fn f() {\n<T>-[\n]>}\n", unit));
    });
}

#[test]
fn big_o_on_the_first_line_copies_its_indent() {
    each_style(|style, unit| {
        let mut ed = rust_indent_editor(&indented("<T>-[f]>oo();\n", unit));
        ed.state.settings.indent_style = style;
        ed.feed_key(key('O'));
        assert_eq!(state(&ed), indented("<T>-[\n]><T>foo();\n", unit));
    });
}

#[test]
fn escape_after_enter_vacates_the_computed_indent() {
    each_style(|style, unit| {
        let mut ed = enter_at("fn f() {\n<T>a();-[\n]>}\n", style, unit);
        ed.feed_key(key_esc());
        assert_eq!(
            ed.doc().text().to_string(),
            "fn f() {\n".to_owned() + &indented("<T>a();\n\n}\n", unit)
        );
    });
}

#[test]
fn a_second_enter_vacates_the_blank_line_and_indents_the_next() {
    each_style(|style, unit| {
        let mut ed = enter_at("fn f() {\n    a();-[\n]>}\n", style, unit);
        ed.feed_key(key_enter());
        assert_eq!(
            state(&ed),
            indented("fn f() {\n    a();\n\n<T>-[\n]>}\n", unit)
        );
    });
}

#[test]
fn enter_at_cursors_on_different_lines_indents_each() {
    each_style(|style, unit| {
        let ed = enter_at(
            "fn f() {\n    a();-{\n}>}\nfn g() {\n    b();-[\n]>}\n",
            style,
            unit,
        );
        assert_eq!(
            state(&ed),
            indented(
                "fn f() {\n    a();\n<T>-{\n}>}\nfn g() {\n    b();\n<T>-[\n]>}\n",
                unit
            )
        );
    });
}

#[test]
fn enter_at_cursors_on_one_line_indents_each() {
    each_style(|style, unit| {
        let ed = enter_at("fn f() {\n    a(); -{b}>(); -[c]>();\n}\n", style, unit);
        assert_eq!(
            state(&ed),
            indented("fn f() {\n    a(); \n<T>-{b}>(); \n<T>-[c]>();\n}\n", unit)
        );
    });
}

#[test]
fn dot_repeat_computes_the_indent_at_the_new_position() {
    each_style(|style, unit| {
        let mut ed = rust_indent_editor(&indented(
            "-[f]>n f() {\n}\nfn g() {\n<T>if x {\n<T>}\n}\n",
            unit,
        ));
        ed.state.settings.indent_style = style;
        ed.feed_keys([key('o'), key('x'), key_esc()]);
        ed.feed_keys([key('j'), key('j'), key('j')]);
        ed.feed_key(key('.'));
        assert_eq!(
            ed.doc().text().to_string(),
            indented(
                "fn f() {\n<T>x\n}\nfn g() {\n<T>if x {\n<T><T>x\n<T>}\n}\n",
                unit
            )
        );
    });
}

#[test]
fn enter_with_typed_text_and_escape_is_one_undo_step() {
    let mut ed = rust_indent_editor("fn f() {\n    a();-[\n]>}\n");
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n    a();\n\tx\n}\n");
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n    a();\n}\n");
}

// ── Opting out ───────────────────────────────────────────────────────────────

const OPEN_BLOCK: &str = "fn f() {-[}]>\n";

/// A block opened on an indented line: on, Enter opens it one level in
/// from that line's indent; copying would keep the indent.
const NESTED_OPEN_BLOCK: &str = "fn f() {\n    if x {-[}]>\n}\n";

/// [`NESTED_OPEN_BLOCK`] after an Enter with automatic indentation off.
const NESTED_UNINDENTED_BREAK: &str = "fn f() {\n    if x {\n-[}]>\n}\n";

#[test]
fn set_global_turns_smart_indent_off() {
    let mut ed = rust_indent_editor(NESTED_OPEN_BLOCK);
    type_cmd(&mut ed, ":set global indent-smart=false");
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), NESTED_UNINDENTED_BREAK);
}

#[test]
fn set_buffer_turns_smart_indent_off() {
    let mut ed = rust_indent_editor(NESTED_OPEN_BLOCK);
    type_cmd(&mut ed, ":set buffer indent-smart=false");
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), NESTED_UNINDENTED_BREAK);
}

#[test]
fn a_buffer_override_turns_smart_indent_off() {
    let mut ed = rust_indent_editor(NESTED_OPEN_BLOCK);
    ed.doc_mut().overrides.indent_smart = Some(false);
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), NESTED_UNINDENTED_BREAK);
}

#[test]
fn smart_indent_off_in_a_buffer_without_a_language_does_not_copy() {
    let mut ed = editor_from("  foo {-[}]>\n");
    type_cmd(&mut ed, ":set global indent-smart=false");
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "  foo {\n-[}]>\n");
}

#[test]
fn o_with_smart_indent_off_opens_a_line_at_the_left_margin() {
    let mut ed = rust_indent_editor("fn f() {\n    -[a]>();\n}\n");
    type_cmd(&mut ed, ":set buffer indent-smart=false");
    ed.feed_key(key('o'));
    assert_eq!(state(&ed), "fn f() {\n    a();\n-[\n]>}\n");
}

#[test]
fn big_o_with_smart_indent_off_opens_a_line_at_the_left_margin() {
    let mut ed = rust_indent_editor("fn f() {\n    a();\n    -[b]>();\n}\n");
    type_cmd(&mut ed, ":set buffer indent-smart=false");
    ed.feed_key(key('O'));
    assert_eq!(state(&ed), "fn f() {\n    a();\n-[\n]>    b();\n}\n");
}

#[test]
fn escape_after_an_unindented_enter_leaves_the_empty_line() {
    let mut ed = rust_indent_editor("fn f() {\n    a();-[\n]>}\n");
    type_cmd(&mut ed, ":set buffer indent-smart=false");
    ed.feed_keys([key('i'), key_enter()]);
    assert_eq!(state(&ed), "fn f() {\n    a();\n-[\n]>}\n");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n    a();\n\n}\n");
}

/// The spaces typed on the line an unindented Enter opened are the user's,
/// so neither Esc nor a second Enter vacates them.
#[test]
fn whitespace_typed_after_an_unindented_enter_is_kept() {
    let mut ed = rust_indent_editor("fn f() {\n    a();-[\n]>}\n");
    type_cmd(&mut ed, ":set buffer indent-smart=false");
    ed.feed_keys([key('i'), key_enter(), key(' '), key(' ')]);
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "fn f() {\n    a();\n  \n-[\n]>}\n");
    ed.feed_keys([key(' '), key(' '), key_esc()]);
    assert_eq!(
        ed.doc().text().to_string(),
        "fn f() {\n    a();\n  \n  \n}\n"
    );
}

#[test]
fn a_buffer_override_turns_smart_indent_on_over_a_global_off() {
    let mut ed = rust_indent_editor(OPEN_BLOCK);
    ed.state.settings.indent_smart = false;
    ed.doc_mut().overrides.indent_smart = Some(true);
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "fn f() {\n\t-[\n]>}\n");
}

#[test]
fn a_buffer_without_a_language_copies_the_indent() {
    let mut ed = editor_from("  foo {-[}]>\n");
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "  foo {\n  -[}]>\n");
}

#[test]
fn a_grammar_without_an_indents_query_copies_the_indent() {
    require_fixtures();
    let mut ed = editor_from("{\n  \"a\": [-[]]>\n}\n");
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    super::language::attach_host(&mut ed, "");
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "{\n  \"a\": [\n  -[]]>\n}\n");
}

// ── Trees and contexts ──────────────────────────────────────────────────────

#[test]
fn enter_uses_the_tree_of_the_text_typed_since_the_last_parse() {
    let mut ed = rust_indent_editor("-[\n]>");
    ed.feed_key(key('i'));
    for ch in "fn f() {".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "fn f() {\n\t-[\n]>}\n");
}

#[test]
fn enter_between_braces_inside_a_string_is_a_plain_break() {
    each_style(|style, unit| {
        let ed = enter_at("fn f() {\n    let s = \"{-[}]>\";\n}\n", style, unit);
        assert_eq!(
            state(&ed),
            indented("fn f() {\n    let s = \"{\n<T>-[}]>\";\n}\n", unit)
        );
    });
}

#[test]
fn enter_bound_through_insert_key_matches_the_enter_key() {
    let source = "fn f() {-[}]>\n";
    let mut plain = rust_indent_editor(source);
    plain.feed_key(key('i'));
    plain.feed_key(key_enter());

    let tmp = tempfile::tempdir().unwrap();
    let mut bound = rust_indent_editor(source);
    bind_insert_key(
        &mut bound,
        tmp.path(),
        "enter",
        r#"(begin (log! 'warn "enter-probe") (insert-key! pane "enter"))"#,
    );
    bound.feed_key(key('i'));
    bound.feed_key(key_enter());

    assert!(
        bound
            .state
            .message_log
            .entries()
            .any(|e| e.text == "enter-probe"),
        "the Insert binding for Enter did not run"
    );

    assert_eq!(state(&bound), state(&plain));
    assert_eq!(state(&plain), "fn f() {\n\t-[\n]>}\n");
}

#[test]
fn enter_in_a_fenced_block_starts_from_the_fences_indent() {
    require_fixtures();
    let buf = crate::editor::buffer::Buffer::at_start(hume_editing::text::BufferText::from(
        "- item\n\n  ```rust\n  fn f() {}\n  ```\n",
    ));
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();
    for (name, symbol, exts) in [
        ("markdown", "tree_sitter_markdown", &["md"][..]),
        ("markdown.inline", "tree_sitter_markdown_inline", &[][..]),
        ("rust", "tree_sitter_rust", &["rs"][..]),
    ] {
        ed.state
            .config
            .languages
            .register_identity(name, exts, &[], &[])
            .unwrap();
        let queries = match name {
            "markdown" => QueryFiles::new(grammar_query_path(name)).with(
                OptionalQuery::Injections,
                require_helix_query(name, OptionalQuery::Injections.file_name()),
            ),
            "rust" => rust_indent_queries(),
            _ => QueryFiles::new(grammar_query_path(name)),
        };
        attach_fixture_grammar_with(&mut ed, name, symbol, &queries);
    }
    ed.scripting = Some(ScriptingHost::new(&Dirs::none()));
    let lang = ed.state.config.languages.intern("markdown");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();

    // Cursor on the `}` of the fenced `fn f() {}`.
    ed.feed_keys("jjjllllllllll".chars().map(key));
    ed.state.settings.indent_style = TabStyle::Soft;
    // The edit leaves the text as it was and the tree one generation behind;
    // only the injected rust layer has an indents query, so a gate reading
    // the root grammar alone would skip the reparse and copy the fence's
    // indent: `  fn f() {\n  }`.
    ed.feed_keys([key('i'), key('x'), key_backspace()]);
    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "- item\n\n  ```rust\n  fn f() {\n      \n  }\n  ```\n"
    );
}

// ── Electric dedent ─────────────────────────────────────────────────────────

/// A function body whose one statement `o` opens a line below.
const BODY: &str = "fn f() {\n    -[a]>();\n";

#[test]
fn a_closing_brace_typed_on_a_line_o_opened_dedents_it() {
    each_style(|style, _| {
        let mut ed = rust_indent_editor(BODY);
        ed.state.settings.indent_style = style;
        ed.feed_keys([key('o'), key('}')]);
        assert_eq!(state(&ed), "fn f() {\n    a();\n}-[\n]>");
        ed.feed_key(key_esc());
        assert_eq!(state(&ed), "fn f() {\n    a();\n-[}]>\n");
    });
}

#[test]
fn a_closing_brace_typed_on_a_line_big_o_opened_dedents_it() {
    each_style(|style, unit| {
        let mut ed = rust_indent_editor("fn f() {\n    a();\n-[b]>();\n");
        ed.state.settings.indent_style = style;
        ed.feed_key(key('O'));
        assert_eq!(
            state(&ed),
            indented("fn f() {\n    a();\n<T>-[\n]>b();\n", unit)
        );
        ed.feed_key(key('}'));
        assert_eq!(state(&ed), "fn f() {\n    a();\n}-[\n]>b();\n");
    });
}

#[test]
fn a_dedented_line_and_its_typing_are_one_undo_step() {
    let mut ed = rust_indent_editor(BODY);
    ed.feed_keys([key('o'), key('}'), key_esc()]);
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n    a();\n}\n");
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n    a();\n");
}

/// Typing on after the dedent leaves the line where the dedent put it.
#[test]
fn typing_on_after_a_dedent_keeps_the_line() {
    let mut ed = rust_indent_editor(BODY);
    ed.feed_keys([key('o'), key('}'), key(';')]);
    assert_eq!(state(&ed), "fn f() {\n    a();\n};-[\n]>");
}

#[test]
fn a_python_elif_clause_dedents_once_its_colon_is_typed() {
    require_grammars(&["python"]);
    let indents = require_helix_query("python", OptionalQuery::Indents.file_name());
    let queries =
        QueryFiles::new(grammar_query_path("python")).with(OptionalQuery::Indents, indents);
    let mut ed = indent_editor("python", &queries, "if a:\n    -[x]>\n");
    ed.state.settings.indent_style = TabStyle::Soft;
    ed.feed_key(key('o'));
    type_chars(&mut ed, "elif c");
    assert_eq!(state(&ed), "if a:\n    x\n    elif c-[\n]>");
    ed.feed_key(key(':'));
    assert_eq!(state(&ed), "if a:\n    x\nelif c:-[\n]>");
}

#[test]
fn closing_braces_typed_at_several_cursors_dedent_each_line() {
    each_style(|style, _| {
        let mut ed = rust_indent_editor("fn f() {\n    -{a}>();\nfn g() {\n    -[b]>();\n");
        ed.state.settings.indent_style = style;
        ed.feed_keys([key('o'), key('}')]);
        assert_eq!(
            state(&ed),
            "fn f() {\n    a();\n}-{\n}>fn g() {\n    b();\n}-[\n]>"
        );
    });
}

/// Backspace took the computed indent and two typed spaces replaced it:
/// the line's whitespace is the user's, so the brace stays where typed.
#[test]
fn a_line_reindented_by_hand_is_not_dedented() {
    each_style(|style, _| {
        let mut ed = rust_indent_editor(BODY);
        ed.state.settings.indent_style = style;
        ed.feed_keys([key('o'), key_backspace(), key(' '), key(' '), key('}')]);
        assert_eq!(state(&ed), "fn f() {\n    a();\n  }-[\n]>");
    });
}

#[test]
fn a_pasted_closing_brace_is_not_dedented() {
    let mut ed = rust_indent_editor(BODY);
    ed.state.settings.indent_style = TabStyle::Soft;
    ed.feed_key(key('o'));
    ed.feed_paste("}");
    assert_eq!(state(&ed), "fn f() {\n    a();\n    }-[\n]>");
}

/// The pasted brace waits under the cursor; typing `}` there steps over
/// it, inserting nothing, so the line keeps its indent.
#[test]
fn stepping_over_a_closing_brace_does_not_dedent() {
    let mut ed = rust_indent_editor(BODY);
    ed.state.settings.indent_style = TabStyle::Soft;
    ed.feed_key(key('o'));
    ed.feed_paste("}");
    ed.feed_keys([key_left(), key('}')]);
    assert_eq!(state(&ed), "fn f() {\n    a();\n    }-[\n]>");
}

/// `o` opens the line with `indent-smart` on; the `}` is typed with it off.
#[test]
fn smart_indent_off_neither_dedents_nor_reparses() {
    let mut ed = rust_indent_editor(BODY);
    ed.state.settings.indent_style = TabStyle::Soft;
    ed.feed_key(key('o'));
    ed.doc_mut().overrides.indent_smart = Some(false);
    ed.feed_key(key('}'));
    assert_eq!(state(&ed), "fn f() {\n    a();\n    }-[\n]>");
    let generation = ed.doc().text().generation();
    assert!(
        !ed.doc()
            .syntax
            .as_ref()
            .expect("the buffer has a tree")
            .is_current(generation),
        "the tree stays at the text before the keystrokes"
    );
}

/// The source `}` triggers counts its calls. It runs once, on the
/// dedented text, and its session keeps its answer.
#[test]
fn a_completion_trigger_sees_the_dedented_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = rust_indent_editor(BODY);
    run(
        &mut ed,
        tmp.path(),
        r#"(define brace-calls 0)
           (register-completion-source! "brace"
             (lambda (id pane prefix)
               (set! brace-calls (+ brace-calls 1))
               (log! 'warn (buffer-text pane))
               (completion-emit! id (list (hash "label" (string-append "brace" (number->string brace-calls))))))
             #:target 'buffer)
           (set-completion-triggers! "brace" "rust" (list "}"))"#,
    );
    ed.feed_keys([key('o'), key('}')]);
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n    a();\n}\n");
    let seen: Vec<_> = ed
        .state
        .message_log
        .entries()
        .filter(|entry| entry.text.starts_with("fn f()"))
        .map(|entry| entry.text.clone())
        .collect();
    assert_eq!(seen, ["fn f() {\n    a();\n}\n"]);
    assert!(
        ed.state.input.buffer_completion().is_some(),
        "the session the brace opened is still open"
    );
}

/// Under hard tabs `o` below the 6-column `a();` opens `\t  `, one level
/// in from `fn f() {` by hybrid width. `}` dedents it to 2 columns, two
/// spaces sharing no prefix with the tab, so the record must be re-armed
/// at the line start. Deleting the `}` leaves those two spaces as the
/// session's own blank indent, which Esc then vacates.
#[test]
fn a_dedented_line_stays_the_sessions_own_to_vacate() {
    let mut ed = rust_indent_editor("fn f() {\n      -[a]>();\n");
    ed.state.settings.indent_style = TabStyle::Hard;
    ed.feed_key(key('o'));
    assert_eq!(state(&ed), "fn f() {\n      a();\n\t  -[\n]>");
    ed.feed_key(key('}'));
    assert_eq!(state(&ed), "fn f() {\n      a();\n  }-[\n]>");
    ed.feed_keys([key_backspace(), key_esc()]);
    assert_eq!(ed.doc().text().to_string(), "fn f() {\n      a();\n\n");
}

/// Characterizes `.` over `o}`: the replayed `}` dedents its line too.
#[test]
fn dot_repeat_replays_the_dedent() {
    let mut ed = rust_indent_editor("fn f() {\n    -[a]>();\nfn g() {\n    b();\n");
    ed.feed_keys([key('o'), key('}'), key_esc()]);
    ed.feed_keys([key('j'), key('j'), key('.')]);
    assert_eq!(
        ed.doc().text().to_string(),
        "fn f() {\n    a();\n}\nfn g() {\n    b();\n}\n"
    );
}

/// Characterizes a mixed multicursor keystroke: the line in `f`'s body
/// dedents, while the top-level line's computed width is already its own,
/// so it is left as typed.
#[test]
fn only_a_line_whose_indent_narrows_is_rewritten_among_several_cursors() {
    let mut ed = rust_indent_editor("fn f() {\n    -{a}>();\n-[x]>;\n");
    ed.state.settings.indent_style = TabStyle::Soft;
    ed.feed_keys([key('o'), key('}')]);
    assert_eq!(state(&ed), "fn f() {\n    a();\n}-{\n}>x;\n}-[\n]>");
}

/// An editor over the marker-DSL `source` with the python grammar and an
/// indents query making only the bare keyword `else` an outdent, writing
/// soft tabs.
fn python_else_editor(source: &str) -> Editor {
    require_grammars(&["python"]);
    let tmp = tempfile::tempdir().unwrap();
    let indents = tmp.path().join("indents.scm");
    std::fs::write(&indents, r#""else" @outdent"#).unwrap();
    let queries =
        QueryFiles::new(grammar_query_path("python")).with(OptionalQuery::Indents, indents);
    let mut ed = indent_editor("python", &queries, source);
    ed.state.settings.indent_style = TabStyle::Soft;
    ed
}

/// `o` below `x` opens a line at `x`'s 4 columns. Typed, `else` could
/// still grow into an identifier; Enter cannot continue it, so the line
/// dedents to 0: `else` opens it, one level less than the baseline `x`.
/// The new line is measured from the dedented `else` line, which it does
/// not open: one level more, 4.
#[test]
fn enter_after_a_closing_keyword_dedents_its_line() {
    let mut ed = python_else_editor("if a:\n    -[x]>\n");
    ed.feed_key(key('o'));
    type_chars(&mut ed, "else");
    assert_eq!(state(&ed), "if a:\n    x\n    else-[\n]>");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "if a:\n    x\nelse\n    -[\n]>");
}

/// `elsewh` is one identifier, which the query does not name, so the line
/// keeps its indent and the new line follows it.
#[test]
fn enter_after_a_word_starting_with_a_closing_keyword_keeps_its_line() {
    let mut ed = python_else_editor("if a:\n    -[x]>\n");
    ed.feed_key(key('o'));
    type_chars(&mut ed, "elsewh");
    ed.feed_key(key_enter());
    assert_eq!(state(&ed), "if a:\n    x\n    elsewh\n    -[\n]>");
}

#[test]
fn a_dedent_on_enter_and_the_break_are_one_undo_step() {
    let mut ed = python_else_editor("if a:\n    -[x]>\n");
    ed.feed_key(key('o'));
    type_chars(&mut ed, "else");
    ed.feed_keys([key_enter(), key_esc()]);
    assert_eq!(ed.doc().text().to_string(), "if a:\n    x\nelse\n\n");
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "if a:\n    x\n");
}

/// Esc selects the typed `else` on line 2, so three `j`s reach `y` on
/// line 5.
#[test]
fn dot_repeat_replays_a_dedent_on_enter() {
    let mut ed = python_else_editor("if a:\n    -[x]>\nif b:\n    y\n");
    ed.feed_key(key('o'));
    type_chars(&mut ed, "else");
    ed.feed_keys([key_enter(), key_esc()]);
    assert_eq!(state(&ed), "if a:\n    x\n-[else]>\n\nif b:\n    y\n");
    ed.feed_keys([key('j'), key('j'), key('j'), key('.')]);
    assert_eq!(
        ed.doc().text().to_string(),
        "if a:\n    x\nelse\n\nif b:\n    y\nelse\n\n"
    );
}

#[test]
fn enter_after_closing_keywords_at_several_cursors_dedents_each_line() {
    let mut ed = python_else_editor("if a:\n    -{x}>\nif b:\n    -[y]>\n");
    ed.feed_key(key('o'));
    type_chars(&mut ed, "else");
    ed.feed_key(key_enter());
    assert_eq!(
        state(&ed),
        "if a:\n    x\nelse\n    -{\n}>if b:\n    y\nelse\n    -[\n]>"
    );
}

/// Characterizes a mixed multicursor Enter. Enter splits the line after
/// `x` and the line `ywh` after `y`; both new lines are armed at 4 columns.
/// `else` typed at both cursors ends one line and starts the identifier
/// `elsewh` on the other. The second Enter dedents only the `else` line:
/// the break inside `elsewh` leaves its line as it is, and the new `wh`
/// line follows it at 4.
#[test]
fn enter_dedents_only_the_line_a_closing_keyword_ends_among_several_cursors() {
    let mut ed = python_else_editor("if a:\n    x-{\n}>if b:\n    y-[w]>h\n");
    ed.feed_keys([key('i'), key_enter()]);
    type_chars(&mut ed, "else");
    assert_eq!(
        state(&ed),
        "if a:\n    x\n    else-{\n}>if b:\n    y\n    else-[w]>h\n"
    );
    ed.feed_key(key_enter());
    assert_eq!(
        state(&ed),
        "if a:\n    x\nelse\n    -{\n}>if b:\n    y\n    else\n    -[w]>h\n"
    );
}
