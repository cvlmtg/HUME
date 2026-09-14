//! Drift test for the generated theme-editor vocabulary file
//! (`tools/theme-editor/src/lib/vocabulary.generated.js`): keeps it in sync
//! with the loader's own accepted-name tables and `super::super::ui_scopes`'
//! UI chrome scope names, so a name added, removed, or renamed on the Rust
//! side fails here instead of only in the JS test suite that used to
//! regex-scrape this crate's source to find it.

use crate::theme::ui_scopes;
use crate::theme::{CURSOR_MODES, cursor_ladder_ids};

use super::flatten::STYLE_KEYS;
use super::values::{ANSI_COLORS, MODIFIER_NAMES, UNDERLINE_MODIFIER, UNDERLINE_NAMES};

/// `<repo>/tools/theme-editor/src/lib/`.
fn vocabulary_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tools/theme-editor/src/lib")
}

fn js_string_array(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Builds the whole generated file's text from the loader's own tables.
///
/// Asserts every emitted name is free of `"`/`\` and embeds cleanly in a JS
/// string literal unescaped — a canary against a future vocabulary name that
/// would otherwise emit malformed JS.
fn render_vocabulary_js() -> String {
    for (name, _) in MODIFIER_NAMES {
        assert!(
            !name.contains(['"', '\\']),
            "modifier name {name:?} cannot be embedded in a JS string literal \
             unescaped — update render_vocabulary_js to escape it"
        );
    }
    for (name, _) in UNDERLINE_NAMES {
        assert!(
            !name.contains(['"', '\\']),
            "underline style name {name:?} cannot be embedded in a JS string literal \
             unescaped — update render_vocabulary_js to escape it"
        );
    }
    for (name, _) in ANSI_COLORS {
        assert!(
            !name.contains(['"', '\\']),
            "ANSI colour name {name:?} cannot be embedded in a JS string literal \
             unescaped — update render_vocabulary_js to escape it"
        );
    }
    for name in ui_scopes::ALL {
        assert!(
            !name.contains(['"', '\\']),
            "UI scope name {name:?} cannot be embedded in a JS string literal \
             unescaped — update render_vocabulary_js to escape it"
        );
    }

    let modifier_names: Vec<&str> = MODIFIER_NAMES.iter().map(|&(n, _)| n).collect();
    let underline_names: Vec<&str> = UNDERLINE_NAMES.iter().map(|&(n, _)| n).collect();

    let ansi_colors = ANSI_COLORS
        .iter()
        .map(|(name, c)| format!("  \"{name}\": \"#{:02x}{:02x}{:02x}\",", c.0, c.1, c.2))
        .collect::<Vec<_>>()
        .join("\n");
    let modifiers = js_string_array(&modifier_names);
    let underline_modifier = UNDERLINE_MODIFIER;
    let underlines = js_string_array(&underline_names);
    let style_keys = js_string_array(&STYLE_KEYS);
    let ui_scopes_list = js_string_array(ui_scopes::ALL);

    // One concrete ladder pair per real mode in `CURSOR_MODES`, rather than a
    // templated fabricated scope — `cursor_ladder_ids` never receives
    // anything but a real mode's own literals in production, and iterating
    // `CURSOR_MODES` here pins that table's own scope names too, not just
    // `cursor_ladder_ids`' rung shape.
    let cursor_ladders = CURSOR_MODES
        .iter()
        .map(|&(_label, mode_scope, primary_mode_scope)| {
            let mode = mode_scope
                .rsplit('.')
                .next()
                .expect("a mode scope always has a last segment");
            // The two scopes must name the same mode: JS keys one ladder
            // pair per mode by this suffix, so a `CURSOR_MODES` entry whose
            // primary scope disagreed would emit a pair silently keyed by
            // the wrong mode.
            assert!(
                primary_mode_scope.ends_with(mode),
                "CURSOR_MODES entry {mode_scope} / {primary_mode_scope} disagree on the mode suffix"
            );
            let (secondary, primary) = cursor_ladder_ids(mode_scope, primary_mode_scope);
            format!(
                "  \"{mode}\": {{\n    secondary: {},\n    primary: {},\n  }},",
                js_string_array(&secondary),
                js_string_array(&primary),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "\
// tools/theme-editor/src/lib/vocabulary.generated.js — GENERATED, do not hand-edit.
//
// The theme loader's own vocabulary (hume-engine/src/theme/loader/) and UI
// chrome scope names (hume-engine/src/theme/ui_scopes.rs), emitted so the
// theme editor can offer only names HUME actually accepts, and reject
// nothing HUME would. Regenerate after any change to the loader's modifier,
// underline, ANSI-colour, style-key, or cursor-ladder vocabulary, or to
// ui_scopes::ALL:
//
//   HUME_WRITE_THEME_VOCABULARY=1 cargo test -p hume-engine theme_vocabulary_js_matches_loader
//
// hume-engine/src/theme/loader/vocabulary.rs's drift test fails the build if
// this file falls out of sync.

export const ANSI_COLORS = {{
{ansi_colors}
}};

export const MODIFIER_NAMES = {modifiers};

export const UNDERLINE_MODIFIER = \"{underline_modifier}\";

export const UNDERLINE_NAMES = {underlines};

export const STYLE_KEYS = {style_keys};

export const CURSOR_LADDERS = {{
{cursor_ladders}
}};

export const UI_SCOPES = {ui_scopes_list};
"
    )
}

/// Fail oracle: edit one name in the shipped `vocabulary.generated.js` (or
/// delete a line) — this test must fail, naming the file as stale.
#[test]
fn theme_vocabulary_js_matches_loader() {
    assert!(
        !MODIFIER_NAMES.is_empty() && !UNDERLINE_NAMES.is_empty() && !ANSI_COLORS.is_empty(),
        "sanity: the loader's vocabulary tables must not be empty"
    );

    let expected = render_vocabulary_js();
    let path = vocabulary_dir().join("vocabulary.generated.js");

    if std::env::var("HUME_WRITE_THEME_VOCABULARY").as_deref() == Ok("1") {
        std::fs::write(&path, &expected).expect("write vocabulary.generated.js");
        return;
    }

    let actual = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — generate it with:\n  \
             HUME_WRITE_THEME_VOCABULARY=1 cargo test -p hume-engine \
             theme_vocabulary_js_matches_loader",
            path.display()
        )
    });

    assert_eq!(
        actual, expected,
        "\ntools/theme-editor/src/lib/vocabulary.generated.js is stale. Regenerate with:\n\
         \n  HUME_WRITE_THEME_VOCABULARY=1 cargo test -p hume-engine theme_vocabulary_js_matches_loader\n"
    );
}
