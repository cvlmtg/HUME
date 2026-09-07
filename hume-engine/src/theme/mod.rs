pub mod error;
pub mod loader;

use std::collections::HashMap;

use rustc_hash::FxHashMap;

use crate::types::{ResolvedStyle, Scope, ScopeId};

// ---------------------------------------------------------------------------
// ScopeRegistry
// ---------------------------------------------------------------------------

/// Maps scope name strings to compact [`ScopeId`] integers.
///
/// Two registration entry points share one map and interning path:
/// - [`ScopeRegistry::intern`] for `&'static str` — used by engine builtins and theme loaders.
/// - [`ScopeRegistry::intern_runtime`] for `&str` — used by Steel-loaded language configs
///   where scope names are runtime strings.
///
/// Interning is cold (construction, a runtime language/grammar load,
/// mid-session plugin activation — never the per-grapheme render path), so a
/// single owned-key map costs one extra allocation per newly-interned static
/// scope over a `&'static str` fast path, in exchange for one map and one
/// interning method instead of two. The total number of distinct scopes is
/// bounded by `u16::MAX`.
///
/// [`Theme::bake_if_stale`], called unconditionally from `prepare_frame`
/// twice every frame (once before the frame's own steps run, once after),
/// re-bakes whenever new scopes were interned since the last bake, so no
/// caller needs to bake manually after interning — including a scope a
/// frame's own steps intern partway through, which the first call alone
/// can't see. After baking, [`Theme::resolve`] is an O(1) `Vec` index.
///
/// Lives on [`crate::pipeline::EngineView`] so it outlives all providers.
#[derive(Default)]
pub struct ScopeRegistry {
    /// Name → id. Owned keys regardless of registration path — a `&str`
    /// lookup costs the same whether the stored key came from a `&'static`
    /// literal or a runtime string.
    map: FxHashMap<Box<str>, ScopeId>,
    /// Combined name table; index is the `ScopeId`. Both registration paths
    /// push here so `name_of` works uniformly.
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

    /// Intern `name`, returning its existing [`ScopeId`] or allocating a new one.
    fn intern_str(&mut self, name: &str) -> ScopeId {
        if let Some(&id) = self.map.get(name) {
            return id;
        }
        let id = self.next_id();
        let boxed: Box<str> = Box::from(name);
        self.names.push(boxed.clone());
        self.map.insert(boxed, id);
        id
    }

    /// Return the [`ScopeId`] for a `&'static str` scope name, interning if new.
    pub fn intern(&mut self, name: &'static str) -> ScopeId {
        self.intern_str(name)
    }

    /// Return the [`ScopeId`] for any `&str` scope name, interning if new.
    ///
    /// Use this for runtime-generated scope names (e.g. tree-sitter capture
    /// names from a Steel-loaded `(register-grammar!)`).
    pub fn intern_runtime(&mut self, name: &str) -> ScopeId {
        self.intern_str(name)
    }

    /// Look up an already-interned scope without inserting.
    pub fn get(&self, name: &str) -> Option<ScopeId> {
        self.map.get(name).copied()
    }

    /// Reverse-lookup: return the name interned as `id`.
    ///
    /// Panics if `id` is out of range (i.e. not produced by this registry).
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
/// Shared so the walk exists once: [`Theme::resolve_raw`] takes the first name
/// with an entry, while `:theme-debug` reports every name that has one.
pub fn fallback_chain(scope: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(scope), |cur| cur.rfind('.').map(|dot| &cur[..dot]))
}

/// The (secondary, primary) rung lists [`Theme::cursor_ladder`] resolves, for
/// one mode's own scope names — see that method's doc for the ladder shape.
///
/// Exposed as a shared function rather than left private so `:theme-debug`
/// can report which rung actually matched, without hand-copying the list: a
/// second copy would silently drift past the exact order and skip-rules
/// `hume-engine/src/theme/tests.rs` pins for it.
pub fn cursor_ladder_ids(
    mode_scope: &'static str,
    primary_mode_scope: &'static str,
) -> ([&'static str; 3], [&'static str; 5]) {
    (
        [mode_scope, "ui.cursor", "ui.selection"],
        [
            primary_mode_scope,
            "ui.cursor.primary",
            "ui.cursor",
            "ui",
            "ui.selection",
        ],
    )
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
    /// Secondary selection-head highlight in Normal mode. Falls back through
    /// `ui.cursor.normal` → `ui.cursor` → `ui.selection`, matching Helix's own
    /// `cursor_scope` in `doc_selection_highlights`.
    pub cursor: ResolvedStyle,
    /// Secondary selection-head highlight in Insert mode. Named `cursor_insert`
    /// for Helix theme compat. Falls back through `ui.cursor.insert` →
    /// `ui.cursor` → `ui.selection` — a secondary head is always painted (it
    /// has no real terminal cursor to fall back on), regardless of the
    /// configured Insert cursor shape.
    pub cursor_insert: ResolvedStyle,
    /// Selection highlight.
    pub selection: ResolvedStyle,
    /// Background tint applied to every grapheme on the primary selection head's line.
    pub cursorline: ResolvedStyle,
    /// Style for virtual / filler text (tilde rows, virtual lines).
    pub virtual_text: ResolvedStyle,
    /// Indent-guide column markers.
    pub indent_guide: ResolvedStyle,
    /// `ui.virtual.invisible` — the `<200b>` stand-in a cluster the terminal
    /// must not be shown is drawn as. Dot-fallback reaches `ui.virtual`, so a
    /// theme that defines nothing still renders these muted rather than as
    /// ordinary text.
    pub invisible: ResolvedStyle,
    /// `ui.virtual.whitespace` — an opted-in whitespace indicator glyph
    /// (`·`, `→`, `⏎`, `⍽`). Dot-fallback reaches `ui.virtual`. Never applied
    /// to `CellContent::TabFill` — the blank a tab renders as with its
    /// indicator off must stay unstyled regardless of this scope.
    pub whitespace: ResolvedStyle,
    /// Primary selection-head highlight in Normal mode. Falls back through
    /// `ui.cursor.primary.normal` → `ui.cursor.primary` → `ui.cursor` → `ui` →
    /// `ui.selection`. Painted only when the resolved cursor shape for the
    /// current mode is `Block` (Normal always is) — see
    /// `PaneRenderSettings::primary_cursor_is_block`.
    pub cursor_primary: ResolvedStyle,
    /// Primary selection-head highlight in Insert mode. Same ladder as
    /// [`Self::cursor_primary`], rooted at `ui.cursor.primary.insert`. Painted
    /// only when `cursor-shape-insert` is `block`; for `bar`/`underline` the
    /// real terminal cursor is the sole indicator and this style is unused.
    pub cursor_insert_primary: ResolvedStyle,
    /// Secondary selection-head highlight in Extend mode — HUME's name for Helix's
    /// Select mode (HUME's own `Select` mode is the `s` regex prompt, unrelated to
    /// this scope, and maps to the Normal ladder instead). Falls back through
    /// `ui.cursor.select` → `ui.cursor` → `ui.selection`.
    pub cursor_select: ResolvedStyle,
    /// Primary selection-head highlight in Extend mode. Same ladder shape as
    /// [`Self::cursor_primary`], rooted at `ui.cursor.primary.select`. Always
    /// painted — Extend has no configurable shape, and is hardwired `Block`.
    pub cursor_select_primary: ResolvedStyle,
    /// Primary selection highlight. Falls back to `selection` if unset.
    pub selection_primary: ResolvedStyle,
    /// Pane background colour. Painted behind all content cells so the theme bg shows
    /// through trailing whitespace and empty lines rather than the terminal default.
    pub background: ResolvedStyle,
    /// Seam divider drawn between sibling panes (`ui.window`, Helix convention).
    pub window: ResolvedStyle,
    /// Seam divider segments adjacent to the focused pane. Dot-notation
    /// fallback to `ui.window` when unset, same as the other `.primary` scopes.
    pub window_focused: ResolvedStyle,
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
    /// `resolve_by_name()`. Never mutated after construction.
    raw: FxHashMap<String, ResolvedStyle>,
    /// Per-[`ScopeId`] resolved styles. Populated by `bake()`.
    /// `baked[id.0]` is the style for the scope with that id.
    baked: Vec<ResolvedStyle>,
    /// Default style used when no scope matches.
    pub default: ResolvedStyle,
    /// Pre-resolved styles for hot-path UI scopes. Always valid (computed in
    /// construction and re-computed in `bake()`).
    pub ui: UiScopes,
}

impl Theme {
    /// Build a theme from a `scope → style` map with static string keys.
    ///
    /// Test-only convenience constructor (no production caller — production
    /// builds themes from TOML via [`loader::parse_theme`]/[`Self::from_owned`]).
    /// Takes a plain `HashMap`, not `FxHashMap`: the map is consumed once via
    /// `.collect()` into the real (`FxHashMap`-backed) storage below, so its
    /// hasher never matters — keeping it a plain `HashMap` lets every test
    /// build `styles` with an ordinary literal instead of `FxHashMap::default()`.
    /// `ui` fields are resolved immediately from `styles`, so callers can use
    /// `theme.ui.*` before calling `bake()`.
    pub fn new(styles: HashMap<&'static str, ResolvedStyle>, default: ResolvedStyle) -> Self {
        let owned: FxHashMap<String, ResolvedStyle> = styles
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        Self::from_owned(owned, default)
    }

    /// Build a theme from a `scope → style` map with owned string keys.
    ///
    /// Used by [`loader::load_theme`] which produces `String` scope names from
    /// TOML parsing.
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

    /// Pre-resolve all scopes interned in `registry` into a flat `Vec`.
    ///
    /// After baking, [`Self::resolve`] is an O(1) `Vec` index — no hashing.
    ///
    /// Unconditional — always re-resolves every interned scope, even ones
    /// already baked. In production, prefer [`Self::bake_if_stale`], which skips the
    /// work when nothing changed; it's what `prepare_frame` calls every frame.
    /// Call `bake` directly only when you need an immediate, unconditional
    /// re-bake against a specific registry (e.g. tests).
    pub fn bake(&mut self, registry: &ScopeRegistry) {
        self.baked.clear();
        self.baked.resize(registry.len(), self.default);
        for i in 0..registry.len() {
            self.baked[i] = self.resolve_raw(registry.name_of(ScopeId(i as u16)));
        }
        // Re-populate ui in case bake() is called after construction (idempotent).
        self.ui = self.compute_ui();
    }

    /// Re-bake only if scopes were interned since the last [`Self::bake`] call.
    ///
    /// `ScopeRegistry` is append-only and `bake` sizes `baked` to exactly
    /// `registry.len()`, so `baked.len() != registry.len()` is precisely "new
    /// scopes are unbaked". Called twice per frame from `prepare_frame` —
    /// once up front (catching up on interning since the last frame) and
    /// once at the very end (catching this frame's own steps, several of
    /// which intern scopes lazily as they run) — so no other call site needs
    /// to remember to bake after interning, and nothing this frame resolves
    /// can outrun `baked`. Cheap when nothing changed: one `usize` compare.
    pub fn bake_if_stale(&mut self, registry: &ScopeRegistry) {
        if self.baked.len() != registry.len() {
            self.bake(registry);
        }
    }

    /// Look up the style for an interned scope.
    ///
    /// **O(1)** after [`Self::bake`]. Returns `default` for IDs created after the
    /// last `bake()` call (a programming error — debug-assert helps catch it).
    #[inline]
    pub fn resolve(&self, id: ScopeId) -> ResolvedStyle {
        debug_assert!(
            (id.0 as usize) < self.baked.len(),
            "ScopeId {:?} is out of range — was bake() called after all providers were registered?",
            id
        );
        self.baked
            .get(id.0 as usize)
            .copied()
            .unwrap_or(self.default)
    }

    /// Resolve a scope name via the dot-notation fallback chain.
    ///
    /// **Slow path** (one `FxHashMap` lookup per dot segment). Use this only for
    /// non-hot call sites such as gutter-cell rendering (~100 calls/frame).
    /// On the per-grapheme hot path, intern the scope and use [`Self::resolve`].
    pub fn resolve_by_name(&self, scope: Scope) -> ResolvedStyle {
        self.resolve_raw(scope.0)
    }

    /// Return `true` if this scope name has an explicit entry in the raw map.
    ///
    /// Used by `:theme-debug` to build the dot-notation fallback chain display,
    /// and by `EditorColors::from_theme` to decide whether the statusline
    /// separator should fall back to the row's own resolved style instead of
    /// walking the dot-notation chain to the untinted base scope.
    pub fn raw_contains(&self, key: &str) -> bool {
        self.raw.contains_key(key)
    }

    /// Walk the dot-notation fallback chain for `s`, returning the resolved style.
    ///
    /// `"keyword.function"` tries `"keyword.function"`, then `"keyword"`, then returns `default`.
    pub(crate) fn resolve_raw(&self, s: &str) -> ResolvedStyle {
        fallback_chain(s)
            .find_map(|key| self.raw.get(key).copied())
            .unwrap_or(self.default)
    }

    // ── Private helpers ──────────────────────────────────────────────────

    fn compute_ui(&self) -> UiScopes {
        let (cursor, cursor_primary) =
            self.cursor_ladder("ui.cursor.normal", "ui.cursor.primary.normal");
        let (cursor_insert, cursor_insert_primary) =
            self.cursor_ladder("ui.cursor.insert", "ui.cursor.primary.insert");
        let (cursor_select, cursor_select_primary) =
            self.cursor_ladder("ui.cursor.select", "ui.cursor.primary.select");
        UiScopes {
            cursor,
            cursor_insert,
            selection: self.resolve_raw("ui.selection"),
            cursorline: self.resolve_raw("ui.cursorline.primary"),
            virtual_text: self.resolve_raw("ui.virtual"),
            // dot-fallback to ui.virtual for themes that don't define the guide explicitly
            indent_guide: self.resolve_raw("ui.virtual.indent-guide"),
            invisible: self.resolve_raw("ui.virtual.invisible"),
            whitespace: self.resolve_raw("ui.virtual.whitespace"),
            cursor_primary,
            cursor_insert_primary,
            cursor_select,
            cursor_select_primary,
            // Primary selection: dot-notation fallback ui.selection.primary → ui.selection is correct.
            selection_primary: self.resolve_raw("ui.selection.primary"),
            background: self.resolve_raw("ui.background"),
            window: self.resolve_raw("ui.window"),
            window_focused: self.resolve_raw("ui.window.focused"),
        }
    }

    /// Build the (secondary, primary) cursor style pair for one mode's own
    /// scope name, matching Helix's two fallback ladders in
    /// `doc_selection_highlights` exactly:
    ///
    /// - secondary: `ui.cursor.<mode>` → `ui.cursor` → `ui.selection`
    /// - primary: `ui.cursor.primary.<mode>` → `ui.cursor.primary` →
    ///   `ui.cursor` → `ui` → `ui.selection`
    ///
    /// The bare `ui` rung exists only on the primary ladder — Helix's own
    /// asymmetry (`base_primary_cursor_scope` is the one prefix-walking
    /// lookup in the function; every other rung, on both ladders, is exact).
    /// `mode_scope`/`primary_mode_scope` are spelled out at each call site
    /// rather than built from a mode string, so every rung stays a `&'static
    /// str` literal with no runtime formatting.
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

    /// Resolve a cursor scope from an explicit, ordered key list — first key with
    /// an entry wins — with NO dot-notation fallback of its own. Returns an empty
    /// (all-`None`) style when no listed key is defined.
    ///
    /// Needed instead of `resolve_raw` because a primary cursor's chain must
    /// reach a key dot-trimming *skips*: `ui.cursor.primary.<mode>` →
    /// `ui.cursor.primary` → `ui.cursor` (Helix's own order), where trimming
    /// the leading key by dots would jump from `ui.cursor.primary` straight
    /// past the mode's own `ui.cursor.<mode>` rung.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
