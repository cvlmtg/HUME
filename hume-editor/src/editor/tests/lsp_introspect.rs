// Introspection builtins: lsp-capabilities, lsp-servers,
// lsp-server-status, buffer-generation, lsp-position-params,
// lsp-primary-range-params, lsp-linewise-ranges-params,
// lsp-position->offset, lsp-range->offsets.

use super::lsp_rig::{LspRig, RigSpec};
use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_lsp::test_util::RecordingLspBackend;
use hume_scripting::ScriptingHost;

/// The JSON `rust-analyzer` receives for the params `expr` (a Scheme
/// expression over `pane`) builds: positions inside are encoded in its
/// negotiated encoding only when the request is sent.
fn sent_params(rig: &mut LspRig, expr: &str) -> serde_json::Value {
    rig.probe(&format!(
        r#"(lsp-request! pane "test/params" {expr} (lambda (err res) (begin)))"#
    ));
    let sid = rig.sid("rust-analyzer");
    rig.requests_to(sid, "test/params")
        .pop()
        .expect("the params were sent")
}

/// [`LspRig::rust`] plus one canned response for `"test/echo"`: the setup
/// every `lsp-position->offset`/`lsp-range->offsets` test below needs to
/// hand the builtin a position that carries a real producing-server tag,
/// since an untagged (hand-built) hash is rejected outright.
fn rig_with_echo(
    tmp: &std::path::Path,
    marked: &str,
    initialize_result: serde_json::Value,
    echo: serde_json::Value,
) -> LspRig {
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    backend.respond_to("test/echo", echo);
    LspRig::drained(tmp, RigSpec::rust(marked), backend)
}

/// [`run_probe`]'s async-response sibling: dispatches a `test/echo` request
/// (queued by [`rig_with_echo`]'s canned response) and
/// evaluates `assertion` (a Scheme expression referencing `bid` and `res`
/// (the echoed, now-tagged value)) once it lands, moving the cursor iff it
/// holds.
fn run_tagged_probe(ed: &mut Editor, tmp: &std::path::Path, assertion: &str) -> bool {
    run(
        ed,
        tmp,
        &format!(
            r#"(define-typed-command! "probe" "" (lambda (bid)
                 (lsp-request! bid "test/echo" (hash) (lambda (err res)
                   (when {assertion} (call! "move-right" bid))))))"#
        ),
    );
    let before = state(ed);
    type_cmd(ed, ":probe");
    ed.drain_lsp();
    ed.settle();
    state(ed) != before
}

#[test]
fn lsp_capabilities_reads_raw_wire_caps_after_handshake() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (json-ref (lsp-capabilities (car (lsp-servers bid))) "hoverProvider") #t)"#,
    );
    assert!(
        fired,
        "lsp-capabilities must return the server's raw wire capabilities"
    );
}

/// `lsp-capabilities` must read the server's raw wire JSON, not a
/// re-serialization of a typed `lsp_types::ServerCapabilities` decode: that
/// round-trip silently drops any field the pinned crate version doesn't
/// model. `documentRangeFormattingProvider.rangesSupport` (LSP 3.18) is
/// exactly such a field: `lsp_types` 0.97 has no representation for it at
/// all, so this is a regression guard on `LspClient::capabilities_json`
/// carrying the raw value through.
#[test]
fn lsp_capabilities_surfaces_a_field_lsp_types_does_not_model() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {
            "documentRangeFormattingProvider": {"rangesSupport": true}
        }}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (json-ref (lsp-capabilities (car (lsp-servers bid))) "documentRangeFormattingProvider" "rangesSupport")
                   #t)"#,
    );
    assert!(
        fired,
        "rangesSupport must survive to Steel even though lsp_types can't model it"
    );
}

#[test]
fn lsp_capabilities_is_false_before_running() {
    let tmp = safe_tempdir();
    // No `initialize` answer is scripted, so the server stays Starting.
    let (backend, _, _) = RecordingLspBackend::new();
    let mut rig = LspRig::open(tmp.path(), RigSpec::rust("-[a]>bcdef\n"), backend);

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (lsp-capabilities (car (lsp-servers bid))) #f)"#,
    );
    assert!(
        fired,
        "capabilities must be #f before the handshake completes"
    );
}

#[test]
fn lsp_server_status_lists_the_running_server() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        &format!(
            r#"(let ((entry (car (lsp-server-status))))
                 (and (equal? (hash-ref entry 'name) "rust-analyzer")
                      (equal? (hash-ref entry 'languages) '("rust"))
                      (equal? (hash-ref entry 'root) "{}")
                      (equal? (hash-ref entry 'state) 'running)
                      (equal? (hash-ref entry 'pending) 0)))"#,
            rig.root.display()
        ),
    );
    assert!(
        fired,
        "lsp-server-status must list the running server correctly"
    );
}

#[test]
fn lsp_servers_names_the_attached_server() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    rig.probe(r#"(log! 'warn (to-string (map lsp-server-name (lsp-servers pane))))"#);

    assert_eq!(rig.warnings(), vec![r#"("rust-analyzer")"#.to_string()]);
}

#[test]
fn lsp_server_registered_reflects_registration() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")"#,
        tmp.path(),
    );

    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        r#"(and (lsp-server-registered? "rust-analyzer") (not (lsp-server-registered? "rust")))"#,
    );
    assert!(
        fired,
        "lsp-server-registered? must be true for the registered name only"
    );
}

#[test]
fn lsp_server_registered_is_false_when_unregistered() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(not (lsp-server-registered? "rust-analyzer"))"#,
    );
    assert!(
        fired,
        "lsp-server-registered? must be false when nothing is registered"
    );
}

#[test]
fn buffer_generation_changes_after_an_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "snap" "" (lambda (bid) (log! 'info (to-string (buffer-generation bid)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":snap");
    let before_gen = ed
        .state
        .status_msg
        .clone()
        .expect("log! set the status message");

    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    type_cmd(&mut ed, ":snap");
    let after_gen = ed
        .state
        .status_msg
        .clone()
        .expect("log! set the status message");

    assert_ne!(
        before_gen, after_gen,
        "buffer-generation must change after a mutation"
    );
}

#[test]
fn lsp_position_params_uses_the_negotiated_utf16_encoding_for_multibyte_chars() {
    let tmp = safe_tempdir();
    // Buffer: "🎉" (char 0, one grapheme, 2 UTF-16 code units) then cursor on 'x' (char 1).
    let mut rig = LspRig::rust(
        tmp.path(),
        "🎉-[x]>rest\n",
        serde_json::json!({"capabilities": {}}),
    ); // UTF-16 default

    let params = sent_params(&mut rig, "(lsp-position-params pane)");

    assert_eq!(
        params["position"],
        serde_json::json!({"line": 0, "character": 2}),
        "UTF-16 negotiated: 🎉 is a surrogate pair, so char index 1 must be wire character 2"
    );
}

#[test]
fn lsp_position_params_uses_the_negotiated_utf8_encoding_for_multibyte_chars() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "🎉-[x]>rest\n",
        serde_json::json!({"capabilities": {"positionEncoding": "utf-8"}}),
    );

    let params = sent_params(&mut rig, "(lsp-position-params pane)");

    assert_eq!(
        params["position"]["character"], 4,
        "UTF-8 negotiated: 🎉 is 4 bytes, so char index 1 must be wire character 4"
    );
}

#[test]
fn lsp_primary_range_params_reflects_the_primary_selection() {
    let tmp = safe_tempdir();
    // Selection covers "bcd" (chars 1..=3, inclusive head at 3): half-open
    // wire range must be [1, 4).
    let mut rig = LspRig::rust(
        tmp.path(),
        "a<[bcd]-ef\n",
        serde_json::json!({"capabilities": {}}),
    );

    let params = sent_params(&mut rig, "(lsp-primary-range-params pane)");

    assert_eq!(
        params["range"],
        serde_json::json!({
            "start": {"line": 0, "character": 1},
            "end": {"line": 0, "character": 4},
        }),
        "range params must span the primary selection, half-open"
    );
}

/// A run of linewise selections that touch end-to-end (the next one starts
/// exactly where the previous one ends) coalesces into a single wire range
/// (an LSP range is naturally contiguous, so splitting a touching run
/// would buy nothing). See `lsp_linewise_ranges_params_splits_on_a_gap` for
/// the case where two selections don't touch.
#[test]
fn lsp_linewise_ranges_params_coalesces_touching_selections() {
    let tmp = safe_tempdir();
    // "line1\nline2\nline3\n": selection 1 covers line0 whole (0..=5),
    // selection 2 covers line1 whole (6..=11); they touch, so the hull is
    // one range [0, 12).
    let mut rig = LspRig::rust(
        tmp.path(),
        "-{line1\n}>-[line2\n]>line3\n",
        serde_json::json!({"capabilities": {}}),
    );

    let params = sent_params(&mut rig, "(lsp-linewise-ranges-params pane)");

    assert_eq!(
        params["ranges"],
        serde_json::json!([{
            "start": {"line": 0, "character": 0},
            "end": {"line": 2, "character": 0},
        }]),
        "two touching linewise selections must coalesce into one range"
    );
}

/// A gap between two linewise selections can't be expressed as one LSP
/// range without also covering the untouched line in between, so it stays two
/// ranges instead (see `disjoint_full_line_selections_send_two_range_
/// formatting_requests` in `tests/unix/lsp_format.rs` for the command-level
/// consequence).
#[test]
fn lsp_linewise_ranges_params_splits_on_a_gap() {
    let tmp = safe_tempdir();
    // "line1\nline2\nline3\n": selection 1 covers line0 (0..=5), selection 2
    // covers line2 (12..=17); line1 sits untouched between them.
    let mut rig = LspRig::rust(
        tmp.path(),
        "-{line1\n}>line2\n-[line3\n]>",
        serde_json::json!({"capabilities": {}}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (length (hash-ref (lsp-linewise-ranges-params bid) "ranges")) 2)"#,
    );
    assert!(
        fired,
        "a gap between two linewise selections must yield two ranges, not one hull"
    );
}

/// A selection collapsed onto an empty line reads as linewise by
/// `SelectionView::is_linewise`'s definition, but is ambiguous (see
/// `linewise_classification`), not a deliberate selection. It must not
/// bridge two real linewise selections it happens to touch on both sides
/// into one coalesced range that reformats the blank line's
/// neighbors together.
#[test]
fn lsp_linewise_ranges_params_does_not_bridge_across_a_collapsed_blank_line_selection() {
    let tmp = safe_tempdir();
    // "line1\n\nline3\n": selection 1 covers line0 whole (0..=5), selection
    // 2 is a collapsed cursor on the empty line1 (char 6, touching both
    // neighbors), selection 3 covers line2 whole (7..=12).
    let mut rig = LspRig::rust(
        tmp.path(),
        "-{line1\n}>-[\n]>-[line3\n]>",
        serde_json::json!({"capabilities": {}}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (length (hash-ref (lsp-linewise-ranges-params bid) "ranges")) 2)"#,
    );
    assert!(
        fired,
        "a collapsed selection on the blank line between two real linewise \
         selections must not bridge them into one coalesced range"
    );
}

/// A lone collapsed selection on an empty line is the sole selection:
/// ambiguous, not linewise, so it contributes no range (distinct from
/// `lsp_linewise_ranges_params_is_empty_when_nothing_is_linewise`, whose
/// selection is a genuine, unambiguous partial-line one).
#[test]
fn lsp_linewise_ranges_params_is_empty_for_a_lone_collapsed_blank_line_selection() {
    let tmp = safe_tempdir();
    // "a\n\nb\n": collapsed cursor on the empty line1 (char 2).
    let mut rig = LspRig::rust(
        tmp.path(),
        "a\n-[\n]>b\n",
        serde_json::json!({"capabilities": {}}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (hash-ref (lsp-linewise-ranges-params bid) "ranges") '())"#,
    );
    assert!(
        fired,
        "a lone collapsed selection on an empty line must yield an empty ranges list"
    );
}

/// A non-linewise selection is skipped, not an error: only the linewise
/// one among a mixed set shows up in `ranges`. `:lsp-fmt` itself treats a
/// mixed set as ambiguous and warns instead of formatting (see
/// `mixed_linewise_and_sub_line_selections_warn_and_format_nothing` in
/// `tests/unix/lsp_format.rs`); this test pins the introspection builtin's
/// own, narrower contract.
#[test]
fn lsp_linewise_ranges_params_skips_non_linewise_selections() {
    let tmp = safe_tempdir();
    // "line1\nline2\n": selection 1 covers line0 whole (0..=5, linewise),
    // selection 2 covers just "lin" on line1 (6..=8, not linewise).
    let mut rig = LspRig::rust(
        tmp.path(),
        "-{line1\n}>-[lin]>e2\n",
        serde_json::json!({"capabilities": {}}),
    );

    let params = sent_params(&mut rig, "(lsp-linewise-ranges-params pane)");

    assert_eq!(
        params["ranges"],
        serde_json::json!([{
            "start": {"line": 0, "character": 0},
            "end": {"line": 1, "character": 0},
        }]),
        "only the linewise selection must appear in ranges"
    );
}

/// No linewise selection at all still resolves (`textDocument` present):
/// `ranges` is simply empty, distinct from the no-server/no-path `#f`.
#[test]
fn lsp_linewise_ranges_params_is_empty_when_nothing_is_linewise() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "a<[bcd]-ef\n",
        serde_json::json!({"capabilities": {}}),
    );

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((p (lsp-linewise-ranges-params bid)))
             (and p
                  (hash-contains? p "textDocument")
                  (equal? (hash-ref p "ranges") '())))"#,
    );
    assert!(
        fired,
        "no linewise selection must yield an empty ranges list, not #f"
    );
}

/// The wire range's `end` must land after a full grapheme
/// cluster, never mid-cluster. `char_to_wire(rope, end_c + 1, ..)` (a raw
/// `+ 1`) would split `é` (`e` + U+0301, two chars, one cluster) if the
/// selection's inclusive `head` sits on the cluster's first char.
#[test]
fn lsp_primary_range_params_end_lands_on_a_grapheme_boundary_not_mid_cluster() {
    let tmp = safe_tempdir();
    // "caf" + é (U+0065 U+0301, two chars) + "\n". Grapheme boundaries:
    // 0,1,2,3,5,6; é occupies chars 3..5. Selection anchor=0, head=3
    // (inclusive) covers "caf" plus é's first char only.
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[c]>af\u{0065}\u{0301}\n",
        serde_json::json!({"capabilities": {}}),
    );

    select(&mut rig.ed, &[(0, 3)], 0);

    let params = sent_params(&mut rig, "(lsp-primary-range-params pane)");

    assert_eq!(
        params["range"]["end"]["character"], 5,
        "end must land after the full é cluster (char 5), not mid-cluster (char 4)"
    );
}

#[test]
fn viewport_range_matches_the_on_viewport_change_hooks_own_computation() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    // Captures the hook's own `(first . end)` payload so the assertion
    // compares two independently-reached values, not the builtin against
    // itself. Both paths share `introspect::pane_visible_range`, so this
    // pins that they stay in sync, not just that the builtin returns
    // *something*.
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define *captured* #f)
           (register-hook! 'on-viewport-change
             (lambda (bid first end) (set! *captured* (hash 'start first 'end end))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let pid = ed.state.focus.id();
    ed.queue_viewport_change(pid);
    ed.settle();

    let host = ed.scripting.take().unwrap();
    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        r#"(equal? *captured* (viewport-range bid))"#,
    );
    assert!(
        fired,
        "viewport-range must agree with the on-viewport-change hook's own \
         computation for the same pane"
    );
}

/// `viewport-range`'s `end` names one past the buffer's last *content* line,
/// never ropey's phantom line past the structural trailing `\n`: the bug
/// that made the manual's documented recipe (`user-manual/docs/plugins.md`)
/// overshoot `buffer-lines`' bounds check whenever the viewport reaches EOF.
///
/// Clamping to `ropey_line_count()` rather than `content_line_count()` would
/// report an `'end` one higher than this asserts.
#[test]
fn viewport_range_end_is_one_past_the_last_content_line_at_eof() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (hash-ref (viewport-range bid) 'end) 3)"#,
    );
    assert!(
        fired,
        "viewport-range's end must be one past the buffer's last content line (3), not one past the ropey phantom-line index (4)"
    );
}

/// A buffer taller than the pane: `end` is one past the last visible *row*
/// (`top_line + height`), not `top_line + height + 1`. `viewport_range` is
/// called directly (not through the `on-viewport-change` hook or a Steel
/// probe) so the pane's height is exactly what this test set, not whatever a
/// dispatched command might have touched.
///
/// `first_line + height + 1` would report 4 for a 3-row pane at the top of a
/// 6-line buffer, naming a row past what's on screen.
#[test]
fn viewport_range_end_is_one_past_the_last_visible_row() {
    let mut ed = editor_from("-[a]>\nb\nc\nd\ne\nf\n");
    ed.viewport_mut().height = 3;
    let t = crate::editor::commands::FocusedPane::current(&ed.state).pane();

    let got = crate::editor::lsp::introspect::viewport_range(&ed.state, &ed.view, t);
    assert_eq!(
        got,
        hume_rope::offset::ExclusiveRange::new(
            hume_rope::line::ContentLine::new(0),
            hume_rope::line::ContentLine::new(3),
        )
    );
}

/// `(viewport-range pane)` needs a pane, not just a buffer: kind-B fail-fast
/// (see `commands::CommandPane::resolve`'s doc): a pane-less handle (`(buffers)`'s
/// own return shape) raises.
#[test]
fn viewport_range_raises_for_a_paneless_buffer_handle() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");

    // `open_extra_file` opens a second buffer into the buffer list without
    // switching any pane to show it, so it stays paneless.
    let extra = tmp.path().join("hidden.rs");
    std::fs::write(&extra, "fn hidden() {}\n").unwrap();
    ed.open_extra_file(&extra);
    let hidden_bid = ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    assert_ne!(
        hidden_bid,
        ed.focused_buffer_id(),
        "test setup: the extra buffer must not be focused"
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        // Only two buffers exist, so "the one that isn't the focused
        // buffer" unambiguously picks out the hidden one; this relies on
        // `equal?`/hash (`equality_hint`) for pane comparison across
        // independently decoded `(buffers)` entries.
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (let ((hidden (car (filter (lambda (b) (not (equal? (buffer-key b) (buffer-key bid)))) (buffers)))))
               (viewport-range hidden))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":probe");
    let msg = ed
        .state
        .status_msg
        .clone()
        .expect("viewport-range on a paneless handle must raise");
    assert!(
        msg.contains("pane"),
        "error must name the missing pane; got: {msg}"
    );
}

/// A buffer shown only in a *background* tab's pane (not paneless, unlike
/// the sibling test above) still resolves: `CommandPane::resolve`
/// only checks that the pane is live and shows the buffer, not which tab
/// it's on. The returned range is trustworthy, not stale: a background
/// tab's panes are kept resynced to the terminal on every resize, same as
/// the active tab's own (see
/// `editor::tests::tab::resizing_while_a_tab_is_hidden_still_resyncs_its_viewport`).
#[test]
fn viewport_range_succeeds_for_a_buffer_shown_only_in_a_background_tab() {
    let tmp = safe_tempdir();
    let path = tmp.path().join("hidden.rs");
    std::fs::write(&path, "fn hidden() {}\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    ed.execute_typed("tabnew", Some(path.to_str().unwrap()))
        .unwrap();
    let hidden_pid = ed.state.focus.id();
    let hidden_bid = ed.focused_buffer_id();
    ed.execute_typed("tabprev", None).unwrap();
    assert_ne!(
        ed.focused_buffer_id(),
        hidden_bid,
        "test setup: back on the original tab, hidden_bid's tab now in the background"
    );

    let t = crate::editor::commands::CommandPane::resolve(
        &ed.state,
        &ed.view,
        hume_scripting::PaneHandle::with_pane(hidden_bid, hidden_pid),
    )
    .expect("a background-tab pane must still resolve");
    let got = crate::editor::lsp::introspect::viewport_range(&ed.state, &ed.view, t);
    assert_eq!(
        got.start,
        hume_rope::line::ContentLine::new(0),
        "a background-tab pane still resolves and reports its own (kept-current) geometry"
    );
}

/// The Steel-facing `(viewport-range pane)` builtin itself, not just the
/// Rust `introspect::viewport_range` function the sibling test above calls
/// directly, one layer under `host_impl`'s own (now-removed) active-tab
/// guard, must resolve a background-tab pane too.
#[test]
fn viewport_range_builtin_succeeds_for_a_background_tab_pane() {
    let tmp = safe_tempdir();
    let path = tmp.path().join("hidden.rs");
    std::fs::write(&path, "fn hidden() {}\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    ed.execute_typed("tabnew", Some(path.to_str().unwrap()))
        .unwrap();
    let hidden_bid = ed.focused_buffer_id();
    ed.execute_typed("tabprev", None).unwrap();
    assert_ne!(
        ed.focused_buffer_id(),
        hidden_bid,
        "test setup: back on the original tab, hidden_bid's tab now in the background"
    );

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        // Same "the one buffer that isn't focused" pick as the paneless
        // test above, then `(buffer-panes hidden)` resolves it to its own
        // background-tab pane before calling viewport-range on that.
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (let* ((hidden (car (filter (lambda (b) (not (equal? (buffer-key b) (buffer-key bid)))) (buffers))))
                    (hidden-pane (car (buffer-panes hidden)))
                    (range (viewport-range hidden-pane)))
               (log! 'info (string-append "range: " (number->string (hash-ref range 'start)) ".." (number->string (hash-ref range 'end)))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":probe");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("range: 0..1"),
        "viewport-range on a background-tab pane must succeed, not raise"
    );
}

/// `lsp-position-params` needs a pane, not just a buffer: kind-B fail-fast
/// (see `commands::CommandPane::resolve`'s doc): a pane-less handle (`(buffers)`'s
/// own return shape) raises, even when the buffer is attached to a running
/// server and still has a seeded (now stale) pane state.
#[test]
fn lsp_position_params_raises_for_a_paneless_buffer_handle() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    rig.ed.open_extra_file(&extra);
    let other_bid = rig
        .ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    rig.ed
        .switch_to_buffer_with_jump(FocusedPane::current(&rig.ed.state), other_bid);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut rig.ed,
        &mut host,
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (let ((hidden (car (filter (lambda (b) (and (buffer-path b) (not (equal? (buffer-key b) (buffer-key bid))))) (buffers)))))
               (lsp-position-params hidden))))"#,
        tmp.path(),
    );
    rig.ed.scripting = Some(host);

    type_cmd(&mut rig.ed, ":probe");
    let msg = rig
        .ed
        .state
        .status_msg
        .clone()
        .expect("lsp-position-params on a paneless handle must raise");
    assert!(
        msg.contains("pane"),
        "error must name the missing pane; got: {msg}"
    );
}

/// A buffer shown in a *non-focused* pane still resolves. This is the
/// inlay-hints-in-a-split path (`inlay.scm`'s refresh fires from
/// `on-viewport-change`, which fires for any pane, not just the focused
/// one), but only once the caller names that pane explicitly via
/// `(buffer-panes hidden)`; a bare buffer handle (`(buffers)`'s own shape)
/// does not resolve on its own (see the sibling `_raises_` test above).
#[test]
fn lsp_position_params_resolves_a_buffer_shown_in_a_non_focused_pane() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    rig.ed.open_extra_file(&extra);
    let other_bid = rig
        .ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    let start_pid = rig.ed.state.focus.id();
    let other_pid = open_pane_in_layout(
        &mut rig.ed.state,
        &mut rig.ed.view,
        start_pid,
        other_bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    rig.ed.state.focus.set_for_test(other_pid);

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let* ((hidden (car (filter (lambda (b) (and (buffer-path b) (not (equal? (buffer-key b) (buffer-key bid))))) (buffers))))
                  (shown (car (buffer-panes hidden))))
             (and (lsp-position-params shown) #t))"#,
    );
    assert!(
        fired,
        "lsp-position-params must still resolve a buffer shown in a non-focused pane, named via buffer-panes"
    );
}

/// `shown_buffer_state`'s resolution has no active-tab restriction: none of
/// its callers (`lsp-position-params` among them) reads a viewport, only a
/// cursor, which stays live no matter which tab is active. A buffer shown
/// only in a *background* tab's pane must still resolve via `buffer-panes`,
/// the same as one shown in a non-focused *pane* does above. The active-tab
/// restriction applies to `viewport-range`, the one caller that needs it.
#[test]
fn lsp_position_params_resolves_a_buffer_shown_only_in_a_background_tab() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );
    let tab_a = rig.ed.state.tabs.current();

    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    rig.ed
        .execute_typed("tabnew", Some(extra.to_str().unwrap()))
        .unwrap();
    let other_bid = rig.ed.focused_buffer_id();
    assert!(
        rig.ed.state.buffer_positions.lsp.has_doc(other_bid),
        "setup: the background-tab buffer is attached"
    );

    rig.ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(rig.ed.state.tabs.current(), tab_a, "setup: back on A");

    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let* ((hidden (car (filter (lambda (b) (and (buffer-path b) (not (equal? (buffer-key b) (buffer-key bid))))) (buffers))))
                  (shown (car (buffer-panes hidden))))
             (and (lsp-position-params shown) #t))"#,
    );
    assert!(
        fired,
        "lsp-position-params must still resolve a buffer shown only in a background tab, named via buffer-panes"
    );
}

#[test]
fn lsp_position_params_is_false_for_an_unattached_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    // No server attached at all.
    let host = ScriptingHost::new();
    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        r#"(equal? (lsp-position-params bid) #f)"#,
    );
    assert!(fired, "no attached server must yield #f, not an error");
}

#[test]
fn lsp_position_to_offset_uses_the_responses_tagged_utf16_encoding() {
    let tmp = safe_tempdir();
    // "🎉" is 1 char, 2 UTF-16 code units: wire character 2 (the emoji's
    // full UTF-16 width) must land on char index 1, the char right after it.
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[x]>🎉rest\n",
        serde_json::json!({"capabilities": {}}), // UTF-16 default
        serde_json::json!({"line": 0, "character": 2}),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(equal? (lsp-position->offset bid res) 1)"#,
    );
    assert!(
        fired,
        "UTF-16 negotiated: wire character 2 must land right after the emoji, at char index 1"
    );
}

#[test]
fn lsp_position_to_offset_uses_the_responses_tagged_utf8_encoding() {
    let tmp = safe_tempdir();
    // "🎉" is 4 UTF-8 bytes: wire character 4 must land on char index 1.
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[x]>🎉rest\n",
        serde_json::json!({"capabilities": {"positionEncoding": "utf-8"}}),
        serde_json::json!({"line": 0, "character": 4}),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(equal? (lsp-position->offset bid res) 1)"#,
    );
    assert!(
        fired,
        "UTF-8 negotiated: wire character 4 must land right after the 4-byte emoji, at char index 1"
    );
}

/// `lsp-position->offset` reads the position's own tagged producing-server
/// encoding: an untagged (hand-built) hash must error, not silently
/// resolve via `bid`'s currently attached server.
///
/// Without `JsonHandle::position_encoding`'s `Err`, this would silently
/// decode against the running UTF-16 server.
#[test]
fn lsp_position_to_offset_untagged_handle_errors() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    run(
        &mut rig.ed,
        tmp.path(),
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (lsp-position->offset bid (hash "line" 0 "character" 0))))"#,
    );
    type_cmd(&mut rig.ed, ":probe");

    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        log.contains("not a value from an LSP server"),
        "an untagged position must error, not silently decode via bid's attached server: {log:?}"
    );
}

/// The tagged counterpart of the test above: a *tagged* position
/// decodes correctly even when `bid` currently has no server attached at
/// all: the encoding travels with the response, not with `bid`'s live
/// attachment. The request is dispatched (and its response tagged) while
/// the server is still attached; `bid`'s document is closed before the
/// response is drained (a second open file keeps the instance alive), so
/// only the tag remains by the time `lsp-position->offset` actually runs,
/// mirroring
/// `lsp_request_with_no_attached_server_reports_an_error_and_fires_callback_with_err`'s
/// own detach-after-send shape.
#[test]
fn lsp_position_to_offset_decodes_via_the_tag_even_after_the_server_detaches() {
    let tmp = safe_tempdir();
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[x]>🎉rest\n",
        serde_json::json!({"capabilities": {}}), // UTF-16
        serde_json::json!({"line": 0, "character": 2}),
    );

    run(
        &mut rig.ed,
        tmp.path(),
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (lsp-request! bid "test/echo" (hash) (lambda (err res)
               (when (equal? (lsp-position->offset bid res) 1)
                 (call! "move-right" bid))))))
           (define-typed-command! "detach" "" (lambda (bid)
             (set-buffer-option! bid "language" "")))"#,
    );
    let other = rig.root.join("src/other.rs");
    std::fs::write(&other, "fn other() {}\n").unwrap();
    rig.ed.open_extra_file(&other);
    let before = state(&rig.ed);
    type_cmd(&mut rig.ed, ":probe");
    // The request is already in flight; detaching now proves the later
    // decode reads the response's own tag, not bid's live attachment.
    // `other.rs` keeps the server running, so the response still arrives.
    type_cmd(&mut rig.ed, ":detach");
    rig.ed.drain_lsp();
    rig.ed.settle();

    assert_ne!(
        state(&rig.ed),
        before,
        "a tagged position must still decode correctly with no server attached"
    );
    rig.probe(r#"(log! 'warn (to-string (lsp-servers pane)))"#);
    assert_eq!(rig.warnings(), vec!["()".to_string()], "bid was detached");
}

#[test]
fn lsp_position_to_offset_is_false_when_it_would_land_on_the_trailing_phantom_line() {
    let tmp = safe_tempdir();
    // "-[x]>abc\n" is "xabc\n": one content line; a wire `line` past it
    // clamps (inside `wire_to_char`) onto the buffer's trailing phantom line
    // rather than erroring, since servers send past-end positions routinely. Every
    // point-anchored decoration setter (`set-inlay-hints!`) rejects that
    // offset outright, so `lsp-position->offset` must refuse here too,
    // rather than handing back a value only useful for failing one step
    // later (and, inside a hint batch, failing every *other* hint with it).
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[x]>abc\n",
        serde_json::json!({"capabilities": {}}),
        serde_json::json!({"line": 5, "character": 0}),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(equal? (lsp-position->offset bid res) #f)"#,
    );
    assert!(
        fired,
        "a wire line past the buffer's content must yield #f, not the phantom line's offset"
    );
}

#[test]
fn lsp_range_to_offsets_converts_both_endpoints_half_open() {
    let tmp = safe_tempdir();
    // "🎉" occupies char 0 (2 UTF-16 code units); 'b' is char 1, wire
    // character 2. A wire range [0, 2) must convert to char offsets (0 . 1),
    // covering just the emoji, half-open.
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[x]>🎉bcdef\n",
        serde_json::json!({"capabilities": {}}), // UTF-16 default
        serde_json::json!({
            "start": {"line": 0, "character": 0},
            "end": {"line": 0, "character": 2},
        }),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(equal? (lsp-range->offsets bid res) (hash 'start 0 'end 1))"#,
    );
    assert!(
        fired,
        "wire range [0, 2) (UTF-16) must convert to half-open char offsets (0 . 1)"
    );
}

#[test]
fn lsp_range_to_offsets_end_may_land_at_the_buffers_char_length() {
    // The opposite of `lsp_position_to_offset_is_false_when_it_
    // would_land_on_the_trailing_phantom_line`: a range's `end` legitimately
    // sits at the buffer's char length (`set-extra-highlights!`'s
    // `validate_range` accepts that boundary), so `lsp-range->offsets` must
    // keep the clamping behavior `lsp-position->offset`
    // refuses: a past-end wire `line` for `end` is not an error here.
    // "-[x]>abc\n" is "xabc\n" (the marked 'x' is real buffer content), 5
    // chars.
    let tmp = safe_tempdir();
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[x]>abc\n",
        serde_json::json!({"capabilities": {}}),
        serde_json::json!({
            "start": {"line": 0, "character": 0},
            "end": {"line": 5, "character": 0},
        }),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(equal? (lsp-range->offsets bid res) (hash 'start 0 'end 5))"#,
    );
    assert!(
        fired,
        "a past-end wire `line` for `end` must clamp to the buffer's char length, not #f"
    );
}

/// `lsp-range->offsets`'s own untagged-handle counterpart to
/// `lsp_position_to_offset_untagged_handle_errors`.
#[test]
fn lsp_range_to_offsets_untagged_handle_errors() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    run(
        &mut rig.ed,
        tmp.path(),
        r#"(define-typed-command! "probe" "" (lambda (bid)
             (lsp-range->offsets bid
               (hash "start" (hash "line" 0 "character" 0)
                     "end" (hash "line" 0 "character" 1)))))"#,
    );
    type_cmd(&mut rig.ed, ":probe");

    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        log.contains("not a value from an LSP server"),
        "an untagged range must error, not silently decode via bid's attached server: {log:?}"
    );
}

/// `lsp-locations->display-parts` reads each location's own tagged
/// producing-server encoding: an untagged (hand-built) handle must error,
/// not silently resolve to the UTF-16 default.
///
/// A guessed UTF-16 fallback in `JsonHandle::position_encoding` would return
/// `Ok(vec![...])` for a well-formed but untagged location.
#[test]
fn lsp_locations_display_parts_untagged_handle_errors() {
    let ed = editor_from("-[a]>bcdef\n");
    let handle = hume_scripting::json::JsonHandle::new(serde_json::json!({
        "uri": "file:///a.rs",
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
    }));

    let err = crate::editor::lsp::introspect::location_display_parts(&ed.state, &[handle])
        .expect_err("an untagged handle must error, not silently guess an encoding");
    assert!(
        err.contains("not a value from an LSP server"),
        "error must name the actual problem (no server tag); got: {err}"
    );
}

/// Each display row names the open buffer its location is in, or `#f` when
/// the file is not open, so a script never matches URIs to buffers itself.
#[test]
fn lsp_locations_display_parts_name_the_open_buffer_of_each_row() {
    let tmp = safe_tempdir();
    let file = std::fs::canonicalize(tmp.path())
        .unwrap()
        .join("src/main.rs");
    let uri = hume_lsp::uri::path_to_uri(&file)
        .unwrap()
        .as_str()
        .to_string();
    let loc = |uri: &str, line: u64| {
        serde_json::json!({
            "uri": uri,
            "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 0}},
        })
    };
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[a]>bc\ndef\n",
        serde_json::json!({"capabilities": {}}),
        serde_json::json!([loc(&uri, 1), loc("file:///nowhere/other.rs", 0)]),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(let ((parts (lsp-locations->display-parts (json-list res))))
             (and (> (buffer-line-count (hash-ref (car parts) 'buffer)) 0)
                  (equal? (hash-ref (cadr parts) 'buffer) #f)))"#,
    );
    assert!(fired);
}

fn echo_location(line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": "file:///nowhere/other.rs",
        "range": {
            "start": {"line": line, "character": character},
            "end": {"line": line, "character": character},
        },
    })
}

/// Two servers often answer with the same location: rows naming the same
/// path, line and column collapse into the first.
#[test]
fn display_parts_dedupes_identical_rows_keeping_the_first() {
    let tmp = safe_tempdir();
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[a]>bc\n",
        serde_json::json!({"capabilities": {}}),
        serde_json::json!([
            echo_location(3, 1),
            echo_location(4, 0),
            echo_location(3, 1)
        ]),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(equal? (map (lambda (p) (hash-ref p 'line))
                        (lsp-locations->display-parts (json-list res)))
                   '(3 4))"#,
    );
    assert!(fired);
}

/// Each row carries the location it was decoded from, so a caller jumps to
/// the row it shows rather than indexing a list the dedupe reshaped.
#[test]
fn display_parts_rows_carry_their_location() {
    let tmp = safe_tempdir();
    let mut rig = rig_with_echo(
        tmp.path(),
        "-[a]>bc\n",
        serde_json::json!({"capabilities": {}}),
        serde_json::json!([
            echo_location(3, 1),
            echo_location(3, 1),
            echo_location(7, 2)
        ]),
    );

    let fired = run_tagged_probe(
        &mut rig.ed,
        tmp.path(),
        r#"(let ((parts (lsp-locations->display-parts (json-list res))))
             (equal? (map (lambda (p) (json-ref (hash-ref p 'location) "range" "start" "line")) parts)
                     '(3 7)))"#,
    );
    assert!(fired);
}

// ── track-position! / tracked-position-params / untrack-position! ────────────

const TRACKING_COMMANDS: &str = r#"
(define *tok* #f)
(define-typed-command! "arm" "" (lambda (bid) (set! *tok* (track-position! bid))))
(define-typed-command! "disarm" "" (lambda (bid) (untrack-position! *tok*)))
(define-typed-command! "report" ""
  (lambda (bid)
    (let ((p (tracked-position-params *tok*)))
      (if p
          (lsp-request! bid "test/params" p (lambda (err res) (begin)))
          (log! 'info "none")))))
"#;

fn tracked_rig(tmp: &std::path::Path) -> LspRig {
    let mut rig = LspRig::rust(
        tmp,
        "abc\nd-[e]>f\n",
        serde_json::json!({"capabilities": {}}),
    );
    install_source(&mut rig.ed, ScriptingHost::new(), TRACKING_COMMANDS, tmp);
    type_cmd(&mut rig.ed, ":arm");
    rig
}

/// The line of the position the last `:report` sent.
fn reported_line(rig: &LspRig) -> serde_json::Value {
    let sid = rig.sid("rust-analyzer");
    rig.requests_to(sid, "test/params")
        .pop()
        .expect(":report sent the params")["position"]["line"]
        .clone()
}

/// The params name the tracked symbol's line after lines were inserted above
/// it, wherever the cursor is.
#[test]
fn tracked_position_params_follow_lines_inserted_above() {
    let tmp = safe_tempdir();
    let mut rig = tracked_rig(tmp.path());
    type_cmd(&mut rig.ed, ":report");
    assert_eq!(reported_line(&rig), 1, "setup: line 1");

    set_cursor(&mut rig.ed, 0);
    rig.ed.handle_key(key('O'));
    rig.ed.handle_key(key_esc());
    assert_eq!(rig.ed.doc().text().to_string(), "\nabc\ndef\n");

    type_cmd(&mut rig.ed, ":report");
    assert_eq!(reported_line(&rig), 2);
}

#[test]
fn a_released_position_answers_false() {
    let tmp = safe_tempdir();
    let mut rig = tracked_rig(tmp.path());
    type_cmd(&mut rig.ed, ":disarm");

    type_cmd(&mut rig.ed, ":report");
    assert_eq!(rig.ed.state.status_msg.as_deref(), Some("none"));
}

#[test]
fn a_false_token_answers_false_and_never_raises() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bc\n",
        serde_json::json!({"capabilities": {}}),
    );
    let fired = run_probe(
        &mut rig.ed,
        ScriptingHost::new(),
        tmp.path(),
        "(and (equal? (tracked-position-params #f) #f) (begin (untrack-position! #f) #t))",
    );
    assert!(fired);
}

#[test]
fn lsp_capability_reads_the_provider_of_a_feature_or_a_method() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {
            "hoverProvider": true,
            "completionProvider": {"triggerCharacters": ["."]},
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": {"rangesSupport": true},
            "renameProvider": false,
        }}),
    );

    rig.probe(
        r#"(let ((s (car (lsp-servers pane))))
             (log! 'warn (to-string (lsp-capability s #:feature 'hover)))
             (log! 'warn (to-string (json-list (json-ref (lsp-capability s #:feature 'completion) "triggerCharacters"))))
             (log! 'warn (to-string (json-ref (lsp-capability s #:method "textDocument/rangeFormatting") "rangesSupport")))
             (log! 'warn (to-string (lsp-capability s #:method "textDocument/formatting")))
             (log! 'warn (to-string (lsp-capability s #:feature 'rename-symbol)))
             (log! 'warn (to-string (lsp-capability s #:method "custom/method"))))"#,
    );

    assert_eq!(
        rig.warnings(),
        vec!["#true", r#"(".")"#, "#true", "#true", "#false", "#false"]
    );
}

#[test]
fn lsp_capability_needs_exactly_one_of_feature_and_method() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[a]>bcdef\n",
        serde_json::json!({"capabilities": {}}),
    );

    rig.probe(
        r#"(let ((s (car (lsp-servers pane))))
             (log! 'warn (to-string (with-handler (lambda (e) 'raised) (lsp-capability s))))
             (log! 'warn (to-string (with-handler (lambda (e) 'raised)
               (lsp-capability s #:feature 'hover #:method "textDocument/hover")))))"#,
    );

    assert_eq!(rig.warnings(), vec!["raised", "raised"]);
}
