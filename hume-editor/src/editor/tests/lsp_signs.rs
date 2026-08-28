// Plugin gutter signs (`set-signs!`): the `update_sign_providers` write side
// that feeds `SharedSignSource` from the signs store, plus the sign
// column's priority ladder and auto-collapsing width. Diagnostic signs
// (`core:lsp`'s own `set-signs!` calls) are covered by
// `tests/unix/lsp_diagnostic_signs.rs` — diagnostics are an ordinary plugin
// sign source now, not a separate Rust-side render path.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s bare
// `Pane::new`) — sign providers are only registered by `build_pane`, same
// reasoning as `lsp_render.rs`.

use super::*;
use hume_engine::builtins::sign_column::Sign;
use hume_engine::pipeline::{PaneId, RenderContext};

fn pane_signs(ed: &Editor, pid: PaneId) -> rustc_hash::FxHashMap<usize, Vec<Sign>> {
    ed.state.panes.render[pid].signs.read().unwrap().clone()
}

fn sign_column_width(ed: &Editor, pid: PaneId) -> u8 {
    ed.view.panes[pid]
        .providers
        .gutter_columns()
        .next()
        .expect("sign column registered first")
        .width(0)
}

/// `RenderContext::new` + `sync_viewport_dims(80, 25)` + `settle` +
/// `prepare_frame` — every sign test's own frame-drive step, differing only
/// in what's armed beforehand.
fn render(ed: &mut Editor) {
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
}

/// Builds an untitled editor containing `"abcdefgh\n"`, arms `arm_body` as a
/// Steel `"arm"` command, runs it, pins `signcolumn` if given, and renders
/// one frame — the harness every plugin-sign test below needs, differing
/// only in what `set-signs!` calls `arm_body` makes and whether the column
/// is pinned.
fn plugin_sign_editor(signcolumn: Option<&str>, arm_body: &str) -> (Editor, PaneId) {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "abcdefgh");
    if let Some(signcolumn) = signcolumn {
        let bid = ed.focused_buffer_id();
        ed.state.buffers.get_mut(bid).overrides.signcolumn = Some(signcolumn.parse().unwrap());
    }
    let source = format!(r#"(define-command! "arm" "" (lambda () {arm_body}))"#);
    run(&mut ed, tmp.path(), &source);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    render(&mut ed);
    (ed, pid)
}

// ── Gutter width ──────────────────────────────────────────────────────────────

#[test]
fn gutter_width_stays_at_default_with_no_signs_under_always_mode() {
    let (ed, pid) = plugin_sign_editor(None, "(+ 1 0)");
    assert_eq!(
        sign_column_width(&ed, pid),
        2,
        "default is `always` — column stays visible even with no signs"
    );
}

#[test]
fn gutter_width_collapses_under_auto_mode_with_no_signs() {
    let (ed, pid) = plugin_sign_editor(Some("auto"), "(+ 1 0)");
    assert_eq!(
        sign_column_width(&ed, pid),
        0,
        "auto mode with no signs — column collapses"
    );
}

#[test]
fn gutter_width_always_2_is_3_cells_wide() {
    let (ed, pid) = plugin_sign_editor(Some("always:2"), "(+ 1 0)");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

// ── Plugin signs ──────────────────────────────────────────────────────────────

#[test]
fn plugin_sign_via_set_signs_appears_in_the_plugin_map() {
    let (ed, pid) = plugin_sign_editor(
        None,
        r#"(set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope" 7)))"#,
    );

    let signs = pane_signs(&ed, pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the line");
    assert_eq!(sign.text, "!");
    assert_eq!(
        sign.slot, 0,
        "this plugin sign is the buffer's only sign channel — slot 0"
    );
    let warn_scope = ed.view.registry.get("warn-scope").unwrap();
    assert_eq!(sign.scope, warn_scope);

    assert_eq!(
        sign_column_width(&ed, pid),
        2,
        "a plugin sign alone must also expand the gutter"
    );
}

/// With no `signcolumn` override, `always` auto-sizes to the buffer's
/// live sign-priority ladder — two plugin sources at distinct priorities
/// on the same line both claim their own slot, ordered highest-priority
/// first, without the user having to pin `always:2` for it. A channel's
/// column position is a property of its priority, stable buffer-wide, not
/// a function of what else happens to share the line.
#[test]
fn default_signcolumn_auto_sizes_to_show_every_channel_present() {
    let (ed, pid) = plugin_sign_editor(
        None,
        r#"(set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))"#,
    );

    let signs = pane_signs(&ed, pid);
    assert_eq!(signs.len(), 1, "one line, one merged entry across sources");
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "two distinct priorities on the ladder — both get their own slot, unpinned"
    );
    assert_eq!(line_signs[0].text, "+", "priority 9 (vcs) — slot 0");
    assert_eq!(line_signs[1].text, "!", "priority 3 (linter) — slot 1");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "auto-sized to 2 slots + 1 padding"
    );
}

/// Bare `auto` (no `:N`) auto-sizes to the live ladder exactly like bare
/// `always` — `auto`'s only distinct behavior is collapsing to zero width
/// when no signs are visible at all (see
/// `gutter_width_collapses_under_auto_mode_with_no_signs`), which this test
/// doesn't exercise since both channels here have live signs.
#[test]
fn bare_auto_auto_sizes_to_multiple_channels_like_bare_always() {
    let (ed, pid) = plugin_sign_editor(
        Some("auto"),
        r#"(set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "two distinct priorities — auto grows past its 1-slot floor, same as bare always"
    );
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "auto-sized to 2 slots + 1 padding, same width bare always resolves to"
    );
}

/// Auto-sizing (bare `always`/`auto`) never grows past
/// `MAX_AUTO_SIGN_SLOTS` — a channel ranked below that cap gets no slot at
/// all, buffer-wide, not just on lines where the higher-priority channels
/// are also present.
#[test]
fn auto_size_cap_hides_the_lowest_priority_channel_buffer_wide() {
    let (ed, pid) = plugin_sign_editor(
        None,
        r#"(set-signs! "a" (current-buffer) (list (list 0 "5" "sc" 5)))
           (set-signs! "b" (current-buffer) (list (list 0 "4" "sc" 4)))
           (set-signs! "c" (current-buffer) (list (list 0 "3" "sc" 3)))
           (set-signs! "d" (current-buffer) (list (list 0 "2" "sc" 2)))
           (set-signs! "e" (current-buffer) (list (list 0 "1" "sc" 1)))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        4,
        "5 distinct priorities registered, but the auto-size cap admits only the top 4"
    );
    assert!(
        line_signs.iter().all(|s| s.text != "1"),
        "priority 1 (ranked 5th) has no slot at all — it isn't merely dropped from this line"
    );
    assert_eq!(
        sign_column_width(&ed, pid),
        5,
        "4 slots + 1 padding, capped regardless of how many distinct priorities exist"
    );
}

/// Pinning `always:1` caps the column at one slot regardless of how many
/// distinct priorities the ladder holds — the lower-priority channel is
/// hidden buffer-wide, not just squeezed off this one line.
#[test]
fn pinned_single_slot_keeps_only_the_higher_priority_sign() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:1"),
        r#"(set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        1,
        "always:1 pins exactly one slot — the other priority's slot doesn't fit"
    );
    assert_eq!(
        line_signs[0].text, "+",
        "priority 9 (vcs) beats priority 3 (linter) for the one slot"
    );
}

/// Priority *is* the slot: two plugin sources at the *same* priority resolve
/// to the *same* slot and contend for it, even with `always:2` pinned — the
/// second slot goes unclaimed rather than absorbing the loser, because
/// nothing else on this line asked for it. The tie itself still resolves
/// deterministically by source name (ascending), not by call order:
/// `signs_for_buffer` (`SourceStore::for_buffer`) yields entries ascending
/// by source name, and the plugin merge (`update_sign_providers`) keeps the
/// first entry per slot — so `"linter"` wins even though `"vcs"` is armed
/// first here.
#[test]
fn two_plugin_sources_at_equal_priority_contend_for_one_slot() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:2"),
        r#"(set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 5)))
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 5)))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        1,
        "both signs share one priority — one slot, one winner, even at width 2"
    );
    assert_eq!(
        line_signs[0].text, "!",
        "equal priority — \"linter\" wins the tie by source name (alphabetically \
         first), even though \"vcs\" was armed first"
    );
    assert_eq!(
        line_signs[0].slot, 0,
        "priority 5 is the buffer's only channel — slot 0"
    );
}

/// With `signcolumn=always:2` pinned, both distinct-priority sources fit
/// their own slot regardless of the ladder's actual length — same outcome
/// as auto-size here (2 live priorities), but via the pinned path instead
/// of `SignColumnConfig::slots_for`'s ladder-length fallback.
#[test]
fn wider_signcolumn_keeps_multiple_signs_per_line() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:2"),
        r#"(set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "signcolumn=always:2 keeps both signs on the line"
    );
    assert_eq!(line_signs[0].text, "+", "priority 9 first");
    assert_eq!(line_signs[1].text, "!", "priority 3 second");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

/// Regression for the ladder's slot index truncating to `u8` before it was
/// bounded to the resolved slot count: a priority ranked at index 256 (or
/// any multiple of 256) on the *un*-truncated ladder wrapped to slot 0 via
/// `256 as u8`, silently contending with the buffer's actual
/// highest-priority sign. 260 distinct priorities push a pinned
/// `always:127` ladder well past that boundary.
#[test]
fn priority_ranked_past_255_never_lands_in_slot_zero() {
    let entries: String = (0..260)
        .map(|i| format!(r#"(list 0 "p{i}" "sc" {i})"#))
        .collect::<Vec<_>>()
        .join(" ");
    let arm_body = format!(r#"(set-signs! "flood" (current-buffer) (list {entries}))"#);
    let (ed, pid) = plugin_sign_editor(Some("always:127"), &arm_body);

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        127,
        "pinned always:127 keeps exactly 127 of the 260 distinct priorities"
    );
    assert_eq!(
        line_signs[0].text, "p259",
        "the highest priority (259) must resolve to slot 0"
    );
    assert!(
        line_signs.iter().all(|s| s.text != "p3"),
        "priority 3 (ranked 257th, past the 127-slot cutoff) must not appear \
         anywhere — its un-truncated rank of 256 would wrap to slot 0 via \
         `as u8` and silently displace the true top priority"
    );
}
