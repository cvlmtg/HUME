//! A pane's geometry between and during frames: the screen layout, each
//! pane's size, gutter and content widths, its visible line range, and the
//! layout key whose change reveals the cursor.

use hume_core::{BufferId, PaneId};
use hume_engine::builtins::sign_column::SignColumn;
use hume_engine::folds::HiddenLines;
use hume_engine::layout::PaneGeometry;
use hume_engine::pane::ViewGeometry;
use hume_engine::pipeline::{ChromeRows, EngineView, PaneSize, ScreenLayout};
use hume_engine::providers::{GutterInputs, SyntaxSource};
use hume_engine::scope_headers::{ContextSettings, ScopeHeaderInputs};
use hume_treesitter::syntax::Syntax;

use super::input_stack::{FocusedBox, ResolvedPopupLayout};
use super::{EditorState, commands};

/// Every input a pane's layout depends on: its
/// [`FormatKey`](hume_engine::display_lines::line_store::FormatKey), the lines
/// its closed folds hide, content width, viewport height and scroll-margin,
/// which decide where the cursor's display line falls, and the buffer's
/// scope-header settings and, while
/// headers are on, its committed syntax layers, which decide how many header
/// rows the top margin grows by.
///
/// `FormatKey` leaves out content width because formatting sees it only
/// through the wrap mode it resolves. This key keeps it because a growing
/// gutter narrows the wrap column just like a resize, and reveal cares.
///
/// `frame::reveal_due_cursor` stores this in
/// `PaneBufferState::last_layout_key` and compares it against a fresh one
/// each time it runs (resize, wrap-mode change, buffer switch, decoration
/// generation, a parse landing while headers are on, a fold opening or
/// closing, and any content edit to the buffer, since `buffer_tag` includes
/// `generation`). A change reveals only a pane that isn't
/// [`PaneBufferState::parked`](crate::editor::pane_state::PaneBufferState::parked):
/// a parked pane's own edit still reveals it, through `reveal_pending`
/// instead, but a change from elsewhere (a sibling pane's edit, a
/// decoration refresh) leaves it where it is.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::editor) struct LayoutKey {
    format: hume_engine::display_lines::line_store::FormatKey,
    /// The lines the pane's closed folds hide.
    hidden: HiddenLines,
    content_width: hume_engine::pipeline::ContentWidth,
    size: hume_engine::pipeline::PaneSize,
    scroll_margin: usize,
    context: ContextSettings,
    /// [`Syntax::committed_gen`], `None` without committed layers or with
    /// headers off.
    layers: Option<u64>,
}

impl LayoutKey {
    /// The part of `self` a `DisplayLineMap` is built from.
    pub(in crate::editor) fn format_key(
        &self,
    ) -> hume_engine::display_lines::line_store::FormatKey {
        self.format
    }

    /// The lines a `DisplayLineMap` of this pane steps over.
    pub(in crate::editor) fn hidden(&self) -> HiddenLines {
        self.hidden.clone()
    }

    /// The width a `DisplayLineMap` of this pane wraps against.
    pub(in crate::editor) fn content_width(&self) -> hume_engine::pipeline::ContentWidth {
        self.content_width
    }

    /// The pane's scroll-margin geometry, `None` for a pane with no rows.
    /// `source` is the syntax of the buffer this key was built for.
    pub(in crate::editor) fn geometry<'a>(
        &self,
        source: Option<&'a dyn SyntaxSource>,
    ) -> Option<ViewGeometry<'a>> {
        self.size.geometry(
            self.scroll_margin,
            ScopeHeaderInputs {
                source,
                settings: self.context,
            },
        )
    }
}

impl EditorState {
    /// The screen's layout as the models have it now: the tab bar's
    /// visibility, the open drawer's items, a docked popup's wrapped text and
    /// the seam setting. Every size and rect read outside the frame's own
    /// pass comes from here, so it reflects a band opened or closed since the
    /// last frame. Never runs Steel, so it is safe inside a Steel call.
    pub(in crate::editor) fn screen_layout(&self, view: &EngineView) -> ScreenLayout {
        self.popup_screen_layout(view).0
    }

    /// [`Self::screen_layout`], with where the open popup renders on it,
    /// `None` with no popup open. A `'fit` popup is measured against the
    /// layout without its band, the pane its cursor box would sit in, and
    /// docks when its wrapped text overflows that box. A frame takes both
    /// once, so its band and its cursor box cannot disagree.
    #[allow(clippy::disallowed_methods)]
    pub(in crate::editor) fn popup_screen_layout(
        &self,
        view: &EngineView,
    ) -> (ScreenLayout, Option<ResolvedPopupLayout>) {
        let area = view.terminal_area();
        let drawer = self.input.drawer().map_or(0, |d| {
            hume_ui::drawer::band_height(d.item_count(&self.buffers), area.height)
        });
        let tabbar = u16::from(self.tabline_visible());
        let layout_with = |popup: u16| {
            let rows = ChromeRows {
                tabbar,
                bands: hume_ui::Bands { drawer, popup }.in_stack_order(),
            };
            view.screen_layout(rows, self.settings.pane_dividers)
        };
        let base = layout_with(0);
        let Some(popup) = self.input.popup() else {
            return (base, None);
        };
        let resolved = match popup.layout {
            hume_core::PopupLayout::Cursor => ResolvedPopupLayout::Cursor,
            hume_core::PopupLayout::Docked => ResolvedPopupLayout::Docked,
            hume_core::PopupLayout::Fit => {
                let bounds = FocusedBox::new(self, view, &base);
                if hume_ui::popup::fits_cursor_box(
                    popup.content(),
                    bounds.pane_rect.height,
                    bounds.content_width(),
                ) {
                    ResolvedPopupLayout::Cursor
                } else {
                    ResolvedPopupLayout::Docked
                }
            }
        };
        let screen = match resolved {
            ResolvedPopupLayout::Cursor => base,
            ResolvedPopupLayout::Docked => {
                let lines = popup.docked_total_rows(area.width);
                layout_with(hume_ui::popup::band_height(lines, area.height))
            }
        };
        (screen, Some(resolved))
    }

    /// Whether the tab bar takes a row of the screen.
    pub(in crate::editor) fn tabline_visible(&self) -> bool {
        match self.settings.tabline {
            crate::editor::settings::TablineVisibility::Always => true,
            crate::editor::settings::TablineVisibility::Never => false,
            crate::editor::settings::TablineVisibility::Dynamic => self.tabs.len() > 1,
        }
    }

    /// The size of pane `pid`, in the active tab or a background one, as
    /// [`Self::screen_layout`] lays it out.
    pub(in crate::editor) fn pane_size(&self, view: &EngineView, pid: PaneId) -> PaneSize {
        let screen = self.screen_layout(view);
        self.tabs
            .layouts(view.focused_layout())
            .find_map(|(_, layout)| screen.pane_size(layout, pid))
            .expect("every pooled pane is a leaf of one tab's layout")
    }

    /// The gutter settings pane `pid` draws by: its buffer's line-number
    /// style, its sign column's width from the buffer's `gutter-signs`
    /// setting, its registered sign sources and, for `auto`, whether the
    /// last sign sync placed a sign in the pane, and whether its fold column
    /// shows. Every gutter width, the frame's and a between-frame reader's
    /// alike, comes from here.
    pub(in crate::editor) fn gutter_inputs(&self, view: &EngineView, pid: PaneId) -> GutterInputs {
        let bid = view.panes[pid].buffer_id();
        let doc = self.buffers.get(bid);
        let overrides = &doc.overrides;
        let foldable = doc
            .syntax
            .as_ref()
            .is_some_and(|syntax| syntax.bundle().folds.is_some())
            || !self.hidden_lines(view, pid).is_empty();
        let collapsed = overrides.gutter_signs(&self.settings).mode
            == crate::editor::settings::SignColumnMode::Auto
            && !self.panes.entries[pid].render.has_signs();
        GutterInputs {
            line_numbers: overrides.gutter_line_numbers(&self.settings),
            sign_width: if collapsed {
                0
            } else {
                SignColumn::width_for_slots(self.sign_slots(bid))
            },
            folds: overrides.gutter_folds(&self.settings) && foldable,
        }
    }

    /// The sign slots buffer `bid` shows: its `gutter-signs` setting resolved
    /// against its registered sign sources. A source's slot is its rank in
    /// the buffer's registry (`DecorationStores::sign_sources`), fixed the
    /// moment it registers, independent of what any frame places.
    pub(in crate::editor) fn sign_slots(&self, bid: BufferId) -> u8 {
        self.buffers
            .get(bid)
            .overrides
            .gutter_signs(&self.settings)
            .slots_for(self.config.decorations.sign_source_count(bid))
    }

    /// Pane `pid`'s gutter and content widths at `size`, by the same
    /// computation the render pass draws it with.
    pub(in crate::editor) fn pane_geometry(
        &self,
        view: &EngineView,
        pid: PaneId,
        size: PaneSize,
    ) -> PaneGeometry {
        let pane = &view.panes[pid];
        hume_engine::layout::compute_viewport(
            self.buffers.get(pane.buffer_id()).text().rope(),
            size,
            pane.providers.gutter_columns(),
            self.gutter_inputs(view, pid),
        )
    }

    /// The composition every `FormatKey` in this crate is built from: every
    /// input `pane`'s line formats depend on besides its content width, which
    /// reaches formatting only through the wrap mode's resolved width (see
    /// `hume_engine::display_lines::line_store::ResolvedFormatKey`'s doc).
    ///
    /// The frame's scroll pass (`frame.rs`'s `scroll_into_view`) and its render
    /// pass (`frame.rs`'s `resolve_pane_settings`) each call this on `pane`'s
    /// current state, and their sharing that pane's line store depends
    /// entirely on the two resolving a bit-identical key. One function
    /// rather than two independently-maintained call sites is what makes
    /// that true.
    pub(in crate::editor) fn format_key(
        &self,
        pane: &hume_engine::pane::Pane,
    ) -> hume_engine::display_lines::line_store::FormatKey {
        let doc = self.buffers.get(pane.buffer_id());
        hume_engine::display_lines::line_store::FormatKey {
            buffer_tag: [
                // Not `{:?}`-formatted: this is a value to compare, not one
                // to show, and `as_ffi` folds the key's index and version
                // (the two things that distinguish a reused slot from the
                // buffer that held it before) into exactly that.
                slotmap::Key::data(&pane.buffer_id()).as_ffi(),
                doc.text().generation(),
                self.config.decorations.generation(pane.buffer_id()),
            ],
            wrap_mode: commands::effective_wrap_mode(doc, &self.settings, pane),
            tab_width: doc.overrides.indent_width(&self.settings),
            whitespace: doc.overrides.whitespace(&self.settings),
        }
    }

    /// The lines pane `pid`'s closed folds hide in the buffer it shows.
    pub(in crate::editor) fn hidden_lines(
        &self,
        view: &hume_engine::pipeline::EngineView,
        pid: hume_core::PaneId,
    ) -> HiddenLines {
        self.panes.entries.shown(&view.panes, pid).hidden_lines()
    }

    /// Pane `pid`'s [`Self::format_key`] plus the content width and geometry
    /// it formats and scrolls by at `size`, and the scroll margin. The frame's
    /// scroll pass reveals the cursor when this changes.
    pub(in crate::editor) fn layout_key(
        &self,
        view: &hume_engine::pipeline::EngineView,
        pid: hume_core::PaneId,
        size: hume_engine::pipeline::PaneSize,
    ) -> LayoutKey {
        let doc = self.buffers.get(view.panes[pid].buffer_id());
        let context = ContextSettings {
            enabled: doc.overrides.context_enabled(&self.settings),
            outer_level: doc.overrides.context_outer_level(&self.settings),
        };
        LayoutKey {
            format: self.format_key(&view.panes[pid]),
            hidden: self.hidden_lines(view, pid),
            content_width: self.pane_geometry(view, pid, size).content_width,
            size,
            scroll_margin: self.settings.scroll_margin,
            context,
            layers: doc
                .syntax
                .as_ref()
                .filter(|_| context.enabled)
                .and_then(Syntax::committed_gen),
        }
    }
}

/// The line range `viewport` shows with `hidden` folded away, end-exclusive,
/// clamped to a buffer of `content_lines`, so the hook payload and
/// `(viewport-range bid)` agree.
/// Never points past the buffer's last *content* line (not ropey's phantom
/// line past the structural trailing `\n`), even when the pane's viewport
/// height exceeds the buffer.
///
/// `height.max(1)` (not `height` directly): a `height == 0` pane (no visible
/// rows, e.g. one not yet laid out) still reports a one-line range rather
/// than an empty one, so callers always get at least the pane's top line
/// instead of a degenerate empty range.
pub(in crate::editor) fn pane_visible_range(
    viewport: &hume_engine::pane::Viewport,
    size: hume_engine::pipeline::PaneSize,
    hidden: &HiddenLines,
    content_lines: hume_rope::line::ContentLineCount,
) -> hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine> {
    let first_line = viewport.top().line;
    // Terminal-row count of shown lines past the top: under wrap one buffer
    // line can span multiple display lines (and therefore fewer terminal
    // rows than buffer lines), so this over-estimates how many buffer lines
    // are visible. Safe here: the range only needs to cover every buffer
    // line that *could* be visible, not name the true last one exactly.
    let height_rows = usize::from(size.height().max(1));
    let end_line = hidden
        .line_after_shown(first_line, height_rows)
        .min(content_lines.end_exclusive());
    hume_rope::offset::ExclusiveRange::new(first_line, end_line)
}

/// Pane `pid`'s visible line range (end-exclusive) between frames, for
/// `(viewport-range pane)` and the `OnViewportChange` payload, read from
/// the settled view (`frame::settled_pane_view`).
pub(in crate::editor) fn viewport_range(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
) -> hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine> {
    let size = state.pane_size(view, pid);
    let key = state.layout_key(view, pid, size);
    let content_lines = state
        .buffers
        .get(view.panes[pid].buffer_id())
        .text()
        .content_line_count();
    let (_, viewport) = super::frame::settled_pane_view(
        &state.buffers,
        &mut state.panes.entries,
        &mut view.panes,
        pid,
        &key,
    );
    pane_visible_range(viewport, size, &key.hidden(), content_lines)
}
