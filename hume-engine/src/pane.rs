use std::str::FromStr;

use slotmap::SecondaryMap;

use crate::layout::gutter_width_for_line;
use crate::pipeline::BufferId;
use crate::providers::ProviderSet;
use crate::types::Selection;
use ropey::Rope;

// ---------------------------------------------------------------------------
// Viewport state  (per-pane scroll / size)
// ---------------------------------------------------------------------------

/// The scrolling and sizing state of one pane's viewport.
#[derive(Clone, Debug)]
pub struct ViewportState {
    /// First fully-visible buffer line.
    pub top_line: usize,
    /// How many display rows of `top_line`'s visual block — virtual `before`
    /// rows, the line's own wrap rows, then virtual `after` rows, in that
    /// order — have already scrolled past. Every row in the block is an
    /// equally skippable unit; nothing about `before`/`after` is special.
    pub top_row_offset: u16,
    /// Horizontal scroll in columns (0 when soft-wrap is on). A document
    /// column, not a terminal cell — widened past `u16` alongside
    /// `Grapheme::display_col` so scrolling isn't ceilinged at column 65535 on an
    /// unwrapped line.
    pub horizontal_offset: u32,
    /// Total width of the pane in terminal cells (gutter + content).
    pub width: u16,
    /// Total height of the pane in terminal cells.
    pub height: u16,
}

impl ViewportState {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            top_line: 0,
            top_row_offset: 0,
            horizontal_offset: 0,
            width,
            height,
        }
    }
}

// ---------------------------------------------------------------------------
// Scroll position  (per-pane, per-buffer scroll memory)
// ---------------------------------------------------------------------------

/// Saved scroll position for one (pane, buffer) pair.
///
/// Stored in `Pane::saved_scrolls` so each pane remembers where it was in a
/// buffer when it switches away. Restored by `recall_scroll` on switch-back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollPosition {
    pub top_line: usize,
    pub top_row_offset: u16,
    pub horizontal_offset: u32,
}

// ---------------------------------------------------------------------------
// Wrap mode
// ---------------------------------------------------------------------------

/// How the formatter handles lines that exceed the content width.
///
/// For `Soft`, `Word`, and `Indent`, `width: 0` is a sentinel meaning "wrap at
/// the content width" (pane width minus gutter). Call `WrapMode::resolve(content_width)`
/// to substitute a concrete column count before handing the mode to engine code.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WrapMode {
    /// No wrapping — horizontal scroll.
    #[default]
    None,
    /// Break at `width` columns (`0` = content width sentinel).
    Soft { width: u16 },
    /// Break at whitespace boundaries; prefer not to split words (`0` = content width sentinel).
    Word { width: u16 },
    /// Word wrap + indent continuation rows to match the line's indent level (`0` = content width sentinel).
    Indent { width: u16 },
}

/// The wrapping style used whenever a "sensible default" wrapping mode is
/// needed but nothing more specific applies: `EditorSettings::wrap_mode`'s
/// own default, and the last-resort fallback `hume-editor`'s
/// `pane_state::toggle_focused_wrap` pins to when `:wrap` turns wrapping on
/// but neither a saved pane pin nor the configured global style would
/// actually wrap.
pub const DEFAULT_WRAP_STYLE: WrapMode = WrapMode::Indent { width: 0 };

/// A pane's wrap-mode override for one buffer — the value type of
/// `Pane::wraps`.
///
/// Kept per (pane, buffer) rather than flat on `Pane` so the pin follows the
/// pane's *view of that buffer*, not the pane itself: switching to another
/// buffer resolves through that buffer's own override/global chain (letting
/// e.g. a per-language `on-language-set` default take effect), and switching
/// back restores this pane's pin for it, the same lifetime `ScrollPosition`
/// already has via `saved_scrolls`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WrapOverride {
    /// `None` means no override — the effective mode falls back to the
    /// buffer's, then the global default (`hume-editor`'s
    /// `commands::effective_wrap_mode` is the resolver — `hume-engine` has
    /// no dependency on `hume-editor` and so cannot resolve that chain
    /// itself). Stored raw (the `width: 0` sentinel unresolved) when `Some`;
    /// call `WrapMode::resolve(content_width)` on the resolved mode since
    /// content width depends on live pane geometry.
    pub mode: Option<WrapMode>,
    /// The *override* to restore when `:wrap` turns wrapping back on for
    /// this buffer — provenance, not a resolved value: `None` means the pane
    /// was inheriting (from the buffer/global setting) when it was last
    /// turned off, so toggling back on returns it to inheriting; `Some(m)`
    /// means it was explicitly pinned to `m` (`:set pane wrap-mode=…`), so
    /// toggling back on restores that exact pin. Never `Some(WrapMode::None)`
    /// — that would make toggle-on a no-op. Written by both
    /// `hume-editor`'s `pane_state::toggle_focused_wrap` (the toggle-off
    /// stash and the "inheriting but the inherited mode doesn't wrap"
    /// fallback) and `pane_state::set_focused_wrap_override` (synced to the
    /// pin whenever it wraps); copied across a same-buffer split by
    /// `inherit_view_state`.
    pub saved: Option<WrapMode>,
}

impl FromStr for WrapMode {
    type Err = String;

    /// Parse a wrap mode from a string.
    ///
    /// Accepted forms:
    /// - `none`                 — no wrapping
    /// - `soft` / `word` / `indent` — wrap at terminal width
    /// - `soft:N` / `word:N` / `indent:N` — wrap at column N (N=0 also means terminal width)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        if lower == "none" {
            return Ok(WrapMode::None);
        }
        // Bare keyword with no colon → sentinel width 0 (terminal width).
        if lower == "soft" {
            return Ok(WrapMode::Soft { width: 0 });
        }
        if lower == "word" {
            return Ok(WrapMode::Word { width: 0 });
        }
        if lower == "indent" {
            return Ok(WrapMode::Indent { width: 0 });
        }
        let (kind, rest) = lower.split_once(':').ok_or_else(|| {
            format!("invalid wrap-mode '{s}': expected none, soft[:N], word[:N], or indent[:N]")
        })?;
        let width: u16 = rest.parse().map_err(|_| {
            format!("invalid wrap-mode width in '{s}': expected a column count, got '{rest}'")
        })?;
        match kind {
            "soft" => Ok(WrapMode::Soft { width }),
            "word" => Ok(WrapMode::Word { width }),
            "indent" => Ok(WrapMode::Indent { width }),
            _ => Err(format!(
                "invalid wrap-mode kind '{kind}' in '{s}': expected soft, word, or indent"
            )),
        }
    }
}

impl std::fmt::Display for WrapMode {
    /// Canonical `kind:width` form (width always explicit, even the `0`
    /// sentinel) — round-trips through `FromStr`, which also accepts the
    /// bare-keyword shorthand this never emits.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Soft { width } => write!(f, "soft:{width}"),
            Self::Word { width } => write!(f, "word:{width}"),
            Self::Indent { width } => write!(f, "indent:{width}"),
        }
    }
}

impl WrapMode {
    /// Concrete wrap column, or `None` if wrapping is off.
    ///
    /// The caller must have already resolved the `width: 0` sentinel via
    /// `WrapMode::resolve(content_width)` — passing an unresolved sentinel is
    /// a bug and panics in both debug and release. The alternative (returning
    /// `None` or `Some(0)`) would silently format at column 0 in production.
    pub fn wrap_width(&self) -> Option<u16> {
        match self {
            WrapMode::None => None,
            WrapMode::Soft { width } | WrapMode::Word { width } | WrapMode::Indent { width } => {
                assert!(
                    *width != 0,
                    "wrap_width() received unresolved sentinel (width: 0) — \
                     call WrapMode::resolve(content_width) before reaching the engine",
                );
                Some(*width)
            }
        }
    }

    /// Replace the `width: 0` sentinel with a concrete column count.
    ///
    /// `WrapMode::None` and concrete non-zero widths pass through unchanged.
    /// The effective mode (see `hume-editor`'s `commands::effective_wrap_mode`)
    /// is raw (unresolved); call this at each use site — editor's
    /// `resolve_pane_settings`, scroll/cursor/mouse — passing that pane's
    /// `pane_width − gutter_width` (see `Pane::content_width`), since content
    /// width depends on live pane geometry and must re-derive on resize.
    pub fn resolve(self, content_width: u16) -> WrapMode {
        match self {
            WrapMode::Soft { width: 0 } => WrapMode::Soft {
                width: content_width,
            },
            WrapMode::Word { width: 0 } => WrapMode::Word {
                width: content_width,
            },
            WrapMode::Indent { width: 0 } => WrapMode::Indent {
                width: content_width,
            },
            other => other,
        }
    }

    pub fn is_wrapping(&self) -> bool {
        !matches!(self, WrapMode::None)
    }

    /// The bare-keyword wire-format strings `FromStr` accepts — the single
    /// source `:set global wrap-mode=<Tab>` completion mirrors. `FromStr` also
    /// accepts `soft:N`/`word:N`/`indent:N` suffix forms, which completion
    /// intentionally doesn't offer (the user types the column count).
    ///
    /// Struct-variant fields (`width`) mean this can't be derived from the
    /// enum itself — it's hand-maintained here, next to `FromStr`, so the two
    /// stay adjacent and a round-trip test can catch drift.
    pub const VALUES: &'static [&'static str] = &["none", "soft", "word", "indent"];
}

// ---------------------------------------------------------------------------
// Whitespace indicators
// ---------------------------------------------------------------------------

/// When to render a whitespace indicator for a particular whitespace type.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WhitespaceRender {
    /// Never render an indicator.
    #[default]
    None,
    /// Always render an indicator.
    All,
    /// Only render for trailing whitespace (before end-of-line).
    Trailing,
}

impl WhitespaceRender {
    /// The wire-format strings `FromStr` accepts — the single source
    /// `:set buffer whitespace-*=<Tab>` completion mirrors, so the two can
    /// never drift out of sync.
    pub const VALUES: &'static [&'static str] = &["none", "all", "trailing"];
}

impl FromStr for WhitespaceRender {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(WhitespaceRender::None),
            "all" => Ok(WhitespaceRender::All),
            "trailing" => Ok(WhitespaceRender::Trailing),
            _ => Err(format!(
                "invalid whitespace render '{s}': expected none, all, or trailing"
            )),
        }
    }
}

impl std::fmt::Display for WhitespaceRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WhitespaceRender::None => "none",
            WhitespaceRender::All => "all",
            WhitespaceRender::Trailing => "trailing",
        })
    }
}

/// Configuration for whitespace indicator rendering.
#[derive(Copy, Clone, Debug)]
pub struct WhitespaceConfig {
    pub space: WhitespaceRender,
    pub tab: WhitespaceRender,
    /// Whether to render the newline indicator. A newline is inherently
    /// always at end-of-line, so unlike `space`/`tab` there is no meaningful
    /// "trailing vs all" distinction here — just on/off.
    pub newline: bool,
    /// Character to show in place of a space when rendered. Not
    /// runtime-configurable — no `:set` key or Steel setter writes it.
    pub space_char: &'static str,
    /// Character to show at the start of a tab expansion.
    pub tab_char: &'static str,
    /// Character to show in place of a newline when rendered.
    pub newline_char: &'static str,
    /// Character to show in place of an invisible Unicode space (NBSP U+00A0,
    /// ideographic space U+3000) when rendered. Distinct from `space_char` so
    /// stray non-breaking spaces stand out from ordinary ones. Gated by the
    /// `space` render mode — no separate render axis.
    pub nbsp_char: &'static str,
}

impl Default for WhitespaceConfig {
    fn default() -> Self {
        Self {
            space: WhitespaceRender::None,
            tab: WhitespaceRender::None,
            newline: false,
            space_char: "·",
            tab_char: "→",
            newline_char: "⏎",
            nbsp_char: "⍽",
        }
    }
}

// ---------------------------------------------------------------------------
// Pane
// ---------------------------------------------------------------------------

/// A single editor pane — an independent view into a buffer.
pub struct Pane {
    /// Which buffer this pane views.
    pub buffer_id: BufferId,
    /// Scroll and size state.
    pub viewport: ViewportState,
    /// Per-buffer scroll memory: where this pane was when it last viewed each buffer.
    /// Populated by `remember_scroll` on buffer switch; restored by `recall_scroll`.
    pub saved_scrolls: SecondaryMap<BufferId, ScrollPosition>,
    /// All active selections, sorted by `head` position.
    /// (`SelectionSet` is start-sorted; `populate_sorted_sels` asserts head order.)
    pub selections: Vec<Selection>,
    /// Index of the primary selection within `selections`.
    pub primary_idx: usize,
    /// Registered providers for this pane.
    pub providers: ProviderSet,
    /// This pane's wrap-mode override, per buffer it has shown — a view
    /// property, not a document one: two panes on the same buffer may wrap
    /// differently, and this pane may pin a different style per buffer. See
    /// [`WrapOverride`]. Read/written through [`Pane::wrap`]/[`Pane::set_wrap`]
    /// rather than directly, mirroring `saved_scrolls`/`remember_scroll`.
    pub wraps: SecondaryMap<BufferId, WrapOverride>,
}

impl Pane {
    /// Create a new pane viewing `buffer_id`, with no wrap-mode override for
    /// any buffer — it inherits each buffer's/the global's effective mode
    /// until `:wrap` or `:set pane wrap-mode=…` pins one for that buffer.
    ///
    /// Callers that need custom providers should use `Pane { providers, ..Pane::new(bid) }`.
    pub fn new(buffer_id: BufferId) -> Self {
        Self {
            buffer_id,
            viewport: ViewportState::new(80, 24),
            saved_scrolls: SecondaryMap::new(),
            selections: vec![Selection { anchor: 0, head: 0 }],
            primary_idx: 0,
            providers: ProviderSet::new(),
            wraps: SecondaryMap::new(),
        }
    }

    /// Copy the view state a same-buffer split inherits from its source pane.
    ///
    /// Exhaustive destructure on purpose: adding a `Pane` field forces a
    /// compile-time inherit-or-skip decision here instead of a silent split
    /// bug.
    pub fn inherit_view_state(&mut self, src: &Pane) {
        let Pane {
            buffer_id: _, // pane identity, set by the split itself
            viewport,
            saved_scrolls,
            selections: _,  // engine render copy, repopulated every frame
            primary_idx: _, // ditto
            providers: _,   // allocated per pane in build_pane
            wraps,
        } = src;
        self.viewport = viewport.clone();
        self.saved_scrolls = saved_scrolls.clone();
        self.wraps = wraps.clone();
    }

    /// This pane's wrap-mode override for the buffer it currently views —
    /// `WrapOverride::default()` (no override, nothing to restore) if this
    /// pane has never pinned or toggled wrap for that buffer.
    pub fn wrap(&self) -> WrapOverride {
        self.wraps.get(self.buffer_id).copied().unwrap_or_default()
    }

    /// Write this pane's wrap-mode override for the buffer it currently views.
    pub fn set_wrap(&mut self, wrap: WrapOverride) {
        self.wraps.insert(self.buffer_id, wrap);
    }

    /// Width available for text after subtracting the gutter, clamped to at least 1.
    ///
    /// `last_line_idx` is the buffer's last ropey line index (used to size
    /// the line-number column — `hume_rope::lines::last_ropey_line`). Call this
    /// before `WrapMode::resolve` to get the concrete wrap column.
    pub fn content_width(&self, last_line_idx: usize) -> u16 {
        let gutter_w = gutter_width_for_line(self.providers.gutter_columns(), last_line_idx);
        self.viewport.width.saturating_sub(gutter_w).max(1)
    }

    /// Snapshot the current viewport scroll into `saved_scrolls` for `buffer_id`.
    pub fn remember_scroll(&mut self) {
        self.saved_scrolls.insert(
            self.buffer_id,
            ScrollPosition {
                top_line: self.viewport.top_line,
                top_row_offset: self.viewport.top_row_offset,
                horizontal_offset: self.viewport.horizontal_offset,
            },
        );
    }

    /// Restore the saved scroll for `id`, or reset to top on first visit.
    ///
    /// `last_content_line` is `id`'s *current* last content line index — the
    /// buffer may have shrunk since this scroll was saved (edited elsewhere
    /// while this pane viewed a different buffer), so `top_line` is clamped
    /// to it, the same bound `reload_buffer_in_place` applies
    /// (`hume-editor/src/editor/buffer/file_open.rs`).
    pub fn recall_scroll(&mut self, id: BufferId, last_content_line: usize) {
        let sp = self.saved_scrolls.get(id).copied().unwrap_or_default();
        self.viewport.top_line = sp.top_line.min(last_content_line);
        self.viewport.top_row_offset = sp.top_row_offset;
        self.viewport.horizontal_offset = sp.horizontal_offset;
    }

    /// Drop the saved scroll and wrap-mode override entries for `id` (called
    /// when the buffer is closed).
    pub fn forget_buffer(&mut self, id: BufferId) {
        self.saved_scrolls.remove(id);
        self.wraps.remove(id);
    }

    /// Line index of the primary selection head, resolved via the rope.
    ///
    /// Called once per frame from the pipeline — O(log n) rope lookup.
    /// Panics (debug and release) if `selections` is empty or `primary_idx`
    /// is out of range — both are violated invariants, not recoverable
    /// cases, so this fails loudly rather than defaulting to char 0 and
    /// hiding the bug.
    pub fn primary_head_line(&self, rope: &Rope) -> usize {
        let head_char = self
            .selections
            .get(self.primary_idx)
            .expect("pane selections empty or primary_idx out of range")
            .head;
        debug_assert!(
            head_char <= rope.len_chars(),
            "stale selection mirror: head {head_char} beyond rope len {} — \
             pane.selections is out of sync with pane.buffer_id",
            rope.len_chars()
        );
        rope.char_to_line(head_char)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
