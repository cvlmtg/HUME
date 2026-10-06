// Hover: `lsp-hover` composing `lsp-request!`, `lsp-capabilities`,
// `show-popup!`, `show-drawer-list!`. Loads the
// real shipped `core:lsp` plugin in place (`RealRuntimeGuard` points
// HUME_RUNTIME at the actual on-disk runtime/ dir) so tests exercise the
// actual code, not a hand-rolled stand-in.
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;

/// A [`core_lsp_rig`] whose server answers `initialize` with
/// `initialize_result` (so `lsp-capabilities` decodes real data). The file
/// is real because `lsp-position-params` requires `buf.path()` to be
/// `Some`; a bare `editor_from` buffer won't do.
///
/// `configure` scripts any responses beyond `initialize` (e.g.
/// `textDocument/hover`) before the backend is boxed into `LspState`.
fn setup(
    tmp: &Path,
    initialize_result: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    // 30 lines: comfortably taller than the default pane height's ⅓-cap
    // (`Pane::new`'s default viewport is 24 rows tall; `(viewport-range bid)`
    // resolves against this immediately, no `prepare_frame` needed), so a
    // one-line hover response lands well under the popup/drawer threshold, whereas
    // a tiny 1-2 line fixture would make even trivial hover content overflow
    // to the drawer, which isn't what these tests are checking.
    let filler = (0..29)
        .map(|i| format!("// line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let marked = format!("-[f]>n main() {{}}\n{filler}\n");
    let (rig, guard) = core_lsp_rig(tmp, &marked, initialize_result, configure);
    let sid = rig.sid("rust-analyzer");
    (rig.ed, guard, sid)
}

fn popup_lines(ed: &Editor) -> Option<Vec<String>> {
    ed.state
        .views
        .popup
        .read()
        .as_ref()
        .map(|s| (*s.lines).clone())
}

fn run_hover(ed: &mut Editor) {
    // `lsp-hover` is key-bindable, not typed, so dispatch it the way `K` would,
    // through the keymap pipeline, not `:`.
    ed.execute_keymap_command("lsp-hover".into(), Some(1), false);
    // Settle *before* the async response arrives. Mirrors the real
    // interactive loop, which drains hooks after every keystroke, well
    // before any network response could land. Draining only at the end
    // would incorrectly replay any hooks queued by dispatch itself after
    // the popup is shown, closing it.
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

#[test]
fn popup_shows_the_fixture_content() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({
                    "contents": {"kind": "plaintext", "value": "fn main()"},
                    "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}
                }),
            );
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        popup_lines(&ed),
        Some(vec!["fn main()".to_string()]),
        "the worked fixture's MarkupContent value must render verbatim in the popup"
    );
}

/// `contents` as a `MarkedString[]` (the deprecated-but-still-emitted array
/// shape), mixing a bare string entry with a `{language, value}` entry:
/// the one branch of `lsp/marked-string->text`/`lsp/hover-contents->text`
/// `popup_shows_the_fixture_content`'s single-`MarkupContent` fixture never
/// reaches.
#[test]
fn popup_joins_a_marked_string_array_with_language_fences() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({
                    "contents": [
                        "plain note",
                        {"language": "rust", "value": "fn main()"}
                    ],
                    "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}
                }),
            );
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        popup_lines(&ed),
        Some(vec![
            "plain note".to_string(),
            "".to_string(),
            "```rust".to_string(),
            "fn main()".to_string(),
            "```".to_string(),
        ]),
        "array entries join with a blank line; the language entry fences as a code block"
    );
}

#[test]
fn null_result_logs_and_shows_no_popup() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to("textDocument/hover", serde_json::Value::Null);
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        popup_lines(&ed).is_none(),
        "null hover result must not open a popup"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no hover info"),
        "expected a 'no hover info' message, got {msg:?}"
    );
}

#[test]
fn error_reports_via_the_message_log() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.fail_with("textDocument/hover", -32603, "boom");
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        popup_lines(&ed).is_none(),
        "a protocol error must not open a popup"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("boom"),
        "expected the server's error message surfaced, got {msg:?}"
    );
}

#[test]
fn popup_is_scrollable_and_closes_on_any_key_except_ctrl_u_d() {
    let tmp = safe_tempdir();
    // 50 lines: comfortably taller than any popup's visible window (cursor
    // cap ~⅓ pane, docked cap ~½ terminal), so Ctrl-d/Ctrl-u below exercise a
    // real scroll, not a short popup with nothing to page through.
    let value = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        move |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": {"kind": "plaintext", "value": value}}),
            );
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    // 50 lines overflows to the docked layout (`popup_lines` only reads the
    // cursor-anchored view), so assert on the band view instead, mirroring
    // `tall_content_docks_instead_of_using_the_drawer`.
    assert!(
        ed.state
            .views
            .popup_band
            .read()
            .as_ref()
            .is_some_and(|s| !s.lines.is_empty()),
        "sanity: docked popup shown"
    );
    assert!(
        ed.state
            .input
            .ref_of::<crate::editor::input_stack::PopupLayer>()
            .is_some(),
        "hover must open a scrollable popup (`#:kind 'scrollable`, its own pushed layer), \
         not the sticky mode-change-only one (the mode layer's own slot)"
    );

    // Ctrl-d/Ctrl-u scroll the popup instead of closing it.
    ed.feed_key(key_ctrl('d'));
    assert!(
        ed.state.input.popup().is_some(),
        "Ctrl-d must scroll the hover popup, not close it"
    );
    ed.feed_key(key_ctrl('u'));
    assert!(
        ed.state.input.popup().is_some(),
        "Ctrl-u must scroll the hover popup, not close it"
    );

    // Any other key (here, cursor movement) dismisses it.
    ed.feed_key(key('j'));
    assert!(
        ed.state.input.popup().is_none(),
        "cursor movement must dismiss the hover popup, not just a mode change"
    );
}

#[test]
fn short_popup_falls_through_ctrl_d_instead_of_swallowing_it() {
    // A hover popup whose content fits on screen has nothing to scroll:
    // Ctrl-d/Ctrl-u must not become a silent no-op that also blocks the
    // buffer's own half-page scroll. `scroll_popup` must not consume the key
    // when `max_scroll == 0`.
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": {"kind": "plaintext", "value": "fn main()"}}),
            );
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    assert!(popup_lines(&ed).is_some(), "sanity: popup shown");

    let head_before = ed.current_view().primary().head().offset();
    ed.feed_key(key_ctrl('d'));
    assert!(
        ed.state.input.popup().is_none(),
        "Ctrl-d on a popup with nothing to scroll must close it, not swallow the key"
    );
    assert!(
        ed.current_view().primary().head().offset() > head_before,
        "Ctrl-d must fall through to the buffer's half-page-down motion once the popup closes"
    );
}

/// `lsp/visible-lines` is `viewport-range`'s exclusive width with no `+ 1`:
/// `viewport-range` is already end-exclusive, so adding an
/// inclusive-range `+ 1` would overcount by one line and shift
/// the ⅓ popup/drawer threshold (`lsp/show-hover`) by one. The default
/// 24-row pane can't tell the two formulas apart (`⌊24/3⌋ == ⌊25/3⌋ == 8`,
/// see `tall_content_docks_instead_of_using_the_drawer` below); a 23-row
/// pane can (`⌊23/3⌋ == 7`, `⌊24/3⌋ == 8`).
///
/// Computing one more than the range's width in `lib.scm`'s
/// `lsp/visible-lines` would report 24 visible lines instead of 23 and raise
/// the threshold to 8, so 8 content lines would float instead of dock.
#[test]
fn visible_lines_threshold_has_no_off_by_one_from_the_old_inclusive_range() {
    let tmp = safe_tempdir();
    let eight_lines = (0..8)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": eight_lines}),
            );
        },
    );
    ed.viewport_mut().height = 23;

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        popup_lines(&ed).is_none(),
        "8 lines against a 23-row pane (threshold 7) must dock, not float"
    );
    assert!(
        matches!(
            ed.state.input.popup().map(|p| &p.layout),
            Some(hume_ui::popup::PopupLayout::Docked)
        ),
        "must be a docked popup, not the drawer"
    );
}

#[test]
fn tall_content_docks_instead_of_using_the_drawer() {
    let tmp = safe_tempdir();
    // The fixture file is ~30 lines against the default 24-row pane height,
    // so the popup threshold (⅓ of visible lines) lands around 8, and 20 lines
    // must overflow to the docked layout regardless of the exact figure.
    let tall = (0..20)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to("textDocument/hover", serde_json::json!({"contents": tall}));
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        popup_lines(&ed).is_none(),
        "tall content must not use the cursor-anchored popup layout"
    );
    assert!(
        matches!(
            ed.state.input.popup().map(|p| &p.layout),
            Some(hume_ui::popup::PopupLayout::Docked)
        ),
        "tall content must still be a popup, just docked, never the drawer"
    );
    assert!(
        ed.state
            .views
            .popup_band
            .read()
            .as_ref()
            .is_some_and(|s| !s.lines.is_empty()),
        "the docked band's view must resolve after a frame"
    );
    assert!(
        ed.state.input.drawer().is_none(),
        "hover overflow must never open the pick-list drawer"
    );
}

#[test]
fn capability_gate_skips_the_request_when_hover_unsupported() {
    // No response scripted for "textDocument/hover". If the capability
    // gate failed open (called the request thunk anyway), the request
    // would go unanswered and `status_msg` would stay unset, not mention
    // "not supported". That is enough to tell, without inspecting the
    // (trait-erased, post-boxing unreachable) sent log.
    let tmp = safe_tempdir();
    // No hoverProvider in the advertised capabilities.
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {}}),
        |_backend, _sid| {},
    );

    run_hover(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("not supported"),
        "expected a not-supported message, got {msg:?}"
    );
}

#[test]
fn capability_gate_skips_the_request_when_the_provider_field_is_null() {
    // A `null` capability field is not the same as advertising support:
    // same check as the missing-key case above, just via an explicit
    // `null` rather than an absent key.
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": null}}),
        |_backend, _sid| {},
    );

    run_hover(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("not supported"),
        "expected a not-supported message, got {msg:?}"
    );
}

/// The user is free to switch buffers while a hover request is in flight:
/// an LSP round-trip is async. If the response lands after that, showing
/// hover text for a symbol the user is no longer looking at would be worse
/// than showing nothing: `lsp-hover` sends its request with
/// `#:require-focus #t`, which drops the callback (never even reaching
/// `hover.scm`'s own body) once the focused buffer no longer matches the
/// buffer that sent the request.
///
/// Without `#:require-focus` on `hover.scm`'s `lsp-request!`, the popup would
/// show regardless of which buffer answered, since a queued callback's
/// `(focused-pane)` is live focus at drain time.
#[test]
fn stale_response_after_a_buffer_switch_shows_no_popup() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": "fn main()"}),
            );
        },
    );

    // Sends the request synchronously; no settle() before the
    // switch below: settle() unconditionally drains LSP, which would
    // deliver the response (and close the race window) before the switch
    // ever happens.
    ed.execute_keymap_command("lsp-hover".into(), Some(1), false);

    let other = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other, "\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();

    ed.drain_lsp();
    ed.settle();
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        popup_lines(&ed),
        None,
        "a hover response for a buffer that's no longer focused must not surface a popup"
    );
}

#[test]
fn allow_stale_is_honored_despite_an_intervening_edit() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": "fn main()"}),
            );
        },
    );

    ed.execute_keymap_command("lsp-hover".into(), Some(1), false);

    // Bump the buffer's generation between send and drain. Without
    // #:allow-stale this response would be dropped. No settle() call until
    // after the edit: settle() unconditionally drains LSP too;
    // draining any earlier would deliver the
    // response (and run lsp-hover's close-on-mode-change dismiss) before
    // the edit ever happens, defeating the "intervening edit" this test
    // means to exercise. The `i`/`X`/Esc mode-change hooks below simply
    // accumulate in `pending_work`, unfired, until the one settle() call at
    // the end, by which point no popup exists yet for a dismiss to race
    // against.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        popup_lines(&ed),
        Some(vec!["fn main()".to_string()]),
        "hover must pass #:allow-stale #t and still show the popup after an intervening edit"
    );
}

#[test]
fn buffer_with_no_path_reports_and_sends_nothing() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": {"kind": "plaintext", "value": "WRONG"}}),
            );
        },
    );
    ed.doc_mut().set_path(None);

    run_hover(&mut ed);

    assert!(popup_lines(&ed).is_none());
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no file"),
        "expected a no-file message, got {msg:?}"
    );
}
