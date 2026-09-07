use super::*;
use crate::test_support::{bg, fg, theme_with};
use hume_grid::Rgb;

fn make_theme() -> Theme {
    theme_with([
        ("keyword", fg(Rgb(0, 0, 255))),
        ("keyword.operator", fg(Rgb(0, 255, 255))),
    ])
}

// ── Theme::resolve (baked path) ──────────────────────────────────────

#[test]
fn direct_lookup() {
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("keyword.operator");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(id).fg, Some(Rgb(0, 255, 255)));
}

#[test]
fn fallback_to_parent() {
    // "keyword.function" is not in the map; falls back to "keyword".
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("keyword.function");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(id).fg, Some(Rgb(0, 0, 255)));
}

#[test]
fn fallback_to_default() {
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("comment");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(id).fg, None);
}

#[test]
fn bake_resolves_all_interned_scopes() {
    // After bake(), both a direct hit and a fallback scope are O(1).
    let mut reg = ScopeRegistry::new();
    let kw = reg.intern("keyword");
    let kw_op = reg.intern("keyword.operator");
    let kw_fn = reg.intern("keyword.function"); // not in map → falls back
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(kw).fg, Some(Rgb(0, 0, 255)));
    assert_eq!(theme.resolve(kw_op).fg, Some(Rgb(0, 255, 255)));
    assert_eq!(theme.resolve(kw_fn).fg, Some(Rgb(0, 0, 255))); // fallback baked in
}

#[test]
fn bake_if_stale_rebakes_scopes_interned_after_bake() {
    let mut reg = ScopeRegistry::new();
    let kw = reg.intern("keyword");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.baked.len(), reg.len());

    // Intern a new scope after bake() — this id is now unbaked.
    let kw_op = reg.intern("keyword.operator");
    assert!(
        theme.baked.len() < reg.len(),
        "registry must have outgrown baked"
    );

    theme.bake_if_stale(&reg);
    assert_eq!(theme.baked.len(), reg.len());
    // Independent oracle: the themed color, not the pre-rebake default.
    assert_eq!(theme.resolve(kw_op).fg, Some(Rgb(0, 255, 255)));
    assert_eq!(theme.resolve(kw).fg, Some(Rgb(0, 0, 255)));

    // No new scopes interned — bake_if_stale is a no-op.
    let baked_before = theme.baked.clone();
    theme.bake_if_stale(&reg);
    assert_eq!(theme.baked, baked_before);
}

#[test]
fn multi_level_fallback() {
    let mut theme = theme_with([("a.b", fg(Rgb(0, 255, 0)))]);

    let mut reg = ScopeRegistry::new();
    let abc = reg.intern("a.b.c");
    let abcd = reg.intern("a.b.c.d");
    theme.bake(&reg);

    assert_eq!(theme.resolve(abc).fg, Some(Rgb(0, 255, 0)));
    assert_eq!(theme.resolve(abcd).fg, Some(Rgb(0, 255, 0)));
}

#[test]
fn empty_theme_returns_default() {
    let mut reg = ScopeRegistry::new();
    let any = reg.intern("anything");
    let empty_str = reg.intern("");
    let mut theme = Theme::default();
    theme.bake(&reg);
    assert_eq!(theme.resolve(any), ResolvedStyle::default());
    assert_eq!(theme.resolve(empty_str), ResolvedStyle::default());
}

// ── Theme::resolve_by_name (slow path, no bake needed) ───────────────

#[test]
fn resolve_by_name_direct() {
    let theme = make_theme();
    assert_eq!(
        theme.resolve_by_name(Scope("keyword.operator")).fg,
        Some(Rgb(0, 255, 255))
    );
}

#[test]
fn resolve_by_name_fallback() {
    let theme = make_theme();
    assert_eq!(
        theme.resolve_by_name(Scope("keyword.function")).fg,
        Some(Rgb(0, 0, 255))
    );
}

#[test]
fn resolve_by_name_default() {
    let theme = make_theme();
    assert_eq!(theme.resolve_by_name(Scope("comment")).fg, None);
}

// ── UiScopes: populated eagerly in new(), no bake() required ─────────

#[test]
fn ui_scopes_available_before_bake() {
    let theme = theme_with([("ui.cursorline", bg(Rgb(0, 0, 255)))]);
    // theme.bake() NOT called — ui.cursorline must still be correct.
    assert_eq!(theme.ui.cursorline.bg, Some(Rgb(0, 0, 255)));
}

#[test]
fn window_focused_falls_back_to_window_when_unset() {
    // No "ui.window.focused" entry — dot-notation must fall back to "ui.window".
    let theme = theme_with([("ui.window", fg(Rgb(0x80, 0x80, 0x80)))]);
    assert_eq!(theme.ui.window_focused.fg, Some(Rgb(0x80, 0x80, 0x80)));
    assert_eq!(theme.ui.window_focused, theme.ui.window);
}

#[test]
fn window_focused_uses_its_own_entry_when_set() {
    let theme = theme_with([
        ("ui.window", fg(Rgb(0x80, 0x80, 0x80))),
        ("ui.window.focused", fg(Rgb(0xff, 0x80, 0x00))),
    ]);
    assert_eq!(theme.ui.window_focused.fg, Some(Rgb(0xff, 0x80, 0x00)));
    assert_ne!(theme.ui.window_focused, theme.ui.window);
}

// ── ui.cursor.match.search: dot-fallback onto ui.cursor.match ────────

#[test]
fn search_match_falls_back_to_bracket_match() {
    // Theme defines ui.cursor.match but not ui.cursor.match.search — the
    // everforest case. Search matches must inherit the bracket-match style.
    let theme = theme_with([("ui.cursor.match", fg(Rgb(255, 128, 0)))]);
    assert_eq!(
        theme.resolve_by_name(Scope("ui.cursor.match.search")).fg,
        Some(Rgb(255, 128, 0)),
    );
}

#[test]
fn search_match_uses_its_own_entry_when_set() {
    // A theme that defines both must keep them distinct — no bundled theme
    // regresses to sharing bracket-match's color.
    let theme = theme_with([
        ("ui.cursor.match", fg(Rgb(255, 128, 0))),
        ("ui.cursor.match.search", fg(Rgb(0, 128, 255))),
    ]);
    assert_eq!(
        theme.resolve_by_name(Scope("ui.cursor.match.search")).fg,
        Some(Rgb(0, 128, 255)),
    );
}

#[test]
fn search_match_falls_back_to_cursor_when_no_bracket_match_either() {
    let theme = theme_with([("ui.cursor", fg(Rgb(10, 20, 30)))]);
    assert_eq!(
        theme.resolve_by_name(Scope("ui.cursor.match.search")).fg,
        Some(Rgb(10, 20, 30)),
    );
}

// ── ui.cursor.select / ui.cursor.primary.select: Extend-mode cursor ──

#[test]
fn cursor_select_falls_back_to_cursor() {
    let theme = theme_with([("ui.cursor", fg(Rgb(1, 2, 3)))]);
    assert_eq!(theme.ui.cursor_select.fg, Some(Rgb(1, 2, 3)));
}

#[test]
fn cursor_select_uses_its_own_entry_when_set() {
    let theme = theme_with([
        ("ui.cursor", fg(Rgb(1, 2, 3))),
        ("ui.cursor.select", fg(Rgb(4, 5, 6))),
    ]);
    assert_eq!(theme.ui.cursor_select.fg, Some(Rgb(4, 5, 6)));
}

#[test]
fn cursor_primary_select_falls_back_through_cursor_primary_then_cursor() {
    let theme = theme_with([("ui.cursor", fg(Rgb(1, 1, 1)))]);
    assert_eq!(theme.ui.cursor_select_primary.fg, Some(Rgb(1, 1, 1)));

    let theme = theme_with([
        ("ui.cursor", fg(Rgb(1, 1, 1))),
        ("ui.cursor.primary", fg(Rgb(2, 2, 2))),
    ]);
    assert_eq!(theme.ui.cursor_select_primary.fg, Some(Rgb(2, 2, 2)));
}

#[test]
fn cursor_primary_select_uses_its_own_entry_when_set() {
    let theme = theme_with([
        ("ui.cursor.primary", fg(Rgb(2, 2, 2))),
        ("ui.cursor.primary.select", fg(Rgb(3, 3, 3))),
    ]);
    assert_eq!(theme.ui.cursor_select_primary.fg, Some(Rgb(3, 3, 3)));
}

/// A theme that colours Extend mode but never declares a `.primary` variant
/// gives the primary head the plain block colour, not the select colour —
/// matching Helix's own `base_primary_cursor_scope`, which prefix-walks
/// `ui.cursor.primary` → `ui.cursor` → `ui` and never consults
/// `ui.cursor.select`. Only `ui.cursor.primary.select` itself (an exact
/// lookup) can put the select colour on the primary head; see
/// `cursor_primary_select_uses_its_own_entry_when_set`.
#[test]
fn cursor_primary_select_never_visits_the_secondary_select_scope() {
    let theme = theme_with([
        ("ui.cursor", fg(Rgb(1, 1, 1))),
        ("ui.cursor.select", fg(Rgb(4, 5, 6))),
    ]);
    assert_eq!(theme.ui.cursor_select_primary.fg, Some(Rgb(1, 1, 1)));
}

// ── ui.cursor.normal / ui.cursor.primary.normal: Normal-mode cursor ──
// Matches Helix's own `ui.cursor.normal` match arm in `doc_selection_highlights`.

#[test]
fn cursor_falls_back_to_bare_cursor_when_no_normal_entry() {
    let theme = theme_with([("ui.cursor", fg(Rgb(7, 8, 9)))]);
    assert_eq!(theme.ui.cursor.fg, Some(Rgb(7, 8, 9)));
}

#[test]
fn cursor_uses_normal_entry_when_set() {
    let theme = theme_with([
        ("ui.cursor", fg(Rgb(7, 8, 9))),
        ("ui.cursor.normal", fg(Rgb(10, 11, 12))),
    ]);
    assert_eq!(theme.ui.cursor.fg, Some(Rgb(10, 11, 12)));
}

#[test]
fn cursor_primary_falls_back_through_primary_then_cursor() {
    // No ui.cursor.primary.normal, no ui.cursor.primary — falls all the way to ui.cursor.
    let theme = theme_with([("ui.cursor", fg(Rgb(1, 1, 1)))]);
    assert_eq!(theme.ui.cursor_primary.fg, Some(Rgb(1, 1, 1)));

    // ui.cursor.primary set, no .normal variant — falls back to ui.cursor.primary.
    let theme = theme_with([
        ("ui.cursor", fg(Rgb(1, 1, 1))),
        ("ui.cursor.primary", fg(Rgb(2, 2, 2))),
    ]);
    assert_eq!(theme.ui.cursor_primary.fg, Some(Rgb(2, 2, 2)));
}

#[test]
fn cursor_primary_normal_uses_its_own_entry_when_set() {
    let theme = theme_with([
        ("ui.cursor.primary", fg(Rgb(2, 2, 2))),
        ("ui.cursor.primary.normal", fg(Rgb(3, 3, 3))),
    ]);
    assert_eq!(theme.ui.cursor_primary.fg, Some(Rgb(3, 3, 3)));
}

/// Normal-mode twin of `cursor_primary_select_never_visits_the_secondary_select_scope`:
/// `ui.cursor.normal` is never consulted while resolving `cursor_primary` —
/// only `ui.cursor.primary.normal` (exact) or `ui.cursor.primary`/`ui.cursor`
/// can.
#[test]
fn cursor_primary_never_visits_the_secondary_normal_scope() {
    let theme = theme_with([
        ("ui.cursor", fg(Rgb(1, 1, 1))),
        ("ui.cursor.normal", fg(Rgb(10, 11, 12))),
    ]);
    assert_eq!(theme.ui.cursor_primary.fg, Some(Rgb(1, 1, 1)));
}

/// Everforest's own regression: `ui.cursor.insert` (secondary) and
/// `ui.cursor.primary` (plain block) are both defined, but
/// `ui.cursor.primary.insert` is not. The primary Insert-mode head must land
/// on the block colour, never on the secondary Insert colour — the bug this
/// ladder replaced gave both heads the same `ui.cursor.insert` colour.
#[test]
fn cursor_insert_primary_never_visits_the_secondary_insert_scope() {
    let theme = theme_with([
        ("ui.cursor.insert", fg(Rgb(4, 5, 6))),
        ("ui.cursor.primary", fg(Rgb(7, 8, 9))),
    ]);
    assert_eq!(theme.ui.cursor_insert_primary.fg, Some(Rgb(7, 8, 9)));
}

/// With no block scope defined at all, the primary Insert ladder now reaches
/// past `ui.cursor`/`ui` all the way to `ui.selection` — the tail Helix's own
/// `base_cursor_scope` falls to when `ui.cursor` is undefined.
#[test]
fn cursor_insert_primary_falls_back_to_selection_when_nothing_else_is_set() {
    let theme = theme_with([("ui.selection", fg(Rgb(1, 2, 3)))]);
    assert_eq!(theme.ui.cursor_insert_primary.fg, Some(Rgb(1, 2, 3)));
}

// ── ScopeRegistry ────────────────────────────────────────────────────

#[test]
fn registry_intern_is_stable() {
    let mut reg = ScopeRegistry::new();
    let a1 = reg.intern("keyword");
    let a2 = reg.intern("keyword"); // second intern returns same id
    assert_eq!(a1, a2);
}

#[test]
fn registry_different_names_get_different_ids() {
    let mut reg = ScopeRegistry::new();
    let a = reg.intern("keyword");
    let b = reg.intern("variable");
    assert_ne!(a, b);
}

#[test]
fn registry_name_of_round_trips() {
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("type.builtin");
    assert_eq!(reg.name_of(id), "type.builtin");
}

#[test]
fn registry_get_returns_none_for_unknown() {
    let reg = ScopeRegistry::new();
    assert_eq!(reg.get("unknown"), None);
}

// ── Theme::from_owned (loader path) ──────────────────────────────────

#[test]
fn from_owned_resolves_same_as_new() {
    let static_styles: HashMap<&'static str, ResolvedStyle> = {
        let mut m = HashMap::new();
        m.insert(
            "keyword",
            ResolvedStyle {
                fg: Some(Rgb(0, 0, 255)),
                ..Default::default()
            },
        );
        m
    };
    let owned_styles: FxHashMap<String, ResolvedStyle> = {
        let mut m = FxHashMap::default();
        m.insert(
            "keyword".to_string(),
            ResolvedStyle {
                fg: Some(Rgb(0, 0, 255)),
                ..Default::default()
            },
        );
        m
    };
    let t1 = Theme::new(static_styles, ResolvedStyle::default());
    let t2 = Theme::from_owned(owned_styles, ResolvedStyle::default());
    // Pinned to the literal both must produce, not to each other: the two
    // constructors funnel into the same code, so comparing their outputs would
    // still hold if dot-notation fallback broke in both at once.
    assert_eq!(
        t1.resolve_by_name(Scope("keyword.function")).fg,
        Some(Rgb(0, 0, 255)),
    );
    assert_eq!(
        t2.resolve_by_name(Scope("keyword.function")).fg,
        Some(Rgb(0, 0, 255)),
    );
}
