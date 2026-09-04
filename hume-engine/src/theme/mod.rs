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
    /// Selection-head highlight (Normal, Visual, … modes). Named `cursor` for Helix theme compat.
    pub cursor: ResolvedStyle,
    /// Selection-head highlight in Insert mode. Named `cursor_insert` for Helix theme compat.
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
    /// Primary selection-head highlight (Normal/Extend/… modes). Falls back to `cursor` if unset.
    pub cursor_primary: ResolvedStyle,
    /// Primary selection-head highlight in Insert mode. Falls back to `cursor_insert` if unset.
    pub cursor_insert_primary: ResolvedStyle,
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
        let mut cur = s;
        loop {
            if let Some(&style) = self.raw.get(cur) {
                return style;
            }
            match cur.rfind('.') {
                Some(dot) => cur = &cur[..dot],
                None => return self.default,
            }
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────

    fn compute_ui(&self) -> UiScopes {
        UiScopes {
            cursor: self.resolve_raw("ui.cursor"),
            // Insert and block cursors are distinct shapes, not a specificity hierarchy.
            // No dot-notation parent fallback: absent → empty (no-op layer) so the real
            // terminal bar cursor shows through the head cell.
            cursor_insert: self.resolve_cursor_chain(&["ui.cursor.insert"]),
            selection: self.resolve_raw("ui.selection"),
            cursorline: self.resolve_raw("ui.cursorline.primary"),
            virtual_text: self.resolve_raw("ui.virtual"),
            // dot-fallback to ui.virtual for themes that don't define the guide explicitly
            indent_guide: self.resolve_raw("ui.virtual.indent-guide"),
            invisible: self.resolve_raw("ui.virtual.invisible"),
            whitespace: self.resolve_raw("ui.virtual.whitespace"),
            // Primary cursor: dot-notation fallback ui.cursor.primary → ui.cursor is correct.
            cursor_primary: self.resolve_raw("ui.cursor.primary"),
            // Primary insert cursor: prefer ui.cursor.primary.insert, then ui.cursor.insert,
            // then empty. No fallback to the block ui.cursor (same rationale as cursor_insert).
            cursor_insert_primary: self
                .resolve_cursor_chain(&["ui.cursor.primary.insert", "ui.cursor.insert"]),
            // Primary selection: dot-notation fallback ui.selection.primary → ui.selection is correct.
            selection_primary: self.resolve_raw("ui.selection.primary"),
            background: self.resolve_raw("ui.background"),
            window: self.resolve_raw("ui.window"),
            window_focused: self.resolve_raw("ui.window.focused"),
        }
    }

    /// Resolve a cursor scope from an explicit, ordered key list with NO dot-notation
    /// parent fallback. Insert and block cursors are distinct shapes, not a specificity
    /// hierarchy: `ui.cursor.insert` must never inherit `ui.cursor`'s block background.
    /// Returns an empty (all-`None`) style when no listed key is defined.
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
