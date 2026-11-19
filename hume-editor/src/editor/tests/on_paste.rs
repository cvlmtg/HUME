//! `on-paste` handlers: what each paste path hands them, how their answers
//! land, and what a handler may not do.

use std::rc::Rc;

use super::*;
use crate::editor::commands;
use crate::editor::dispatch::CmdCtx;
use crate::editor::paste_plan::{PastePlan, PlannedPaste};
use crate::editor::registry::MappableCommand;
use pretty_assertions::assert_eq;

/// Wraps a replaced text as a Markdown link to the pasted text, leaving a
/// paste at a cursor as it is.
const LINK: &str = r#"(register-hook! 'on-paste
  (lambda (pane pasted previous)
    (and previous (string-append "[" previous "](" pasted ")"))))"#;

/// Marks a paste with what it replaced, `-` at a cursor.
const TAG: &str = r#"(register-hook! 'on-paste
  (lambda (pane pasted previous)
    (string-append pasted "<" (or previous "-") ">")))"#;

// ── p / P ─────────────────────────────────────────────────────────────────────

#[test]
fn p_over_a_selection_pastes_the_handlers_text_as_one_undo_step() {
    let mut ed = editor_with_steel("-[foo]> bar\n", LINK);
    seed_ring(&mut ed, &["url"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "-[[foo](url)]> bar\n");
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), "-[foo]> bar\n");
}

#[test]
fn capital_p_over_a_selection_pastes_the_handlers_text() {
    let mut ed = editor_with_steel("-[foo]> bar\n", LINK);
    seed_ring(&mut ed, &["url"]);
    ed.feed_key(key('P'));
    assert_eq!(state(&ed), "-[[foo](url)]> bar\n");
}

#[test]
fn a_paste_at_a_cursor_replaces_nothing() {
    let mut ed = editor_with_steel("-[a]>b\n", TAG);
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X<->]>b\n");
}

#[test]
fn a_smart_paste_onto_its_own_text_replaces_nothing() {
    let mut ed = editor_with_steel("-[X]>\n", TAG);
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "X-[X<->]>\n");
}

#[test]
fn a_false_answer_pastes_the_text_as_is() {
    let mut ed = editor_with_steel(
        "-[foo]>\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) #f))",
    );
    seed_ring(&mut ed, &["url"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "-[url]>\n");
}

#[test]
fn the_kill_ring_keeps_the_pasted_text() {
    let mut ed = editor_with_steel("-[foo]>\n", LINK);
    seed_ring(&mut ed, &["url"]);
    ed.feed_key(key('p'));
    assert_eq!(
        ed.state.kill_ring.head().map(<[_]>::to_vec),
        Some(vec!["url".into()])
    );
}

#[test]
fn each_selection_gets_its_own_value_and_replaced_text() {
    let mut ed = editor_with_steel("-[ab]> -{cd}>\n", TAG);
    ed.state.kill_ring.push(vec!["X", "Y"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "-[X<ab>]> -{Y<cd>}>\n");
}

#[test]
fn a_joined_value_reaches_each_selection_with_its_own_replaced_text() {
    let mut ed = editor_with_steel("-[ab]> -[cd]> -{ef}>\n", TAG);
    ed.state.kill_ring.push(vec!["X", "Y"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "-[XY<ab>]> -[XY<cd>]> -{XY<ef>}>\n");
}

#[test]
fn a_whole_lines_answer_gets_its_line_break() {
    let mut ed = editor_with_steel(
        "-[a]>b\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) \"X\"))",
    );
    seed_ring(&mut ed, &["line\n"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "ab\n-[X\n]>");
}

#[test]
fn an_empty_whole_lines_answer_pastes_nothing() {
    let mut ed = editor_with_steel(
        "-[a]>b\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) \"\"))",
    );
    seed_ring(&mut ed, &["line\n"]);
    ed.feed_key(key('p'));
    assert_eq!(text(&ed), "ab\n");
}

#[test]
fn an_empty_whole_lines_answer_over_a_mid_line_selection_keeps_the_line_whole() {
    let mut ed = editor_with_steel(
        "a-[bc]>d\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) \"\"))",
    );
    seed_ring(&mut ed, &["line\n"]);
    ed.feed_key(key('p'));
    assert_eq!(text(&ed), "ad\n");
}

// ── [ / ] ─────────────────────────────────────────────────────────────────────

#[test]
fn ring_cycling_hands_each_entry_the_text_the_paste_replaced() {
    let mut ed = editor_with_steel("-[foo]>\n", LINK);
    seed_ring(&mut ed, &["old", "new"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "-[[foo](new)]>\n");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "-[[foo](old)]>\n");
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), "-[foo]>\n");
}

// ── Handlers ──────────────────────────────────────────────────────────────────

#[test]
fn handlers_run_in_registration_order_on_each_others_text() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        r#"(register-hook! 'on-paste (lambda (pane pasted previous) (string-append pasted "1")))
           (register-hook! 'on-paste (lambda (pane pasted previous) #f))
           (register-hook! 'on-paste (lambda (pane pasted previous) (string-append pasted "2")))"#,
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X12]>\n");
}

#[test]
fn a_raising_handler_is_reported_and_the_next_one_still_runs() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        r#"(register-hook! 'on-paste (lambda (pane pasted previous) (error "boom")))
           (register-hook! 'on-paste (lambda (pane pasted previous) (string-append pasted "!")))"#,
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X!]>\n");
    assert!(
        has_error(&ed, "on-paste handler from user: boom"),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn an_answer_that_is_not_text_is_reported_and_pastes_the_text_as_is() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) 42))",
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(
            &ed,
            "on-paste handler from user: returned 42, not a string or #f"
        ),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_symbol_answer_is_reported_and_pastes_the_text_as_is() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) 'foo))",
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(
            &ed,
            "on-paste handler from user: returned foo, not a string or #f"
        ),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_stopped_by_the_budget_lets_the_text_paste_as_is() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        "(register-hook! 'on-paste (lambda (pane pasted previous) (let loop () (loop))))",
    );
    ed.state.settings.steel_command_budget_ms = 50;
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(has_error(&ed, "on-paste: "), "{:?}", error_log(&ed));
}

#[test]
fn a_script_runs_after_one_whose_paste_handler_ran_out_of_budget() {
    let mut ed = editor_with_steel(
        "-[a]>b\n",
        r#"(register-hook! 'on-paste (lambda (pane pasted previous) (let loop () (loop))))
           (define-command! "probe" "" (lambda (pane) (paste-text! pane "X")))
           (define-command! "step" "" (lambda (pane) (call! "move-right" pane)))"#,
    );
    ed.state.settings.steel_command_budget_ms = 50;
    ed.execute_keymap_command("probe", Count::typed(1), false);
    ed.execute_keymap_command("step", Count::typed(1), false);
    assert_eq!(state(&ed), "Xa-[b]>\n");
}

/// A handler whose `body` runs before it answers "Y", pasting "X" with `p`
/// at `-[a]>`: what `p` gives when the handler raised and the paste went in
/// as it was.
fn pasted_past_handler(body: &str) -> Editor {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        &format!(
            r#"(register-hook! 'on-paste
                 (lambda (pane pasted previous) {body} "Y"))"#
        ),
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    ed
}

/// The refusal a handler gets from `function` for a write that goes
/// through the editor.
fn cannot_change(function: &str) -> String {
    format!("{function}: the editor cannot be changed while an on-paste handler runs")
}

#[test]
fn a_handler_cannot_run_an_editor_command() {
    let ed = pasted_past_handler(r#"(call! "move-right" pane)"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("call!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_command_a_handler_calls_cannot_change_the_editor() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        r#"(define-command! "probe" ""
             (lambda (pane) (log! 'warning "probe ran") (call! "move-right" pane)))
           (register-hook! 'on-paste
             (lambda (pane pasted previous) (call! "probe" pane) "Y"))"#,
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        log_entries(&ed).contains(&(Severity::Warning, "probe ran")),
        "{:?}",
        log_entries(&ed)
    );
    assert!(
        has_error(&ed, &cannot_change("call!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_paste() {
    let ed = pasted_past_handler(r#"(paste-text! pane "Z")"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("paste-text!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_write_to_the_terminal() {
    let ed = pasted_past_handler(r#"(print! "Z")"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("print!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_start_finishing_a_paste() {
    let ed = pasted_past_handler("(%start-paste-finish!)");
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(
            &ed,
            "on-paste handler from user: the editor cannot be changed while an on-paste handler runs"
        ),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_edit_the_buffer() {
    let ed = pasted_past_handler(
        r#"(edit-text! pane (list (hash 'start-char 0 'end-char 1 'text "E")))"#,
    );
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("edit-text!")),
        "{:?}",
        error_log(&ed)
    );
    assert!(!has_error(&ed, "paste dropped"), "{:?}", error_log(&ed));
}

#[test]
fn a_handler_cannot_move_the_cursor() {
    let ed = pasted_past_handler("(goto-location! pane (hash 'target pane 'char-offset 0))");
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("goto-location!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_set_an_option() {
    let ed = pasted_past_handler(r#"(set-option! "scroll-margin" 42)"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert_ne!(ed.state.settings.scroll_margin, 42);
    assert!(
        has_error(&ed, &cannot_change("set-option!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_set_the_register_prefix() {
    let ed = pasted_past_handler(r#"(set-register-prefix! "0")"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("set-register-prefix!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_cannot_queue_an_editor_change() {
    let ed = pasted_past_handler(r#"(set-completion-triggers! "rust" "brace" (list #\}))"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(&ed, &cannot_change("set-completion-triggers!")),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_handler_can_read_the_editor() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        r#"(register-hook! 'on-paste
             (lambda (pane pasted previous)
               (string-append pasted (number->string (string-length (buffer-text pane))))))"#,
    );
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X2]>\n");
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

#[test]
fn a_handler_can_read_a_register() {
    let mut ed = editor_with_steel(
        "-[a]>\n",
        r#"(register-hook! 'on-paste
             (lambda (pane pasted previous) (string-append pasted (car (read-register "3")))))"#,
    );
    ed.state.registers.write_text('3', vec!["R".to_string()]);
    seed_ring(&mut ed, &["X"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[XR]>\n");
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

#[test]
fn a_handler_that_catches_a_refusal_pastes_the_text_as_is() {
    let ed =
        pasted_past_handler(r#"(with-handler (lambda (e) #f) (set-option! "scroll-margin" 42))"#);
    assert_eq!(state(&ed), "a-[X]>\n");
    assert!(
        has_error(
            &ed,
            "the editor cannot be changed while an on-paste handler runs"
        ),
        "{:?}",
        error_log(&ed)
    );
}

#[test]
fn a_script_can_change_the_editor_after_its_paste_handlers_ran() {
    let mut ed = editor_with_steel(
        "-[a]>bc\n",
        &format!(
            r#"{TAG}
               (define-command! "probe" ""
                 (lambda (pane) (paste-text! pane "X") (call! "move-right" pane)))"#
        ),
    );
    ed.execute_keymap_command("probe", Count::typed(1), false);
    assert_eq!(state(&ed), "X<->a-[b]>c\n");
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

#[test]
fn a_script_can_change_the_editor_after_a_paste_handler_raised() {
    let mut ed = editor_with_steel(
        "-[a]>bc\n",
        r#"(register-hook! 'on-paste (lambda (pane pasted previous) (error "boom")))
           (define-command! "probe" ""
             (lambda (pane) (paste-text! pane "X") (call! "move-right" pane)))"#,
    );
    ed.execute_keymap_command("probe", Count::typed(1), false);
    assert_eq!(state(&ed), "Xa-[b]>c\n");
    assert!(has_error(&ed, "boom"), "{:?}", error_log(&ed));
}

#[test]
fn a_handler_takes_the_pane_and_both_texts() {
    let mut ed = editor_from("-[a]>\n");
    let mut host = editor_host(&mut ed);
    let mut init = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let err = host
        .eval_source(
            "(register-hook! 'on-paste (lambda (pasted previous) #f))",
            &mut init,
        )
        .expect_err("a two-argument handler is refused");
    assert!(
        err.contains("an 'on-paste handler is a lambda of 3 parameters"),
        "{err}"
    );
}

// ── Terminal paste ────────────────────────────────────────────────────────────

#[test]
fn a_terminal_paste_over_a_selection_pastes_the_handlers_text_as_one_undo_step() {
    let mut ed = editor_with_steel("-[foo]> bar\n", LINK);
    ed.feed_paste("url");
    assert_eq!(text(&ed), "[foo](url) bar\n");
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), "-[foo]> bar\n");
}

#[test]
fn an_insert_mode_paste_lands_in_the_insert_sessions_undo_step() {
    let mut ed = editor_with_steel("-[a]>\n", TAG);
    ed.feed_keys([key('i'), key('Q')]);
    ed.feed_paste("X");
    ed.feed_key(key_esc());
    assert_eq!(text(&ed), "QX<->a\n");
    ed.feed_key(key('u'));
    assert_eq!(text(&ed), "a\n");
}

// ── paste-text! and call! ─────────────────────────────────────────────────────

#[test]
fn paste_text_lands_the_handlers_text_before_the_script_goes_on() {
    let mut ed = editor_with_steel(
        "-[foo]>\n",
        &format!(
            r#"{LINK}
               (define-command! "probe" ""
                 (lambda (pane)
                   (paste-text! pane "url")
                   (log! 'warning (buffer-text pane))))"#
        ),
    );
    ed.execute_keymap_command("probe", Count::typed(1), false);
    assert_eq!(text(&ed), "[foo](url)\n");
    assert!(
        log_entries(&ed).contains(&(Severity::Warning, "[foo](url)\n")),
        "{:?}",
        log_entries(&ed)
    );
}

#[test]
fn a_paste_command_called_from_a_script_lands_the_handlers_text() {
    let mut ed = editor_with_steel(
        "-[foo]>\n",
        &format!(
            r#"{LINK}
               (define-command! "probe" ""
                 (lambda (pane) (log! 'warning (to-string (call! "paste-after" pane)))))"#
        ),
    );
    seed_ring(&mut ed, &["url"]);
    ed.execute_keymap_command("probe", Count::typed(1), false);
    assert_eq!(state(&ed), "-[[foo](url)]>\n");
    assert!(
        log_entries(&ed).contains(&(Severity::Warning, "#true")),
        "{:?}",
        log_entries(&ed)
    );
}

// ── Dot-repeat ────────────────────────────────────────────────────────────────

#[test]
fn dot_repeat_hands_the_handler_the_new_selections_text() {
    let mut ed = editor_with_steel("-[foo]> bar\n", LINK);
    seed_ring(&mut ed, &["url"]);
    ed.feed_key(key('p'));
    select_marked(&mut ed, "[foo](url) -[bar]>\n");
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "[foo](url) -[[bar](url)]>\n");
}

#[test]
fn dot_repeat_of_an_insert_mode_paste_runs_the_handler_again() {
    let mut ed = editor_with_steel("-[a]>b\n", TAG);
    ed.feed_key(key('i'));
    ed.feed_paste("X");
    ed.feed_key(key_esc());
    select_marked(&mut ed, "X<->a-[b]>\n");
    ed.feed_key(key('.'));
    assert_eq!(text(&ed), "X<->aX<->b\n");
}

// ── A paste that cannot land ──────────────────────────────────────────────────

/// `planned`, parked and taken as a script's `%finish-paste!` takes it, to
/// land it later.
fn finishing(ed: &mut Editor, planned: PlannedPaste) -> PastePlan {
    ed.state.park_paste(planned);
    assert!(ed.state.start_paste_finish());
    ed.state
        .take_finishing_paste()
        .expect("the paste just started finishing")
}

/// The paste `p`-family command `name` plans at the focused pane, run through
/// the command pipeline.
fn planned_by(ed: &mut Editor, name: &str) -> PlannedPaste {
    let Some(MappableCommand::Native(cmd)) = ed.state.config.registry.get_mappable(name).cloned()
    else {
        panic!("'{name}' is a native command");
    };
    let fp = FocusedPane::current(&ed.view);
    let Ok(bound) = commands::BoundCommand::focused(cmd, fp, None) else {
        panic!("'{name}' takes no operand");
    };
    let ctx = CmdCtx {
        count: Count::typed(1),
        extend: false,
    };
    let Ok(planned) = commands::run(&mut ed.state, &mut ed.view, bound, ctx) else {
        panic!("'{name}' refused: {:?}", error_log(ed));
    };
    planned
}

/// A terminal paste of `values` planned at the focused pane's selections.
fn planned_paste_of(ed: &mut Editor, values: &[&str]) -> PlannedPaste {
    let values: Rc<[String]> = values.iter().map(|v| (*v).to_owned()).collect();
    ed.state
        .plan_selections_paste(&ed.view, values)
        .expect("the buffer may change")
}

/// Why `plan` with `answers` does not land.
fn landing_error(ed: &mut Editor, plan: PastePlan, answers: Vec<Option<String>>) -> String {
    let landed = ed.state.apply_paste_plan(&ed.view, plan, answers);
    landed
        .expect_err("the paste cannot land")
        .message()
        .to_owned()
}

#[test]
fn a_paste_whose_buffer_changed_while_it_waited_is_dropped() {
    let mut ed = editor_from("-[a]>b\n");
    let planned = planned_paste_of(&mut ed, &["X"]);
    let plan = finishing(&mut ed, planned);
    ed.feed_key(key('d'));
    let error = landing_error(&mut ed, plan, Vec::new());
    assert!(error.contains("its buffer changed"), "{error}");
    assert_eq!(text(&ed), "b\n");
}

#[test]
fn a_ring_cycle_whose_paste_session_ended_while_it_waited_is_dropped() {
    let mut ed = editor_from("-[a]>\n");
    seed_ring(&mut ed, &["old", "new"]);
    ed.feed_key(key('p'));
    let planned = planned_by(&mut ed, "paste-ring-older");
    let plan = finishing(&mut ed, planned);
    ed.state.commit_paste_session();
    let error = landing_error(&mut ed, plan, Vec::new());
    assert!(error.contains("its paste session ended"), "{error}");
    assert_eq!(state(&ed), "a-[new]>\n");
}

#[test]
fn more_answers_than_selections_are_refused() {
    let mut ed = editor_from("-[a]>\n");
    let planned = planned_paste_of(&mut ed, &["X"]);
    let plan = finishing(&mut ed, planned);
    let answers = vec![Some("Y".to_owned()), Some("Z".to_owned())];
    let error = landing_error(&mut ed, plan, answers);
    assert!(error.contains("2 answers for 1 selections"), "{error}");
    assert_eq!(text(&ed), "a\n");
}

// ── Where handlers run ────────────────────────────────────────────────────────

#[test]
fn ring_cycling_newer_hands_each_entry_the_text_the_paste_replaced() {
    let mut ed = editor_with_steel("-[foo]>\n", LINK);
    seed_ring(&mut ed, &["old", "new"]);
    ed.feed_keys([key('p'), key('['), key(']')]);
    assert_eq!(state(&ed), "-[[foo](new)]>\n");
}

#[test]
fn a_terminal_paste_into_a_prompt_runs_no_handler() {
    let mut ed = editor_with_steel("-[a]>\n", TAG);
    ed.feed_key(key(':'));
    ed.feed_paste("X");
    assert_eq!(minibuf_input(&ed), "X");
}

#[test]
fn a_paste_into_a_read_only_buffer_is_refused_before_any_handler_runs() {
    let mut ed = read_only_editor_from("-[a]>\n");
    attach_steel(
        &mut ed,
        r#"(register-hook! 'on-paste
             (lambda (pane pasted previous) (log! 'warning "handler ran") #f))"#,
    );
    ed.feed_paste("X");
    assert_eq!(state(&ed), "-[a]>\n");
    assert_eq!(status(&ed), read_only_refusal(&ed));
    assert!(
        !log_entries(&ed).contains(&(Severity::Warning, "handler ran")),
        "{:?}",
        log_entries(&ed)
    );
}

#[test]
fn a_paste_with_the_completion_menu_open_runs_the_handlers() {
    let mut ed = editor_with_steel("-[a]>b\n", TAG);
    ed.feed_key(key('i'));
    open_completion_session(&mut ed, &["alpha"]);
    assert!(ed.state.input.buffer_completion().is_some());
    ed.feed_paste("X");
    ed.feed_key(key_esc());
    assert_eq!(text(&ed), "X<->ab\n");
}
