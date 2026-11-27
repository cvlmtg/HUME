use super::*;

// ── Bundled theme loading (end-to-end wiring) ─────────────────────────────────

fn bundled_themes_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes")
}

/// Every bundled theme's name, sorted.
fn bundled_theme_names() -> Vec<String> {
    // Read the directory rather than listing the names: it is already the
    // source of truth for `:theme <Tab>`, and a hand-written list here means a
    // newly bundled theme silently skips every check in this file.
    let mut names: Vec<String> = std::fs::read_dir(bundled_themes_dir())
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
}

/// Load every bundled theme through the same loader path production code
/// uses. No `bake()`: every check below reads scopes via `resolve_by_name`,
/// which walks the raw dot-notation map directly (same as
/// `EditorColors::from_theme` at render time). Baking only feeds the
/// ID-based `resolve()` fast path, which none of these tests exercise.
fn load_bundled_themes() -> Vec<(String, hume_engine::theme::Theme)> {
    let paths = vec![bundled_themes_dir()];

    bundled_theme_names()
        .into_iter()
        .map(|name| {
            let loaded = hume_engine::theme::loader::load_theme(&name, &paths)
                .unwrap_or_else(|e| panic!("bundled theme '{name}' failed to load: {e}"));
            // A bundled theme is HUME's own content, not a third-party import:
            // a warning here means a bug we shipped, not something a user
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
            style.fg.is_some()
                || style.bg.is_some()
                || style
                    .modifiers
                    .contains(hume_engine::types::Modifiers::REVERSED),
            "bundled theme '{name}': ui.cursor.primary has neither fg, bg, nor reversed"
        );
    }
}

/// `sand` sets its own `ui.context` entry, so header rows do not rely on the
/// `ui.popup` fallback.
#[test]
fn sand_defines_ui_context() {
    let (_, sand) = load_bundled_themes()
        .into_iter()
        .find(|(name, _)| name == "sand")
        .expect("sand is bundled");
    assert!(sand.has_entry("ui.context"));
    assert_eq!(
        sand.ui().context,
        sand.resolve_by_name(hume_engine::types::Scope("ui.context"))
    );
}

/// Themes written for HUME, covered by the repository's own `LICENSE`.
const HUME_ORIGINAL_THEMES: [&str; 1] = ["sand"];

/// Exhibit A of the MPL-2.0, which every file copied from Helix must carry.
const MPL_NOTICE: &str = "This Source Code Form is subject to the terms of the Mozilla Public \
    License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain \
    one at https://mozilla.org/MPL/2.0/.";

/// A ported theme is redistributed third-party work: its header names the
/// upstream license shipped in `runtime/themes/licenses/`, and a file taken
/// from Helix also carries the MPL notice its license requires in the file.
#[test]
fn bundled_themes_carry_their_license_notices() {
    let themes_dir = bundled_themes_dir();
    for name in bundled_theme_names() {
        if HUME_ORIGINAL_THEMES.contains(&name.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(themes_dir.join(format!("{name}.toml")))
            .unwrap_or_else(|e| panic!("bundled theme '{name}' must be readable: {e}"));
        let header: Vec<&str> = text
            .lines()
            .take_while(|l| l.is_empty() || l.starts_with('#'))
            .filter_map(|l| l.strip_prefix('#'))
            .map(str::trim)
            .collect();

        let license = header
            .iter()
            .find_map(|l| l.strip_prefix("License:"))
            .map(str::trim)
            .unwrap_or_else(|| panic!("bundled theme '{name}': header has no 'License:' line"));
        assert!(
            license.starts_with("licenses/") && themes_dir.join(license).is_file(),
            "bundled theme '{name}': 'License: {license}' must name a file in runtime/themes/licenses/"
        );

        let from_helix = header
            .iter()
            .filter_map(|l| l.strip_prefix("Source:"))
            .any(|url| {
                url.trim()
                    .starts_with("https://github.com/helix-editor/helix/")
            });
        if from_helix {
            assert!(
                header.join(" ").contains(MPL_NOTICE),
                "bundled theme '{name}': ported from Helix but its header lacks the MPL-2.0 notice"
            );
        }
    }
}

/// Every bundled theme must carry its own `ui.cursor.match.search` entry.
/// `has_entry`, not `resolve_by_name`: after the `ui.selection.search` →
/// `ui.cursor.match.search` rename, a theme that dropped the key would
/// silently resolve through the dot-notation fallback onto `ui.cursor.match`
/// instead. A `resolve_by_name` check can't tell "has its own colour" from
/// "fell back to the bracket-match colour", only `has_entry` can.
#[test]
fn bundled_themes_define_their_own_search_match_scope() {
    for (name, theme) in load_bundled_themes() {
        assert!(
            theme.has_entry("ui.cursor.match.search"),
            "bundled theme '{name}': missing ui.cursor.match.search (renamed from ui.selection.search)"
        );
    }
}

/// Every bundled theme must give the four diagnostic *gutter* scopes
/// (Helix's own `error`/`warning`/`info`/`hint` naming for this surface) a
/// colour and, crucially, no underline: that decoration belongs to the
/// editing-area counterpart (`diagnostic.error` etc.), not to a sign-column
/// glyph. Drift-tolerant: asserts presence and absence-of-decoration, never
/// a hex value.
#[test]
fn bundled_theme_gutter_diagnostic_scopes_have_no_underline() {
    for (name, theme) in load_bundled_themes() {
        for scope in ["error", "warning", "info", "hint"] {
            assert!(
                theme.has_entry(scope),
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
                "bundled theme '{name}': gutter scope '{scope}' must not underline: \
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
                    "bundled theme '{name}': '{}' and '{}' share the row tint {:?}: \
                     added, deleted and changed rows must not look alike",
                    scopes[i], scopes[j], bgs[i]
                );
            }
        }
    }
}

/// The four scopes each markdown heading level uses, level 1 first.
const HEADING_SCOPES: [(&str, &str, &str, &str); 6] = [
    (
        "markup.heading.1",
        "markup.heading.1.marker",
        "markup.heading.1.text",
        "markup.heading.1.line",
    ),
    (
        "markup.heading.2",
        "markup.heading.2.marker",
        "markup.heading.2.text",
        "markup.heading.2.line",
    ),
    (
        "markup.heading.3",
        "markup.heading.3.marker",
        "markup.heading.3.text",
        "markup.heading.3.line",
    ),
    (
        "markup.heading.4",
        "markup.heading.4.marker",
        "markup.heading.4.text",
        "markup.heading.4.line",
    ),
    (
        "markup.heading.5",
        "markup.heading.5.marker",
        "markup.heading.5.text",
        "markup.heading.5.line",
    ),
    (
        "markup.heading.6",
        "markup.heading.6.marker",
        "markup.heading.6.text",
        "markup.heading.6.line",
    ),
];

/// Every heading level has a band in every bundled theme. A missing `.line`
/// entry resolves through `markup.heading.N` to a style with no background,
/// which this rejects.
#[test]
fn bundled_theme_heading_levels_have_a_band() {
    for (name, theme) in load_bundled_themes() {
        for &(_, _, _, line) in &HEADING_SCOPES {
            assert!(
                theme
                    .resolve_by_name(hume_engine::types::Scope(line))
                    .bg
                    .is_some(),
                "bundled theme '{name}': '{line}' has no background"
            );
        }
    }
}

/// Each `markup.heading.N.text` is bold and keeps the color the theme already
/// gives `markup.heading.N`, so the heading text does not change hue when the
/// plugin hides the raw `#` marker behind the level number.
#[test]
fn bundled_theme_heading_text_is_bold_in_its_headings_color() {
    for (name, theme) in load_bundled_themes() {
        for (heading, _, text, _) in HEADING_SCOPES {
            let text_style = theme.resolve_by_name(hume_engine::types::Scope(text));
            let heading_style = theme.resolve_by_name(hume_engine::types::Scope(heading));
            assert!(
                text_style
                    .modifiers
                    .contains(hume_engine::types::Modifiers::BOLD),
                "bundled theme '{name}': '{text}' is not bold"
            );
            assert_eq!(
                text_style.fg, heading_style.fg,
                "bundled theme '{name}': '{text}' differs in color from '{heading}'"
            );
        }
    }
}

/// Each `markup.heading.N.marker` is the level glyph and bar drawn in
/// the color of `markup.heading.N` over the line's band, never reversed: a
/// reversed bar would show as a block with a gap on its right.
#[test]
fn bundled_theme_heading_marker_is_its_headings_color_not_reversed() {
    for (name, theme) in load_bundled_themes() {
        for (heading, marker, _, _) in HEADING_SCOPES {
            let marker_style = theme.resolve_by_name(hume_engine::types::Scope(marker));
            let heading_style = theme.resolve_by_name(hume_engine::types::Scope(heading));
            assert!(
                !marker_style
                    .modifiers
                    .contains(hume_engine::types::Modifiers::REVERSED),
                "bundled theme '{name}': '{marker}' is reversed"
            );
            assert_eq!(
                marker_style.fg, heading_style.fg,
                "bundled theme '{name}': '{marker}' differs in color from '{heading}'"
            );
        }
    }
}

/// A fenced code block's band needs a background of its own: the scope it
/// falls back to, `markup.raw.block`, styles the text and carries none.
#[test]
fn bundled_theme_code_block_band_has_a_background() {
    for (name, theme) in load_bundled_themes() {
        let style = theme.resolve_by_name(hume_engine::types::Scope("markup.raw.block.line"));
        assert!(
            style.bg.is_some(),
            "bundled theme '{name}': 'markup.raw.block.line' has no background"
        );
    }
}

/// A mode scope that differs from the base row style must carry a `bg` of its
/// own: the whole-row tint (`EditorColors::from_theme`, `hume-editor/src/statusline/colors.rs`)
/// reads one style per mode and paints it across the entire statusline, so an
/// entry that sets only `fg` leaves the row untinted under accent-colored text.
/// The `bg` may match the base row's; only its absence breaks the tint.
/// `ui.statusline.normal` is exempt: it equals the base row in every bundled
/// theme.
///
/// Overlaps with `bundled_theme_mode_scopes_are_pairwise_distinct` below for
/// every scope but `normal` (which isn't checked here, and isn't in that
/// test's own `mode_scopes` comparison base either), kept separate because
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
                assert!(
                    mode_style.bg.is_some(),
                    "bundled theme '{name}': '{scope}' overrides 'ui.statusline' with no bg: \
                     the whole-row tint would leave the statusline untinted under accent text"
                );
            }
        }
    }
}

/// A Rust buffer with the real `rust` grammar fixture attached and parsed, so
/// `:theme-debug` has highlight spans to read.
fn rust_editor(initial: &str) -> Editor {
    let mut ed =
        super::structural::grammar_editor_undrained("rust", "tree_sitter_rust", &[], initial);
    ed.reparse_stale_buffers();
    ed
}

/// Theme entries for the `:theme-debug` tests: `keyword` is defined, so a
/// capture with no entry of its own falls back to `default`.
fn keyword_entries() -> std::collections::HashMap<&'static str, hume_engine::types::ResolvedStyle> {
    std::collections::HashMap::from([(
        "keyword",
        hume_engine::types::ResolvedStyle {
            fg: Some(hume_grid::Rgb(0x44, 0x55, 0x66)),
            ..Default::default()
        },
    )])
}

/// `:theme-debug`'s statusline line for `initial` under the keyword fixture.
fn theme_debug_message(initial: &str) -> String {
    let mut ed = rust_editor(initial);
    ed.view.theme = hume_engine::theme::Theme::new(keyword_entries());
    ed.execute_typed("theme-debug", None)
        .expect(":theme-debug must succeed");
    ed.state
        .status_msg
        .text()
        .expect(":theme-debug reports Info, which lands in status_msg")
        .to_owned()
}

/// The line names the syntax scope under the cursor and the theme keys that
/// resolve it. `fn` is `keyword.function`, which falls back to the theme's
/// `keyword`; `main` is `function`, which has no entry at all.
#[test]
fn theme_debug_reports_the_scope_under_the_cursor() {
    assert_eq!(
        theme_debug_message("-[f]>n main() {}\n"),
        "syntax: keyword.function → keyword fg=#445566 bg=- | effective fg=#445566 bg=-"
    );
    assert_eq!(
        theme_debug_message("fn -[m]>ain() {}\n"),
        "syntax: function → default fg=- bg=- | effective fg=- bg=-"
    );
}

#[test]
fn theme_debug_reports_no_scope_when_no_tier_is_active() {
    let mut ed = editor_from("-[a]>b\n");
    ed.execute_typed("theme-debug", None)
        .expect(":theme-debug must succeed");
    assert_eq!(
        ed.state.status_msg.text(),
        Some("no highlight scope at cursor")
    );
}

#[test]
fn theme_debug_reports_no_scope_on_whitespace() {
    assert_eq!(
        theme_debug_message("fn -[ ]>main() {}\n"),
        "no highlight scope at cursor"
    );
}

/// A plugin span over bytes 0..2 of line 0, in the Extra tier.
struct ProbeExtra(hume_engine::types::ScopeId);

impl hume_engine::providers::DecorationSource for ProbeExtra {
    fn kinds(&self) -> hume_engine::providers::DecorationKinds {
        hume_engine::providers::DecorationKinds::HIGHLIGHT
    }

    fn decorations_for_line(
        &self,
        line: hume_rope::line::ContentLine,
        out: &mut Vec<hume_engine::providers::Decoration>,
    ) {
        if line.index() == 0 {
            out.push(hume_engine::providers::Decoration::Highlight {
                range: hume_rope::offset::ExclusiveRange::new(
                    hume_rope::column::ByteCol::new(0),
                    hume_rope::column::ByteCol::new(2),
                ),
                scope: self.0,
                tier: hume_engine::providers::HighlightTier::Extra,
            });
        }
    }
}

#[test]
fn theme_debug_reports_every_active_layer() {
    let mut ed = rust_editor("-[f]>n main() {}\n");
    let scope = ed.view.registry.intern("plugin.mark");
    let mut entries = keyword_entries();
    entries.insert(
        "plugin.mark",
        hume_engine::types::ResolvedStyle {
            bg: Some(hume_grid::Rgb(0x11, 0x22, 0x33)),
            modifiers: hume_engine::types::Modifiers::BOLD,
            ..Default::default()
        },
    );
    ed.view.theme = hume_engine::theme::Theme::new(entries);
    let pid = ed.view.focused_pane();
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(ProbeExtra(scope)));
    ed.execute_typed("theme-debug", None)
        .expect(":theme-debug must succeed");
    assert_eq!(
        ed.state.status_msg.text(),
        Some(
            "syntax: keyword.function → keyword fg=#445566 bg=- | \
             extra: plugin.mark → plugin fg=- bg=#112233 modifiers BOLD | \
             effective fg=#445566 bg=#112233 modifiers BOLD"
        )
    );
}

#[test]
fn theme_debug_uses_script_styles_in_the_chain() {
    let mut ed = rust_editor("-[f]>n main() {}\n");
    ed.view.theme = hume_engine::theme::Theme::new(keyword_entries());
    // No install pending: the script's style must land on the theme on screen.
    ed.state.settings.theme = ed.state.shown_theme.clone();
    crate::editor::theme::set_scope_styles(
        &mut ed.state,
        &mut ed.view,
        &[hume_core::ScopeStyleInput {
            scope: "keyword".to_owned(),
            fg: Some("#aabbcc".to_owned()),
            ..Default::default()
        }],
        hume_core::ConfigOrigin::User,
    )
    .expect("script style must apply");
    ed.execute_typed("theme-debug", None)
        .expect(":theme-debug must succeed");
    assert_eq!(
        ed.state.status_msg.text(),
        Some("syntax: keyword.function → keyword fg=#aabbcc bg=- | effective fg=#aabbcc bg=-")
    );
}

/// `load_theme_by_name` reports failure via the message log and returns `false`;
/// the theme stays unchanged.
#[test]
fn load_theme_by_name_fails_gracefully() {
    let mut ed = editor_from("-[a]>b\n");
    let ok =
        crate::editor::theme::load_theme_by_name(&mut ed.state, &mut ed.view, "no_such_theme_xyz");
    assert!(!ok, "expected false for nonexistent theme");
    // Failure warning ends up in the message log, not as an error result.
    assert!(
        ed.state.message_log.has_unseen(),
        "expected a warning message"
    );
}

#[test]
fn a_failed_theme_load_leaves_an_error_already_on_the_statusline() {
    let mut ed = editor_from("-[a]>b\n");
    ed.state.status_msg.show(Severity::Error, "earlier failure");
    let ok =
        crate::editor::theme::load_theme_by_name(&mut ed.state, &mut ed.view, "no_such_theme_xyz");
    assert!(!ok);
    assert_eq!(ed.state.status_msg.text(), Some("earlier failure"));
}
