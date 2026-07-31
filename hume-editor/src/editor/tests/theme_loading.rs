use super::*;

// ── Bundled theme loading (end-to-end wiring) ─────────────────────────────────

/// Load every bundled theme through the same loader path production code
/// uses. No `bake()`: every check below reads scopes via `resolve_by_name`,
/// which walks the raw dot-notation map directly (same as
/// `EditorColors::from_theme` at render time) — baking only feeds the
/// ID-based `resolve()` fast path, which none of these tests exercise.
fn load_bundled_themes() -> Vec<(&'static str, hume_engine::theme::Theme)> {
    use std::path::PathBuf;
    let themes_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes");
    let paths = vec![themes_dir];

    ["dark", "light", "gruvbox", "sand"]
        .into_iter()
        .map(|name| {
            let theme = hume_engine::theme::loader::load_theme(name, &paths)
                .unwrap_or_else(|e| panic!("bundled theme '{name}' failed to load: {e}"));
            (name, theme)
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
        "ui.statusline.extend",
        "ui.statusline.search",
        "ui.statusline.command",
        "ui.statusline.select",
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
        "ui.statusline.extend",
        "ui.statusline.search",
        "ui.statusline.command",
        "ui.statusline.select",
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
