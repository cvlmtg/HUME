use super::*;

// ── Bundled theme loading (end-to-end wiring) ─────────────────────────────────

/// Load every bundled theme through the same loader path production code
/// uses. No `bake()`: every check below reads scopes via `resolve_by_name`,
/// which walks the raw dot-notation map directly (same as
/// `EditorColors::from_theme` at render time) — baking only feeds the
/// ID-based `resolve()` fast path, which none of these tests exercise.
fn load_bundled_themes() -> Vec<(String, hume_engine::theme::Theme)> {
    use std::path::PathBuf;
    let themes_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes");
    let paths = vec![themes_dir.clone()];

    // Read the directory rather than listing the names: it is already the
    // source of truth for `:theme <Tab>`, and a hand-written list here means a
    // newly bundled theme silently skips every check in this file.
    let mut names: Vec<String> = std::fs::read_dir(&themes_dir)
        .expect("runtime/themes must be readable")
        .map(|e| e.expect("runtime/themes entry must be readable").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .map(|p| {
            p.file_stem()
                .expect("a *.toml path has a stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert!(
        !names.is_empty(),
        "runtime/themes must bundle at least one theme"
    );

    names
        .into_iter()
        .map(|name| {
            let loaded = hume_engine::theme::loader::load_theme(&name, &paths)
                .unwrap_or_else(|e| panic!("bundled theme '{name}' failed to load: {e}"));
            // A bundled theme is HUME's own content, not a third-party import
            // — a warning here means a bug we shipped, not something a user
            // needs to see. Stronger than "it loads": without this, a
            // malformed key would still pass every check below it, just
            // with an empty style standing in for the one it broke.
            assert!(
                loaded.warnings.is_empty(),
                "bundled theme '{name}' produced load warnings: {:?}",
                loaded.warnings
            );
            (name, loaded.theme)
        })
        .collect()
}

/// Smoke-test all bundled themes through the full loader → resolve pipeline.
/// Catches wiring regressions (bad paths, parse errors, missing palette
/// entries) without needing a running editor.
#[test]
fn bundled_themes_load_and_resolve() {
    for (name, theme) in load_bundled_themes() {
        let style = theme.resolve_by_name(hume_engine::types::Scope("ui.cursor.primary"));
        assert!(
            style.fg.is_some() || style.bg.is_some(),
            "bundled theme '{name}': ui.cursor.primary has neither fg nor bg"
        );
    }
}

/// Every bundled theme must carry its own `ui.cursor.match.search` entry.
/// `raw_contains`, not `resolve_by_name`: after the `ui.selection.search` →
/// `ui.cursor.match.search` rename, a theme that dropped the key would
/// silently resolve through the dot-notation fallback onto `ui.cursor.match`
/// instead — a `resolve_by_name` check can't tell "has its own colour" from
/// "fell back to the bracket-match colour", only `raw_contains` can.
#[test]
fn bundled_themes_define_their_own_search_match_scope() {
    for (name, theme) in load_bundled_themes() {
        assert!(
            theme.raw_contains("ui.cursor.match.search"),
            "bundled theme '{name}': missing ui.cursor.match.search (renamed from ui.selection.search)"
        );
    }
}

/// Every bundled theme must give the four diagnostic *gutter* scopes
/// (Helix's own `error`/`warning`/`info`/`hint` naming for this surface) a
/// colour and, crucially, no underline — that decoration belongs to the
/// editing-area counterpart (`diagnostic.error` etc.), not to a sign-column
/// glyph. Drift-tolerant: asserts presence and absence-of-decoration, never
/// a hex value.
#[test]
fn bundled_theme_gutter_diagnostic_scopes_have_no_underline() {
    for (name, theme) in load_bundled_themes() {
        for scope in ["error", "warning", "info", "hint"] {
            assert!(
                theme.raw_contains(scope),
                "bundled theme '{name}': missing gutter diagnostic scope '{scope}'"
            );
            let style = theme.resolve_by_name(hume_engine::types::Scope(scope));
            assert!(
                style.fg.is_some(),
                "bundled theme '{name}': gutter scope '{scope}' has no fg"
            );
            assert_eq!(
                style.underline,
                hume_engine::types::UnderlineStyle::None,
                "bundled theme '{name}': gutter scope '{scope}' must not underline — \
                 that decoration belongs to the editing-area 'diagnostic.{scope}' scope"
            );
        }
    }
}

/// The three `diff.*.line` row tints must differ from each other in every
/// bundled theme. They are the only signal distinguishing an added, deleted or
/// changed row once the eye is past the one-glyph gutter marker, so two of them
/// sharing a background makes those rows indistinguishable at a glance.
/// Also pins that the directory scan above found real themes: an empty or
/// one-entry `runtime/themes` would make every other check here vacuous.
#[test]
fn bundled_theme_diff_line_tints_are_pairwise_distinct() {
    let scopes = ["diff.plus.line", "diff.minus.line", "diff.delta.line"];
    let themes = load_bundled_themes();
    assert!(
        themes.len() >= 3,
        "expected every bundled theme to be discovered, got: {:?}",
        themes.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );

    for (name, theme) in themes {
        let bgs: Vec<_> = scopes
            .iter()
            .map(|s| theme.resolve_by_name(hume_engine::types::Scope(s)).bg)
            .collect();
        for i in 0..scopes.len() {
            for j in (i + 1)..scopes.len() {
                assert_ne!(
                    bgs[i], bgs[j],
                    "bundled theme '{name}': '{}' and '{}' share the row tint {:?} — \
                     added, deleted and changed rows must not look alike",
                    scopes[i], scopes[j], bgs[i]
                );
            }
        }
    }
}

/// A mode scope that differs from the base row style must differ in its
/// `bg` — the whole-row tint (`EditorColors::from_theme`, `ui/theme.rs`) reads
/// a single style per mode and paints it across the entire statusline, so a
/// scope that overrides only `fg` renders as illegible accent-on-base-bg text
/// rather than a tinted row. `ui.statusline.normal` is exempt: it equals the
/// base row in every bundled theme by construction.
///
/// Overlaps with `bundled_theme_mode_scopes_are_pairwise_distinct` below for
/// every scope but `normal` (which isn't checked here, and isn't in that
/// test's own `mode_scopes` comparison base either) — kept separate because
/// this one anchors each scope directly against `ui.statusline`, rather than
/// against another mode scope.
#[test]
fn bundled_theme_mode_scopes_tint_the_whole_row() {
    let mode_scopes = [
        "ui.statusline.insert",
        "ui.statusline.select",
        "ui.statusline.search",
        "ui.statusline.command",
        "ui.statusline.sift",
    ];

    for (name, theme) in load_bundled_themes() {
        let base = theme.resolve_by_name(hume_engine::types::Scope("ui.statusline"));
        for scope in mode_scopes {
            let mode_style = theme.resolve_by_name(hume_engine::types::Scope(scope));
            if mode_style != base {
                assert_ne!(
                    mode_style.bg, base.bg,
                    "bundled theme '{name}': '{scope}' differs from 'ui.statusline' only in fg — \
                     the whole row tints with this style, so a bg-less override paints accent \
                     text on the untinted base background"
                );
            }
        }
    }
}

/// Every one of the six `ui.statusline.<mode>` scopes must resolve to a
/// distinct `bg` in every bundled theme. Since the whole-row tint makes row
/// color the primary mode signal, two modes sharing a background are
/// pixel-identical apart from a three-character label — easy for a
/// per-theme retune to miss without a check across every mode pair.
#[test]
fn bundled_theme_mode_scopes_are_pairwise_distinct() {
    let mode_scopes = [
        "ui.statusline.normal",
        "ui.statusline.insert",
        "ui.statusline.select",
        "ui.statusline.search",
        "ui.statusline.command",
        "ui.statusline.sift",
    ];

    for (name, theme) in load_bundled_themes() {
        let bgs: Vec<_> = mode_scopes
            .iter()
            .map(|scope| theme.resolve_by_name(hume_engine::types::Scope(scope)).bg)
            .collect();

        for i in 0..mode_scopes.len() {
            for j in (i + 1)..mode_scopes.len() {
                assert_ne!(
                    bgs[i], bgs[j],
                    "bundled theme '{name}': '{}' and '{}' share the same row bg {:?} — \
                     two modes would be indistinguishable",
                    mode_scopes[i], mode_scopes[j], bgs[i]
                );
            }
        }
    }
}

/// `:theme-debug`'s cursor rows must name a real rung chain — the bundled
/// `gruvbox` theme sets `ui.cursor.normal` directly, so that row's chain must
/// say so, not print the placeholder word the pre-fix implementation used in
/// place of every cursor row's chain. Pinned to `gruvbox` rather than `sand`:
/// gruvbox mirrors an established upstream Helix theme and isn't expected to
/// change, where `sand` is HUME's own theme and still gets retuned — a
/// fixture that happens to rely on one of its rungs would drift out from
/// under this test with no relation to what it actually checks. There is
/// deliberately no assertion that a theme's cursor colors differ across
/// modes: which modes get a distinct cursor cue, if any, is the theme
/// author's call, not a bundled-theme requirement.
#[test]
fn theme_debug_cursor_rows_show_a_real_chain_not_a_placeholder() {
    let mut ed = editor_from("-[a]>b\n");
    // `load_theme_by_name` resolves through the real XDG theme dirs, which
    // this unit test has no fixture for — load straight from the repo's
    // `runtime/themes`, the same path `load_bundled_themes` above uses.
    let themes_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes");
    ed.view.theme = hume_engine::theme::loader::load_theme("gruvbox", &[themes_dir])
        .expect("bundled theme 'gruvbox' must load")
        .theme;
    ed.execute_typed("theme-debug", None)
        .expect(":theme-debug must succeed");
    let text = ed
        .state
        .status_msg
        .as_ref()
        .expect(":theme-debug reports Info, which lands in status_msg");
    assert!(
        text.contains("cursor (normal): chain=ui.cursor.normal"),
        "expected the normal cursor row to name its defined rung, got: {text}"
    );
    assert!(
        !text.contains("resolved"),
        "cursor rows must not print the literal placeholder word, got: {text}"
    );
}

/// `load_theme_by_name` reports failure via the message log and returns `false`;
/// the theme stays unchanged.
#[test]
fn load_theme_by_name_fails_gracefully() {
    let mut ed = editor_from("-[a]>b\n");
    let ok = crate::editor::theme::load_theme_by_name(
        &mut ed.view,
        &mut ed.state.message_log,
        &mut ed.state.status_msg,
        "no_such_theme_xyz",
    );
    assert!(!ok, "expected false for nonexistent theme");
    // Failure warning ends up in the message log, not as an error result.
    assert!(
        ed.state.message_log.has_unseen(),
        "expected a warning message"
    );
}
