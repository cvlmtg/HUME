//! Arity-1 command list-arg validation, load-plugin runtime-guard
//! firing, and the plum grammars.scm balance check.

use super::*;

// ── arity-1 command list-arg validation (plum-ensure-grammars pattern) ───────

/// An arity-1 command that validates its arg is a non-empty list must error
/// when called with no args (#f from minibuffer path).
///
/// Flip: change `unless` guard to `(when #t ...)` and the error disappears.
#[test]
fn arity1_list_command_rejects_false_arg() {
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "needs-list" ""
             (lambda (items)
               (unless (and (list? items) (not (null? items)))
                 (error "needs-list: requires a non-empty list"))
               (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "needs-list",
            None,
            vec![SteelVal::BoolV(false)],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.message.contains("requires a non-empty list"),
        "expected list-required error, got: {err}"
    );
}

/// An arity-1 command that validates its arg is a non-empty list must succeed
/// when passed a real list.
///
/// Flip: change the guard to always error and the Ok below becomes Err.
#[test]
fn arity1_list_command_accepts_list_arg() {
    use steel::rvals::IntoSteelVal as _;
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "needs-list" ""
             (lambda (items)
               (unless (and (list? items) (not (null? items)))
                 (error "needs-list: requires a non-empty list"))
               (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    let items: Vec<SteelVal> = vec!["rust".into_steelval().unwrap()];
    let list_val = items.into_steelval().unwrap();
    let result = h.call_steel_cmd(
        "needs-list",
        None,
        vec![list_val],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    );
    assert!(
        result.is_ok(),
        "expected Ok for valid list arg, got: {:?}",
        result.err()
    );
}

/// `(load-plugin …)` raises a Steel error when called from a command body
/// (`EvalMode::Command`) — the `ensure_top_level` gate rejects it.
///
/// Flip: remove `ensure_top_level` from `load_plugin` and the call returns `Ok`,
/// silently queuing a load request that is never drained.
#[test]
fn load_plugin_runtime_guard_fires() {
    // (load-plugin ...) from a command body (EvalMode::Command) must be rejected.
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "try-load" "" (lambda () (load-plugin "user/tp")))"#,
        &mut mock,
    )
    .unwrap();

    let err = h
        .call_steel_cmd(
            "try-load",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.message.contains("top level") || err.message.contains("init.scm"),
        "error must mention top-level restriction; got: {err}"
    );
}

/// Test that core:plum grammars.scm has balanced parentheses.
/// This plugin is loaded via `(load-plugin "core:plum")` in init.scm.
/// An earlier imbalance caused "Parse: Unexpected EOF" on startup.
#[test]
fn plum_grammars_scm_balanced() {
    let src = include_str!("../../../runtime/plugins/core/plum/grammars.scm");

    // Count structural parens only. Parens inside string literals, `;` line
    // comments, `#| |#` block comments, and `#\(` char literals are not
    // structural and must be skipped, or the oracle is not independent of the
    // file's prose (a comment with an unbalanced paren would mask or fake an
    // imbalance in the actual code).
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // String literal — consume to the closing quote, honoring `\` escapes.
                while let Some(s) = chars.next() {
                    match s {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            ';' => {
                // Line comment — consume to end of line.
                while chars.next_if(|&s| s != '\n').is_some() {}
            }
            '#' if chars.peek() == Some(&'\\') => {
                // Char literal `#\x` (e.g. `#\(`) — skip the `\` and the char.
                chars.next();
                chars.next();
            }
            '#' if chars.peek() == Some(&'|') => {
                // Block comment `#| ... |#` — consume to the closing `|#`.
                chars.next();
                while let Some(s) = chars.next() {
                    if s == '|' && chars.next_if(|&t| t == '#').is_some() {
                        break;
                    }
                }
            }
            '(' => opens += 1,
            ')' => closes += 1,
            _ => {}
        }
    }

    assert_eq!(
        opens, closes,
        "grammars.scm: {opens} opens vs {closes} closes — unbalanced parens",
    );
}
