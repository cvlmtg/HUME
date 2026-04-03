use std::collections::HashMap;

use crate::types::{ResolvedStyle, Scope, ScopeId};

// ---------------------------------------------------------------------------
// ScopeRegistry
// ---------------------------------------------------------------------------

/// Maps `&'static str` scope names to compact [`ScopeId`] integers.
///
/// Populate the registry by calling [`intern`] once per scope name at
/// construction time (e.g. when registering a `TreeSitterHighlighter`).
/// Then call [`Theme::bake`] with the registry before the first render.
/// After baking, [`Theme::resolve`] is an O(1) `Vec` index — no hashing.
///
/// Lives on [`crate::pipeline::EditorView`] so it outlives all providers.
#[derive(Default)]
pub struct ScopeRegistry {
    map: HashMap<&'static str, ScopeId>,
    names: Vec<&'static str>,
}

impl ScopeRegistry {
    pub fn new() -> Self {
        Self { map: HashMap::new(), names: Vec::new() }
    }

    /// Return the [`ScopeId`] for `name`, interning it if not yet seen.
    pub fn intern(&mut self, name: &'static str) -> ScopeId {
        if let Some(&id) = self.map.get(name) {
            return id;
        }
        debug_assert!(
            self.names.len() < u16::MAX as usize,
            "ScopeRegistry overflow: more than 65 535 distinct scope names"
        );
        let id = ScopeId(self.names.len() as u16);
        self.names.push(name);
        self.map.insert(name, id);
        id
    }

    /// Look up an already-interned scope without inserting.
    pub fn get(&self, name: &'static str) -> Option<ScopeId> {
        self.map.get(name).copied()
    }

    /// Reverse-lookup: return the name that was interned as `id`.
    ///
    /// Panics if `id` is out of range (i.e. not produced by this registry).
    pub fn name_of(&self, id: ScopeId) -> &'static str {
        self.names[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

// ---------------------------------------------------------------------------
// UiScopes
// ---------------------------------------------------------------------------

/// Pre-resolved styles for the UI scopes used on the per-grapheme hot path.
///
/// Computed eagerly in [`Theme::new`] so they are always valid — no bake
/// required to use them. Re-computed in [`Theme::bake`] in case the baked
/// resolution path would differ (it won't, since `raw` is immutable after
/// construction, but calling it is idempotent).
#[derive(Default)]
pub struct UiScopes {
    /// Cursor block/cell background (Normal, Visual, … modes).
    pub cursor: ResolvedStyle,
    /// Cursor bar/underline background (Insert mode).
    pub cursor_insert: ResolvedStyle,
    /// Selection highlight.
    pub selection: ResolvedStyle,
    /// Cursor-line background tint applied to every grapheme on the cursor row.
    pub cursorline: ResolvedStyle,
    /// Style for virtual / filler text (tilde rows, virtual lines).
    pub virtual_text: ResolvedStyle,
    /// Indent-guide column markers.
    pub indent_guide: ResolvedStyle,
    /// Primary cursor style (Normal/Extend/… modes). Falls back to `cursor` if unset.
    pub cursor_primary: ResolvedStyle,
    /// Primary cursor style in Insert mode. Falls back to `cursor_insert` if unset.
    pub cursor_insert_primary: ResolvedStyle,
    /// Primary selection highlight. Falls back to `selection` if unset.
    pub selection_primary: ResolvedStyle,
}


// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/// Maps semantic scope names to concrete styles.
///
/// # Two-phase API
///
/// 1. Construct with [`Theme::new`] (or [`Theme::default`]).
///    `theme.ui` is immediately usable — hot-path UI scopes are resolved from
///    the raw map on construction.
///
/// 2. After registering all providers, call [`Theme::bake`] with the
///    [`ScopeRegistry`]. This pre-resolves every provider-emitted scope into a
///    flat `Vec`, making [`Theme::resolve`] an O(1) index with no hashing.
///
/// # Fallback
///
/// Scope names use dot-notation with automatic fallback:
/// `keyword.function` → `keyword` → default.
pub struct Theme {
    /// Source map: written once at construction, read by `bake()` and
    /// `resolve_by_name()`. Never mutated after `new()`.
    raw: HashMap<&'static str, ResolvedStyle>,
    /// Per-[`ScopeId`] resolved styles. Populated by `bake()`.
    /// `baked[id.0]` is the style for the scope with that id.
    baked: Vec<ResolvedStyle>,
    /// Default style used when no scope matches.
    pub default: ResolvedStyle,
    /// Pre-resolved styles for hot-path UI scopes. Always valid (computed in
    /// `new()` and re-computed in `bake()`).
    pub ui: UiScopes,
}

impl Theme {
    /// Build a theme from a raw `scope → style` map.
    ///
    /// `ui` fields are resolved immediately from `styles`, so callers can use
    /// `theme.ui.*` before calling `bake()`.
    pub fn new(styles: HashMap<&'static str, ResolvedStyle>, default: ResolvedStyle) -> Self {
        let mut t = Self {
            raw: styles,
            baked: Vec::new(),
            default,
            ui: UiScopes::default(),
        };
        t.ui = t.compute_ui();
        t
    }

    /// Pre-resolve all scopes interned in `registry` into a flat `Vec`.
    ///
    /// After baking, [`resolve`] is an O(1) `Vec` index — no hashing.
    ///
    /// Call this once, after all providers have been registered (so their
    /// scopes are interned) and before the first render.
    pub fn bake(&mut self, registry: &ScopeRegistry) {
        self.baked.clear();
        self.baked.resize(registry.len(), self.default);
        for i in 0..registry.len() {
            self.baked[i] = self.resolve_raw(registry.name_of(ScopeId(i as u16)));
        }
        // Re-populate ui in case bake() is called after new() (idempotent).
        self.ui = self.compute_ui();
    }

    /// Look up the style for an interned scope.
    ///
    /// **O(1)** after [`bake`]. Returns `default` for IDs created after the
    /// last `bake()` call (a programming error — debug-assert helps catch it).
    #[inline]
    pub fn resolve(&self, id: ScopeId) -> ResolvedStyle {
        debug_assert!(
            (id.0 as usize) < self.baked.len(),
            "ScopeId {:?} is out of range — was bake() called after all providers were registered?",
            id
        );
        self.baked.get(id.0 as usize).copied().unwrap_or(self.default)
    }

    /// Resolve a scope name via the dot-notation fallback chain.
    ///
    /// **Slow path** (one `HashMap` lookup per dot segment). Use this only for
    /// non-hot call sites such as gutter-cell rendering (~100 calls/frame).
    /// On the per-grapheme hot path, intern the scope and use [`resolve`].
    pub fn resolve_by_name(&self, scope: Scope) -> ResolvedStyle {
        self.resolve_raw(scope.0)
    }

    // ── Private helpers ──────────────────────────────────────────────────

    fn compute_ui(&self) -> UiScopes {
        UiScopes {
            cursor: self.resolve_raw("ui.cursor"),
            cursor_insert: self.resolve_raw("ui.cursor.insert"),
            selection: self.resolve_raw("ui.selection"),
            cursorline: self.resolve_raw("ui.cursorline"),
            virtual_text: self.resolve_raw("ui.virtual"),
            indent_guide: self.resolve_raw("ui.indent_guide"),
            // Primary cursor: dot-notation fallback ui.cursor.primary → ui.cursor is correct.
            cursor_primary: self.resolve_raw("ui.cursor.primary"),
            // Primary insert cursor: if no explicit ui.cursor.primary.insert is defined, prefer
            // ui.cursor.insert over ui.cursor.primary — the mode-specific style takes precedence
            // over the primary-generic style. Dot-notation alone would give the wrong chain
            // (ui.cursor.primary.insert → ui.cursor.primary → ui.cursor, skipping ui.cursor.insert).
            cursor_insert_primary: if self.raw.contains_key("ui.cursor.primary.insert") {
                self.resolve_raw("ui.cursor.primary.insert")
            } else {
                self.resolve_raw("ui.cursor.insert") // falls back to ui.cursor
            },
            // Primary selection: dot-notation fallback ui.selection.primary → ui.selection is correct.
            selection_primary: self.resolve_raw("ui.selection.primary"),
        }
    }

    fn resolve_raw(&self, mut s: &'static str) -> ResolvedStyle {
        loop {
            if let Some(&style) = self.raw.get(s) {
                return style;
            }
            match s.rfind('.') {
                Some(dot) => s = &s[..dot],
                None => return self.default,
            }
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(HashMap::new(), ResolvedStyle::default())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn make_theme() -> Theme {
        let mut styles = HashMap::new();
        styles.insert("keyword", ResolvedStyle { fg: Some(Color::Blue), ..Default::default() });
        styles.insert("keyword.operator", ResolvedStyle { fg: Some(Color::Cyan), ..Default::default() });
        Theme::new(styles, ResolvedStyle::default())
    }

    // ── Theme::resolve (baked path) ──────────────────────────────────────

    #[test]
    fn direct_lookup() {
        let mut reg = ScopeRegistry::new();
        let id = reg.intern("keyword.operator");
        let mut theme = make_theme();
        theme.bake(&reg);
        assert_eq!(theme.resolve(id).fg, Some(Color::Cyan));
    }

    #[test]
    fn fallback_to_parent() {
        // "keyword.function" is not in the map; falls back to "keyword".
        let mut reg = ScopeRegistry::new();
        let id = reg.intern("keyword.function");
        let mut theme = make_theme();
        theme.bake(&reg);
        assert_eq!(theme.resolve(id).fg, Some(Color::Blue));
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
        assert_eq!(theme.resolve(kw).fg, Some(Color::Blue));
        assert_eq!(theme.resolve(kw_op).fg, Some(Color::Cyan));
        assert_eq!(theme.resolve(kw_fn).fg, Some(Color::Blue)); // fallback baked in
    }

    #[test]
    fn multi_level_fallback() {
        let mut styles = HashMap::new();
        styles.insert("a.b", ResolvedStyle { fg: Some(Color::Green), ..Default::default() });
        let mut theme = Theme::new(styles, ResolvedStyle::default());

        let mut reg = ScopeRegistry::new();
        let abc = reg.intern("a.b.c");
        let abcd = reg.intern("a.b.c.d");
        theme.bake(&reg);

        assert_eq!(theme.resolve(abc).fg, Some(Color::Green));
        assert_eq!(theme.resolve(abcd).fg, Some(Color::Green));
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
        assert_eq!(theme.resolve_by_name(Scope("keyword.operator")).fg, Some(Color::Cyan));
    }

    #[test]
    fn resolve_by_name_fallback() {
        let theme = make_theme();
        assert_eq!(theme.resolve_by_name(Scope("keyword.function")).fg, Some(Color::Blue));
    }

    #[test]
    fn resolve_by_name_default() {
        let theme = make_theme();
        assert_eq!(theme.resolve_by_name(Scope("comment")).fg, None);
    }

    // ── UiScopes: populated eagerly in new(), no bake() required ─────────

    #[test]
    fn ui_scopes_available_before_bake() {
        let mut styles = HashMap::new();
        styles.insert(
            "ui.cursorline",
            ResolvedStyle { bg: Some(Color::Blue), ..Default::default() },
        );
        let theme = Theme::new(styles, ResolvedStyle::default());
        // theme.bake() NOT called — ui.cursorline must still be correct.
        assert_eq!(theme.ui.cursorline.bg, Some(Color::Blue));
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
}
