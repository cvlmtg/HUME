//! Names of the theme scopes HUME's UI chrome (statusline, tabline, popup,
//! menu, drawer, gutter, window chrome) resolves by fixed name — mirrors
//! `CURSOR_MODES`/`cursor_ladder_ids` (`theme/mod.rs`) one level up: a
//! second hand-copy of any of these, anywhere in the workspace or in
//! `tools/theme-editor/src/data.js`'s catalog, is exactly the drift this
//! module exists to close off. `hume-engine/src/theme/loader/vocabulary.rs`
//! renders [`ALL`] into `tools/theme-editor/src/lib/vocabulary.generated.js`.
//!
//! `ui.cursorline` is deliberately absent: it is resolved only as a
//! dot-fallback parent for `ui.cursorline.primary`, never looked up by its
//! own name (same rule `tools/theme-editor/tests/coverage.test.js`'s
//! `FALLBACK_ONLY_CHROME` documents on the JS side), so it isn't a name any
//! Rust call site resolves.
//!
//! Cursor, Virtual, and Diagnostic scope names are out of scope for this
//! table — they are not yet consolidated the same way.

pub const BACKGROUND: &str = "ui.background";
pub const TEXT: &str = "ui.text";
pub const TEXT_FOCUS: &str = "ui.text.focus";
pub const SELECTION: &str = "ui.selection";
pub const SELECTION_PRIMARY: &str = "ui.selection.primary";
pub const LINENR: &str = "ui.linenr";
pub const LINENR_SELECTED: &str = "ui.linenr.selected";
pub const STATUSLINE: &str = "ui.statusline";
pub const STATUSLINE_NORMAL: &str = "ui.statusline.normal";
pub const STATUSLINE_INSERT: &str = "ui.statusline.insert";
pub const STATUSLINE_SELECT: &str = "ui.statusline.select";
pub const STATUSLINE_SEARCH: &str = "ui.statusline.search";
pub const STATUSLINE_COMMAND: &str = "ui.statusline.command";
pub const STATUSLINE_SIFT: &str = "ui.statusline.sift";
pub const STATUSLINE_SEPARATOR: &str = "ui.statusline.separator";
pub const POPUP: &str = "ui.popup";
pub const POPUP_SCROLL: &str = "ui.popup.scroll";
pub const MENU: &str = "ui.menu";
pub const MENU_SELECTED: &str = "ui.menu.selected";
pub const MENU_SCROLL: &str = "ui.menu.scroll";
pub const WINDOW: &str = "ui.window";
pub const WINDOW_FOCUSED: &str = "ui.window.focused";
/// HUME's own tab bar (Vim-style saved window layouts) — not the same
/// feature as Helix's per-buffer `ui.bufferline` strip, hence its own scope
/// name; see [`BUFFERLINE`]'s doc for the fallback relationship between them.
pub const TABLINE: &str = "ui.tabline";
pub const TABLINE_ACTIVE: &str = "ui.tabline.active";
/// Helix's own scope name, never resolved directly by HUME — read only as
/// `TablineColors::from_theme`'s (`hume-editor/src/statusline/colors.rs`)
/// fallback when a theme defines `ui.bufferline` (every shipped Helix theme
/// does) but not `ui.tabline` (no Helix theme does, since it's HUME's own
/// addition), so an unmodified Helix theme still colours the tab bar instead
/// of leaving it unstyled.
pub const BUFFERLINE: &str = "ui.bufferline";
pub const BUFFERLINE_ACTIVE: &str = "ui.bufferline.active";
pub const DRAWER: &str = "ui.drawer";
pub const CURSORLINE_PRIMARY: &str = "ui.cursorline.primary";

/// Every name above, in the theme editor catalog's display order. The
/// vocabulary generator's own source of truth for what to emit.
pub const ALL: &[&str] = &[
    BACKGROUND,
    TEXT,
    TEXT_FOCUS,
    SELECTION,
    SELECTION_PRIMARY,
    LINENR,
    LINENR_SELECTED,
    STATUSLINE,
    STATUSLINE_NORMAL,
    STATUSLINE_INSERT,
    STATUSLINE_SELECT,
    STATUSLINE_SEARCH,
    STATUSLINE_COMMAND,
    STATUSLINE_SIFT,
    STATUSLINE_SEPARATOR,
    POPUP,
    POPUP_SCROLL,
    MENU,
    MENU_SELECTED,
    MENU_SCROLL,
    WINDOW,
    WINDOW_FOCUSED,
    TABLINE,
    TABLINE_ACTIVE,
    BUFFERLINE,
    BUFFERLINE_ACTIVE,
    DRAWER,
    CURSORLINE_PRIMARY,
];
