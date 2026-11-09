pub mod diagnostic_scopes;
pub mod error;
pub mod loader;
pub mod ui_scopes;

use std::collections::HashMap;

use rustc_hash::FxHashMap;

use crate::types::{ResolvedStyle, Scope, ScopeId};

// ScopeRegistry

/// Maps scope name strings to compact [`ScopeId`] integers, at most
/// `u16::MAX` of them. Append-only.
///
/// Interning is cold (construction, a grammar load, plugin activation), so
/// static and runtime names share one owned-key map. Interned scopes become
/// resolvable at the next [`Theme::bake_if_stale`].
///
/// Lives on [`crate::pipeline::EngineView`] so it outlives all providers.
#[derive(Default)]
pub struct ScopeRegistry {
    map: FxHashMap<Box<str>, ScopeId>,
    /// Indexed by `ScopeId`.
    names: Vec<Box<str>>,
}

impl ScopeRegistry {
    pub fn new() -> Self {
        Self {
            map: FxHashMap::default(),
            names: Vec::new(),
        }
    }

    fn next_id(&self) -> ScopeId {
        debug_assert!(
            self.names.len() < u16::MAX as usize,
            "ScopeRegistry overflow: more than 65 535 distinct scope names"
        );
        ScopeId(self.names.len() as u16)
    }

    /// Return the [`ScopeId`] for `name`, interning it if new.
    pub fn intern(&mut self, name: &str) -> ScopeId {
        if let Some(&id) = self.map.get(name) {
            return id;
        }
        let id = self.next_id();
        let boxed: Box<str> = Box::from(name);
        self.names.push(boxed.clone());
        self.map.insert(boxed, id);
        id
    }

    /// Look up an already-interned scope without inserting.
    pub fn get(&self, name: &str) -> Option<ScopeId> {
        self.map.get(name).copied()
    }

    /// The name interned as `id`.
    ///
    /// # Panics
    /// Panics if `id` was not produced by this registry.
    pub fn name_of(&self, id: ScopeId) -> &str {
        &self.names[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// The dot-notation fallback chain for `scope`, most specific first:
/// `"keyword.function"` yields `"keyword.function"`, then `"keyword"`.
///
/// `Theme::resolve_raw` takes the first name with an entry; `:theme-debug`
/// reports every name that has one.
pub fn fallback_chain(scope: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(scope), |cur| cur.rfind('.').map(|dot| &cur[..dot]))
}

/// The three cursor modes a theme's `ui.cursor*` scopes distinguish, as
/// `(display label, mode scope, primary mode scope)`, shared by
/// `Theme::compute_ui` and `:theme-debug`.
pub const CURSOR_MODES: [(&str, &str, &str); 3] = [
    ("normal", "ui.cursor.normal", "ui.cursor.primary.normal"),
    ("insert", "ui.cursor.insert", "ui.cursor.primary.insert"),
    // HUME's Extend mode keeps Helix's `select` scope names.
    ("extend", "ui.cursor.select", "ui.cursor.primary.select"),
];

/// The (secondary, primary) rung lists `Theme::cursor_ladder` resolves, for
/// one mode's own scope names. Public so `:theme-debug` can report which
/// rung matched.
pub fn cursor_ladder_ids(
    mode_scope: &'static str,
    primary_mode_scope: &'static str,
) -> ([&'static str; 3], [&'static str; 5]) {
    (
        [mode_scope, CURSOR, ui_scopes::SELECTION],
        [
            primary_mode_scope,
            CURSOR_PRIMARY,
            CURSOR,
            "ui",
            ui_scopes::SELECTION,
        ],
    )
}

/// The two cursor ladder roots. Cursor scope names live here rather than in
/// `ui_scopes`.
pub const CURSOR: &str = "ui.cursor";
pub const CURSOR_PRIMARY: &str = "ui.cursor.primary";

/// The bracket/search-match highlight under the cursor, resolved directly
/// rather than through a ladder.
pub const CURSOR_MATCH: &str = "ui.cursor.match";
pub const CURSOR_MATCH_SEARCH: &str = "ui.cursor.match.search";

// UiScopes

/// Pre-resolved styles for the UI scopes used on the per-grapheme hot path.
/// Computed at construction, so usable before any bake.
#[derive(Default)]
pub struct UiScopes {
    /// Secondary selection-head highlight in Normal mode. Falls back through
    /// `ui.cursor.normal` → `ui.cursor` → `ui.selection`.
    pub cursor: ResolvedStyle,
    /// Secondary selection-head highlight in Insert mode. Falls back through
    /// `ui.cursor.insert` → `ui.cursor` → `ui.selection`. Painted only when
    /// `insert-cursor-shape` is `block`.
    pub cursor_insert: ResolvedStyle,
    /// Selection highlight.
    pub selection: ResolvedStyle,
    /// Background tint applied to every grapheme on the primary selection head's line.
    pub cursorline: ResolvedStyle,
    /// Style for virtual / filler text (tilde display lines, virtual display lines).
    pub virtual_text: ResolvedStyle,
    /// Indent-guide column markers.
    pub indent_guide: ResolvedStyle,
    /// `ui.virtual.invisible`: the `<200b>` stand-in for a cluster the
    /// terminal must not show. Falls back to `ui.virtual`.
    pub invisible: ResolvedStyle,
    /// `ui.virtual.whitespace`: an opted-in whitespace indicator glyph
    /// (`·`, `→`, `⏎`, `⍽`). Falls back to `ui.virtual`. Never applied to
    /// `CellContent::TabFill`.
    pub whitespace: ResolvedStyle,
    /// Primary selection-head highlight in Normal mode. Falls back through
    /// `ui.cursor.primary.normal` → `ui.cursor.primary` → `ui.cursor` → `ui` →
    /// `ui.selection`. Painted only when the mode's cursor shape is `Block`
    /// (`PaneRenderSettings::cursor_is_block`).
    pub cursor_primary: ResolvedStyle,
    /// Primary selection-head highlight in Insert mode. Same ladder as
    /// [`Self::cursor_primary`], rooted at `ui.cursor.primary.insert`. Painted
    /// only when `insert-cursor-shape` is `block`; otherwise the terminal
    /// cursor marks this head.
    pub cursor_insert_primary: ResolvedStyle,
    /// Secondary selection-head highlight in Extend mode (Helix's Select
    /// mode). Falls back through `ui.cursor.select` → `ui.cursor` →
    /// `ui.selection`.
    pub cursor_select: ResolvedStyle,
    /// Primary selection-head highlight in Extend mode. Same ladder shape as
    /// [`Self::cursor_primary`], rooted at `ui.cursor.primary.select`. Always
    /// painted: Extend's cursor is always `Block`.
    pub cursor_select_primary: ResolvedStyle,
    /// Primary selection highlight. Falls back to `selection` if unset.
    pub selection_primary: ResolvedStyle,
    /// Pane background, painted behind all content cells.
    pub background: ResolvedStyle,
    /// Seam divider drawn between sibling panes (`ui.window`, Helix convention).
    pub window: ResolvedStyle,
    /// Seam divider segments adjacent to the focused pane. Falls back to
    /// `ui.window`.
    pub window_focused: ResolvedStyle,
    /// Scope header rows. Falls back to `ui.popup`, which is not on
    /// `ui.context`'s dot chain, so the fallback is explicit in `compute_ui`.
    pub context: ResolvedStyle,
}

// Theme

/// Maps semantic scope names to concrete styles.
///
/// # Two-phase API
///
/// 1. Construct with [`Theme::from_owned`] (or [`Theme::default`]).
///    `theme.ui` is usable at once.
///
/// 2. Bake against the [`ScopeRegistry`] ([`Theme::bake_if_stale`]) before
///    [`Theme::resolve`] reads an interned scope.
///
/// Scope names fall back by dot-notation: `keyword.function` → `keyword` →
/// default.
pub struct Theme {
    /// Source map, never mutated after construction.
    raw: FxHashMap<String, ResolvedStyle>,
    /// Indexed by [`ScopeId`]; `None` when no name on the scope's fallback
    /// chain has an entry.
    baked: Vec<Option<ResolvedStyle>>,
    /// Default style used when no scope matches.
    pub default: ResolvedStyle,
    /// Pre-resolved styles for hot-path UI scopes.
    pub ui: UiScopes,
}

impl Theme {
    /// Build a theme from a `scope → style` map with static string keys.
    ///
    /// A convenience for tests; production builds themes through
    /// [`loader::parse_theme`]/[`Self::from_owned`]. A plain `HashMap` lets a
    /// test build `styles` from a literal.
    pub fn new(styles: HashMap<&'static str, ResolvedStyle>, default: ResolvedStyle) -> Self {
        let owned: FxHashMap<String, ResolvedStyle> = styles
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        Self::from_owned(owned, default)
    }

    /// Build a theme from a `scope → style` map with owned string keys.
    pub fn from_owned(styles: FxHashMap<String, ResolvedStyle>, default: ResolvedStyle) -> Self {
        let mut t = Self {
            raw: styles,
            baked: Vec::new(),
            default,
            ui: UiScopes::default(),
        };
        t.ui = t.compute_ui();
        t
    }

    /// Re-resolve every scope interned in `registry`, already baked or not.
    /// Production calls [`Self::bake_if_stale`] instead.
    pub fn bake(&mut self, registry: &ScopeRegistry) {
        self.baked.clear();
        self.baked.resize(registry.len(), None);
        for i in 0..registry.len() {
            self.baked[i] = self.lookup(registry.name_of(ScopeId(i as u16)));
        }
        self.ui = self.compute_ui();
    }

    /// Re-bake only if scopes were interned since the last [`Self::bake`] call.
    ///
    /// `ScopeRegistry` is append-only and `bake` sizes `baked` to
    /// `registry.len()`, so a length mismatch means new scopes are unbaked.
    /// Called at the end of `prepare_frame`, whose steps intern scopes
    /// lazily, and wherever the editor resolves styles outside `render`.
    pub fn bake_if_stale(&mut self, registry: &ScopeRegistry) {
        if self.baked.len() != registry.len() {
            self.bake(registry);
        }
    }

    /// Look up the style for an interned scope.
    ///
    /// Returns `default` for an id interned after the last bake, a
    /// programming error the debug-assert catches.
    #[inline]
    pub fn resolve(&self, id: ScopeId) -> ResolvedStyle {
        self.baked_style(id).unwrap_or(self.default)
    }

    /// The style an interned scope adds when layered over another style:
    /// like [`Self::resolve`], but empty rather than `default` when no name
    /// on the fallback chain has an entry, so the style underneath shows.
    #[inline]
    pub fn resolve_layer(&self, id: ScopeId) -> ResolvedStyle {
        self.baked_style(id).unwrap_or_default()
    }

    /// The baked style of an interned scope, `None` when nothing on its
    /// fallback chain has an entry or `id` was interned after the last bake.
    #[inline]
    fn baked_style(&self, id: ScopeId) -> Option<ResolvedStyle> {
        debug_assert!(
            (id.0 as usize) < self.baked.len(),
            "ScopeId {:?} is out of range: was bake() called after all providers were registered?",
            id
        );
        self.baked.get(id.0 as usize).copied().flatten()
    }

    /// Resolve a scope name via the dot-notation fallback chain.
    ///
    /// One map lookup per dot segment; the per-grapheme hot path interns the
    /// scope and uses [`Self::resolve`] instead.
    pub fn resolve_by_name(&self, scope: Scope) -> ResolvedStyle {
        self.resolve_raw(scope.0)
    }

    /// Return `true` if this scope name has an explicit entry in the raw map.
    pub fn raw_contains(&self, key: &str) -> bool {
        self.raw.contains_key(key)
    }

    /// Resolve `s` through its dot-notation fallback chain, or `default`.
    pub(crate) fn resolve_raw(&self, s: &str) -> ResolvedStyle {
        self.lookup(s).unwrap_or(self.default)
    }

    /// The style of the first name on `s`'s fallback chain with an entry.
    fn lookup(&self, s: &str) -> Option<ResolvedStyle> {
        fallback_chain(s).find_map(|key| self.raw.get(key).copied())
    }

    fn compute_ui(&self) -> UiScopes {
        let [
            (_, normal, normal_primary),
            (_, insert, insert_primary),
            (_, select, select_primary),
        ] = CURSOR_MODES;
        let (cursor, cursor_primary) = self.cursor_ladder(normal, normal_primary);
        let (cursor_insert, cursor_insert_primary) = self.cursor_ladder(insert, insert_primary);
        let (cursor_select, cursor_select_primary) = self.cursor_ladder(select, select_primary);
        UiScopes {
            cursor,
            cursor_insert,
            selection: self.resolve_raw(ui_scopes::SELECTION),
            cursorline: self.resolve_raw(ui_scopes::CURSORLINE_PRIMARY),
            virtual_text: self.resolve_raw(ui_scopes::VIRTUAL_TEXT),
            indent_guide: self.resolve_raw(ui_scopes::VIRTUAL_INDENT_GUIDE),
            invisible: self.resolve_raw(ui_scopes::VIRTUAL_INVISIBLE),
            whitespace: self.resolve_raw(ui_scopes::VIRTUAL_WHITESPACE),
            cursor_primary,
            cursor_insert_primary,
            cursor_select,
            cursor_select_primary,
            selection_primary: self.resolve_raw(ui_scopes::SELECTION_PRIMARY),
            background: self.resolve_raw(ui_scopes::BACKGROUND),
            window: self.resolve_raw(ui_scopes::WINDOW),
            window_focused: self.resolve_raw(ui_scopes::WINDOW_FOCUSED),
            context: self
                .raw
                .get(ui_scopes::CONTEXT)
                .copied()
                .unwrap_or_else(|| self.resolve_raw(ui_scopes::POPUP)),
        }
    }

    /// Build the (secondary, primary) cursor style pair for one mode, matching
    /// Helix's two fallback ladders:
    ///
    /// - secondary: `ui.cursor.<mode>` → `ui.cursor` → `ui.selection`
    /// - primary: `ui.cursor.primary.<mode>` → `ui.cursor.primary` →
    ///   `ui.cursor` → `ui` → `ui.selection`
    ///
    /// The bare `ui` rung is on the primary ladder only, as in Helix.
    fn cursor_ladder(
        &self,
        mode_scope: &'static str,
        primary_mode_scope: &'static str,
    ) -> (ResolvedStyle, ResolvedStyle) {
        let (secondary_ids, primary_ids) = cursor_ladder_ids(mode_scope, primary_mode_scope);
        (
            self.resolve_cursor_chain(&secondary_ids),
            self.resolve_cursor_chain(&primary_ids),
        )
    }

    /// Resolve a cursor scope from an ordered key list, first key with an
    /// entry winning, with no dot-notation fallback per key. Empty when no
    /// listed key is defined.
    ///
    /// Not `resolve_raw`: both ladders end at `ui.selection`, which
    /// dot-trimming never reaches, and the secondary ladder skips `ui`.
    fn resolve_cursor_chain(&self, keys: &[&str]) -> ResolvedStyle {
        keys.iter()
            .find_map(|k| self.raw.get(*k).copied())
            .unwrap_or_default()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_owned(FxHashMap::default(), ResolvedStyle::default())
    }
}

#[cfg(test)]
mod tests;
