pub mod diagnostic_scopes;
pub mod error;
pub mod loader;
pub mod scope_styles;
pub mod style_spec;
pub mod ui_scopes;
mod values;

use std::collections::HashMap;

use rustc_hash::FxHashMap;

use crate::theme::error::ThemeError;
use crate::theme::scope_styles::{Layered, ScopeStyles};
use crate::theme::style_spec::Palette;
use crate::types::{ResolvedStyle, Scope, ScopeId};

// ScopeRegistry

/// A new scope name was refused because the registry holds
/// [`ScopeRegistry::NAME_LIMIT`] names already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeLimitReached;

impl std::fmt::Display for ScopeLimitReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "too many distinct scope names in this session (limit {})",
            ScopeRegistry::NAME_LIMIT
        )
    }
}

impl std::error::Error for ScopeLimitReached {}

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

    /// How many distinct names [`ScopeRegistry::intern_name`] lets in. The
    /// ids from this count up stay free for HUME's own names, which
    /// [`ScopeRegistry::intern`] takes whatever the count.
    pub const NAME_LIMIT: usize = 65_536 - 256;

    fn next_id(&self) -> ScopeId {
        assert!(
            self.names.len() < u16::MAX as usize,
            "ScopeRegistry overflow: more than 65 535 distinct scope names"
        );
        ScopeId(self.names.len() as u16)
    }

    fn insert_new(&mut self, name: &str) -> ScopeId {
        let id = self.next_id();
        let boxed: Box<str> = Box::from(name);
        self.names.push(boxed.clone());
        self.map.insert(boxed, id);
        id
    }

    /// Return the [`ScopeId`] for one of HUME's own scope names, interning it
    /// if new. Never refused: a name from a script or a query file goes
    /// through [`ScopeRegistry::intern_name`].
    pub fn intern(&mut self, name: &str) -> ScopeId {
        match self.map.get(name) {
            Some(&id) => id,
            None => self.insert_new(name),
        }
    }

    /// Return the [`ScopeId`] for a name that came from a script or a query
    /// file. A name already interned always succeeds; a new one is refused
    /// once [`ScopeRegistry::NAME_LIMIT`] names are interned.
    pub fn intern_name(&mut self, name: &str) -> Result<ScopeId, ScopeLimitReached> {
        if let Some(&id) = self.map.get(name) {
            return Ok(id);
        }
        if self.names.len() >= Self::NAME_LIMIT {
            return Err(ScopeLimitReached);
        }
        Ok(self.insert_new(name))
    }

    /// Intern placeholder names until the registry holds
    /// [`ScopeRegistry::NAME_LIMIT`] of them.
    #[cfg(any(test, feature = "test-util"))]
    pub fn fill_to_limit(&mut self) {
        while self.names.len() < Self::NAME_LIMIT {
            let filler = format!("scope-filler.{}", self.names.len());
            self.intern_name(&filler)
                .expect("below the limit, so never refused");
        }
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

/// Which scopes [`Theme::apply_scope_styles`] layers.
#[derive(Clone, Copy, Debug)]
pub enum Restyle {
    /// Every scope, replacing the styles layered before.
    All,
    /// The styles set since they were last marked shown, in the order
    /// first set; every other style keeps its layer.
    Unshown,
}

/// Maps semantic scope names to concrete styles.
///
/// # Two-phase API
///
/// 1. Construct with [`Theme::from_owned`] (or [`Theme::default`]).
///    `theme.ui()` is usable at once.
///
/// 2. Bake against the [`ScopeRegistry`] ([`Theme::bake_if_stale`]) before
///    [`Theme::resolve`] reads an interned scope.
///
/// Scope names fall back by dot-notation: `keyword.function` → `keyword` →
/// default.
///
/// A name's entry is the first of `above`, `raw` and `below` to have one
/// (`entry`), so a script's style sits above or beneath the theme's per
/// name, never per fallback chain.
pub struct Theme {
    /// The theme file's map, never mutated after construction.
    raw: FxHashMap<String, ResolvedStyle>,
    /// The palette `raw` was resolved against.
    palette: Palette,
    /// The user's script styles, set by [`Self::apply_scope_styles`].
    above: FxHashMap<String, ResolvedStyle>,
    /// The other origins' script styles, set by [`Self::apply_scope_styles`].
    below: FxHashMap<String, ResolvedStyle>,
    /// Indexed by [`ScopeId`]; `None` when no name on the scope's fallback
    /// chain has an entry.
    baked: Vec<Option<ResolvedStyle>>,
    default: ResolvedStyle,
    ui: UiScopes,
}

impl Theme {
    /// Build a theme from a `scope → style` map with static string keys.
    ///
    /// A convenience for tests; production builds themes through
    /// [`loader::parse_theme`]/[`Self::from_owned`]. A plain `HashMap` lets a
    /// test build `styles` from a literal.
    pub fn new(styles: HashMap<&'static str, ResolvedStyle>) -> Self {
        let owned: FxHashMap<String, ResolvedStyle> = styles
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        Self::from_owned(owned, Palette::default())
    }

    /// Build a theme from a `scope → style` map with owned string keys,
    /// resolved against `palette`.
    pub fn from_owned(styles: FxHashMap<String, ResolvedStyle>, palette: Palette) -> Self {
        let mut t = Self {
            raw: styles,
            palette,
            above: FxHashMap::default(),
            below: FxHashMap::default(),
            baked: Vec::new(),
            default: ResolvedStyle::default(),
            ui: UiScopes::default(),
        };
        t.derive_from_entries();
        t
    }

    /// Layer the scopes `which` selects from `styles` over and under the
    /// theme's own entries, and resolve their colors against the theme's
    /// palette. Returns a warning for each color that does not resolve, in
    /// the order the scopes are layered; its field stays unset. Scopes
    /// baked before re-bake at the next [`Self::bake_if_stale`].
    pub fn apply_scope_styles(&mut self, styles: &ScopeStyles, which: Restyle) -> Vec<ThemeError> {
        let mut warnings = Vec::new();
        match which {
            Restyle::All => {
                self.above.clear();
                self.below.clear();
                for layered in styles.layered() {
                    self.layer_scope(layered, &mut warnings);
                }
            }
            Restyle::Unshown => {
                for layered in styles.unshown() {
                    self.layer_scope(layered, &mut warnings);
                }
            }
        }
        self.baked.clear();
        self.derive_from_entries();
        warnings
    }

    /// The style used when no scope matches: the `ui.text` entry, or empty.
    #[inline]
    pub fn default_style(&self) -> ResolvedStyle {
        self.default
    }

    /// Pre-resolved styles for hot-path UI scopes.
    #[inline]
    pub fn ui(&self) -> &UiScopes {
        &self.ui
    }

    /// Recompute what the entries determine: `default` and the
    /// pre-resolved `ui`. `ui.text` (Helix's base-foreground convention)
    /// folds into `default`, the style every cell starts from (see
    /// `style::apply_styles`). Without it, plain text has `fg: None` and
    /// renders in the terminal's own default colour, which the pane dim has
    /// no numeric value to blend, so it stays at full strength in an
    /// unfocused pane.
    fn derive_from_entries(&mut self) {
        self.default = self.entry(ui_scopes::TEXT).unwrap_or_default();
        self.ui = self.compute_ui();
    }

    /// Set `layered.scope`'s entry in `above` and `below` from its styles.
    fn layer_scope(&mut self, layered: Layered<'_>, warnings: &mut Vec<ThemeError>) {
        for (spec, layer) in [
            (layered.above_theme, &mut self.above),
            (layered.below_theme, &mut self.below),
        ] {
            let Some(spec) = spec else { continue };
            let (style, color_warnings) = spec.resolve(layered.scope, &self.palette);
            warnings.extend(color_warnings);
            layer.insert(layered.scope.to_owned(), style);
        }
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

    /// Whether this exact scope name has an entry, from the theme or a script.
    pub fn has_entry(&self, key: &str) -> bool {
        self.entry(key).is_some()
    }

    /// The entry for this exact scope name: the user's script style, else
    /// the theme's, else another origin's script style.
    fn entry(&self, key: &str) -> Option<ResolvedStyle> {
        self.above
            .get(key)
            .or_else(|| self.raw.get(key))
            .or_else(|| self.below.get(key))
            .copied()
    }

    /// Resolve `s` through its dot-notation fallback chain, or `default`.
    pub(crate) fn resolve_raw(&self, s: &str) -> ResolvedStyle {
        self.lookup(s).unwrap_or(self.default)
    }

    /// The style of the first name on `s`'s fallback chain with an entry.
    fn lookup(&self, s: &str) -> Option<ResolvedStyle> {
        fallback_chain(s).find_map(|key| self.entry(key))
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
                .entry(ui_scopes::CONTEXT)
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
        keys.iter().find_map(|k| self.entry(k)).unwrap_or_default()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_owned(FxHashMap::default(), Palette::default())
    }
}

#[cfg(test)]
mod tests;
