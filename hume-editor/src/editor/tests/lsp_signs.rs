// Plugin gutter signs (`set-signs!`/`register-sign-source!`): the
// `update_sign_providers` write side that feeds `SharedSignSource` from the
// signs store, plus the sign-source registry (a source's gutter slot is its
// rank in `DecorationStores::sign_sources`, not anything carried on a
// `set-signs!` entry) and the auto-collapsing width. Diagnostic signs
// (`core:lsp`'s own `set-signs!` calls) are an ordinary plugin sign source,
// covered by `tests/unix/lsp_diagnostic_signs.rs`.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s bare
// `Pane::new`) — sign providers are only registered by `build_pane`, same
// reasoning as `lsp_render.rs`.

use super::*;

/// Builds an untitled editor containing `"abcdefgh\n"`, arms `arm_body` as a
/// Steel `"arm"` command, runs it, pins `signcolumn` if given, and renders
/// one frame — the harness every plugin-sign test below needs, differing
/// only in what `arm_body` does (typically a `register-sign-source!` call
/// per source, then a `set-signs!` per source) and whether the column is
/// pinned.
fn plugin_sign_editor(signcolumn: Option<&str>, arm_body: &str) -> (Editor, PaneId) {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "abcdefgh");
    if let Some(signcolumn) = signcolumn {
        let bid = ed.focused_buffer_id();
        ed.state.buffers.get_mut(bid).overrides.signcolumn = Some(signcolumn.parse().unwrap());
    }
    let source = format!(r#"(define-typed-command! "arm" "" (lambda () {arm_body}))"#);
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

// ── Sign source registration ────────────────────────────────────────────────

#[test]
fn set_signs_for_an_unregistered_source_errors_naming_the_builtin() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "abcdefgh");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda ()
             (set-signs! "nope" (current-buffer) (list (list 0 "!" "sc")))))"#,
    );
    type_cmd(&mut ed, ":arm");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("set-signs!") && log.contains("nope"),
        "must name the builtin and the unregistered source: {log:?}"
    );
}

/// A registered source reserves its slot the moment it registers, so the
/// gutter width never moves as that source's (or another registered
/// source's) signs come and go.
#[test]
fn registered_sources_keep_the_gutter_width_stable_as_signs_come_and_go() {
    let (mut ed, pid) = plugin_sign_editor(
        None,
        r#"(register-sign-source! "a" (current-buffer) 2)
           (register-sign-source! "b" (current-buffer) 1)
           (set-signs! "a" (current-buffer) (list (list 0 "+" "sc")))"#,
    );
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "two registered sources — 2 slots + 1 padding, whether or not \"b\" \
         has actually placed a sign yet"
    );

    let tmp = safe_tempdir();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm-b" "" (lambda ()
             (set-signs! "b" (current-buffer) (list (list 0 "-" "sc")))))"#,
    );
    type_cmd(&mut ed, ":arm-b");
    render(&mut ed);
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "\"b\" placing a sign must not change the width — its slot was \
         already reserved by registration"
    );

    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "clear-b" "" (lambda ()
             (set-signs! "b" (current-buffer) '())))"#,
    );
    type_cmd(&mut ed, ":clear-b");
    render(&mut ed);
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "\"b\" clearing its sign must not shrink the width either"
    );
}

#[test]
fn re_registering_a_sign_source_updates_its_priority_and_slot() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:2"),
        r#"(register-sign-source! "a" (current-buffer) 1)
           (register-sign-source! "b" (current-buffer) 2)
           (set-signs! "a" (current-buffer) (list (list 0 "A" "sc")))
           (set-signs! "b" (current-buffer) (list (list 0 "B" "sc")))
           (register-sign-source! "a" (current-buffer) 10)"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        &*line_signs[0].text, "A",
        "re-registering \"a\" at priority 10 must move it ahead of \"b\" — \
         slot 0 now, even though \"a\" registered first and \"b\" had the \
         higher priority when it placed its sign"
    );
    assert_eq!(&*line_signs[1].text, "B");
}

// ── Plugin signs ──────────────────────────────────────────────────────────────

#[test]
fn plugin_sign_via_set_signs_appears_in_the_plugin_map() {
    let (ed, pid) = plugin_sign_editor(
        None,
        r#"(register-sign-source! "linter" (current-buffer) 7)
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope")))"#,
    );

    let signs = pane_signs(&ed, pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the line");
    assert_eq!(&*sign.text, "!");
    assert_eq!(
        sign.slot, 0,
        "this plugin sign is the buffer's only registered channel — slot 0"
    );
    let warn_scope = scope(&ed, "warn-scope");
    assert_eq!(sign.scope, warn_scope);

    assert_eq!(
        sign_column_width(&ed, pid),
        2,
        "a plugin sign alone must also expand the gutter"
    );
}

/// With no `signcolumn` override, `always` auto-sizes to however many sign
/// sources are registered — two sources at distinct priorities on the same
/// line both claim their own slot, ordered highest-priority first, without
/// the user having to pin `always:2` for it. A channel's column position is
/// a property of its registration, stable buffer-wide, not a function of
/// what else happens to share the line.
#[test]
fn default_signcolumn_auto_sizes_to_show_every_channel_present() {
    let (ed, pid) = plugin_sign_editor(
        None,
        r#"(register-sign-source! "linter" (current-buffer) 3)
           (register-sign-source! "vcs" (current-buffer) 9)
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "a")))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b")))"#,
    );

    let signs = pane_signs(&ed, pid);
    assert_eq!(signs.len(), 1, "one line, one merged entry across sources");
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "two registered sources — both get their own slot, unpinned"
    );
    assert_eq!(&*line_signs[0].text, "+", "priority 9 (vcs) — slot 0");
    assert_eq!(&*line_signs[1].text, "!", "priority 3 (linter) — slot 1");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "auto-sized to 2 slots + 1 padding"
    );
}

/// Bare `auto` (no `:N`) auto-sizes to the registered sources exactly like
/// bare `always` — `auto`'s only distinct behavior is collapsing to zero
/// width when no signs are visible at all (see
/// `gutter_width_collapses_under_auto_mode_with_no_signs`), which this test
/// doesn't exercise since both channels here have live signs.
#[test]
fn bare_auto_auto_sizes_to_multiple_channels_like_bare_always() {
    let (ed, pid) = plugin_sign_editor(
        Some("auto"),
        r#"(register-sign-source! "linter" (current-buffer) 3)
           (register-sign-source! "vcs" (current-buffer) 9)
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "a")))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b")))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "two registered sources — auto grows past its 1-slot floor, same as bare always"
    );
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "auto-sized to 2 slots + 1 padding, same width bare always resolves to"
    );
}

/// Auto-sizing follows the registered-source count, uncapped below
/// `SignColumnConfig::MAX_SLOTS` (127) — five registered sources auto-size
/// to five slots.
#[test]
fn auto_size_grows_to_five_registered_sources() {
    let (ed, pid) = plugin_sign_editor(
        None,
        r#"(register-sign-source! "a" (current-buffer) 5)
           (register-sign-source! "b" (current-buffer) 4)
           (register-sign-source! "c" (current-buffer) 3)
           (register-sign-source! "d" (current-buffer) 2)
           (register-sign-source! "e" (current-buffer) 1)
           (set-signs! "a" (current-buffer) (list (list 0 "5" "sc")))
           (set-signs! "b" (current-buffer) (list (list 0 "4" "sc")))
           (set-signs! "c" (current-buffer) (list (list 0 "3" "sc")))
           (set-signs! "d" (current-buffer) (list (list 0 "2" "sc")))
           (set-signs! "e" (current-buffer) (list (list 0 "1" "sc")))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        5,
        "five registered sources — all five get their own slot"
    );
    assert_eq!(
        sign_column_width(&ed, pid),
        6,
        "5 slots + 1 padding, unbounded by anything but MAX_SLOTS (127)"
    );
}

/// Pinning `always:1` caps the column at one slot regardless of how many
/// sources are registered — the lower-priority channel is hidden
/// buffer-wide, not just squeezed off this one line.
#[test]
fn pinned_single_slot_keeps_only_the_higher_priority_sign() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:1"),
        r#"(register-sign-source! "linter" (current-buffer) 3)
           (register-sign-source! "vcs" (current-buffer) 9)
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "a")))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b")))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        1,
        "always:1 pins exactly one slot — the other source's slot doesn't fit"
    );
    assert_eq!(
        &*line_signs[0].text, "+",
        "priority 9 (vcs) beats priority 3 (linter) for the one slot"
    );
}

/// A registered source's slot is a property of *registration*, so two
/// sources at the *same* declared priority don't contend for one slot —
/// both register their own distinct slot, ties broken by name (ascending)
/// at registration time.
#[test]
fn equal_priority_sign_sources_get_distinct_slots_ordered_by_name() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:2"),
        r#"(register-sign-source! "vcs" (current-buffer) 5)
           (register-sign-source! "linter" (current-buffer) 5)
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b")))
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "a")))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "equal priority — both sources still get their own slot"
    );
    assert_eq!(
        &*line_signs[0].text, "!",
        "equal priority ties break by name at registration — \"linter\" \
         (alphabetically first) ranks slot 0, even though \"vcs\" registered first"
    );
    assert_eq!(&*line_signs[1].text, "+");
}

/// With `signcolumn=always:2` pinned, both distinct-priority sources fit
/// their own slot regardless of how many sources are registered — same
/// outcome as auto-size here (2 registered sources), but via the pinned
/// path instead of `SignColumnConfig::slots_for`'s source-count fallback.
#[test]
fn wider_signcolumn_keeps_multiple_signs_per_line() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:2"),
        r#"(register-sign-source! "linter" (current-buffer) 3)
           (register-sign-source! "vcs" (current-buffer) 9)
           (set-signs! "linter" (current-buffer) (list (list 0 "!" "a")))
           (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b")))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "signcolumn=always:2 keeps both signs on the line"
    );
    assert_eq!(&*line_signs[0].text, "+", "priority 9 first");
    assert_eq!(&*line_signs[1].text, "!", "priority 3 second");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

/// A source ranked past the resolved slot count is hidden entirely, not
/// miscast into slot 0 — the slot index is bounds-checked against the
/// resolved slot count (`slot >= slots`) while it's still a plain `usize`
/// registry rank, strictly before the `as u8` narrowing
/// `update_sign_providers` needs for `Sign::slot`, so a rank that doesn't
/// fit can never silently wrap into a slot that does.
#[test]
fn a_source_ranked_past_the_resolved_slot_count_is_hidden_not_miscast_into_slot_zero() {
    let (ed, pid) = plugin_sign_editor(
        Some("always:2"),
        r#"(register-sign-source! "a" (current-buffer) 3)
           (register-sign-source! "b" (current-buffer) 2)
           (register-sign-source! "c" (current-buffer) 1)
           (set-signs! "a" (current-buffer) (list (list 0 "3" "sc")))
           (set-signs! "b" (current-buffer) (list (list 0 "2" "sc")))
           (set-signs! "c" (current-buffer) (list (list 0 "1" "sc")))"#,
    );

    let signs = pane_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "always:2 pins exactly two slots — the third registered source (rank 2) doesn't fit"
    );
    assert_eq!(&*line_signs[0].text, "3", "rank 0 (highest priority)");
    assert_eq!(&*line_signs[1].text, "2", "rank 1");
    assert!(
        line_signs.iter().all(|s| &*s.text != "1"),
        "\"c\" (rank 2, past the always:2 cutoff) must not appear anywhere — \
         not wrapped into slot 0"
    );
}
