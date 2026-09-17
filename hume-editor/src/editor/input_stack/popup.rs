//! The `Scrollable` popup layer (hover, `gn`/`gp`'s diagnostic overlay). A
//! `Sticky` popup (signature help) has no handler here: it lives in a mode
//! layer's `sticky_popup` slot instead, never its own layer — see
//! [`super::stack::Layer::sticky_popup_slot`]'s doc.

use termina::event::{KeyCode, Modifiers};

use hume_engine::pipeline::{EngineView, RenderContext};
use hume_engine::theme::Theme;
use hume_engine::theme::ui_scopes;
use hume_engine::types::{EditorMode, Scope};

use super::super::{Editor, EditorState};
use super::placement::{focused_cursor_char, popup_placement};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

/// `(show-popup! text)`'s raw, unwrapped content — held on `EditorState`
/// until the next frame's `Editor::sync_popup_view`/`sync_popup_band_view`
/// (per `layout`) resolves it into a positioned
/// `hume_ui::popup::PopupState` or a `PopupBandState`. Never buried: every
/// layer's own `Layer::setup` retires it before landing (`PopupLayer`'s own,
/// for its self-replace contract; `MenuLayer`/`DrawerLayer`/`PickerLayer`,
/// which evict any open popup outright; `CompletionLayer`, which evicts only
/// this home via `InputStack::clear_popup_layer`, leaving a `Sticky` popup
/// in a mode layer's slot untouched) or is itself gated on the stack being
/// settled, so `close-popup!`/`popup()` never need to look past `top()`.
pub(in crate::editor) struct PopupLayer {
    pub(in crate::editor) text: String,
    /// Which of the two homes this popup used to get here (`Popup` layer vs.
    /// a mode layer's `sticky_popup` slot) already encodes `kind` for every
    /// production consumer — dismiss policy is entirely a function of
    /// storage location now, not this field. Kept for tests, which still
    /// assert on it directly rather than reaching for the storage location
    /// as a proxy.
    #[allow(dead_code)]
    pub(in crate::editor) kind: hume_scripting::host::PopupKind,
    /// First visible wrapped row, for a `Scrollable` popup. Clamped against
    /// `max_scroll` in [`scroll_popup`] before each delta is applied, since
    /// content height (and so `max_scroll`) can shrink between key presses
    /// (e.g. the terminal grows) without this field being touched.
    /// `Editor::sync_popup_view`/`sync_popup_band_view` additionally clamp a
    /// *copy* of this value for rendering each frame — that clamp is
    /// view-only and never writes back to the model.
    pub(in crate::editor) scroll: usize,
    /// `#:lang` — rebuilt fresh on every `show-popup!`, dropped with the
    /// popup on close. No separate invalidation path: the popup's lifetime
    /// IS the syntax's (SSOT).
    pub(in crate::editor) syntax: Option<super::super::popup_syntax::MarkupSyntax>,
    pub(in crate::editor) layout: hume_ui::popup::PopupLayout,
    /// Lazily built from `text`/`syntax` on first [`Self::content_mut`]
    /// call — not eagerly at `show-popup!` time, because a `#:lang` popup's
    /// styled runs need a *baked* theme (`Theme::resolve` debug-asserts on
    /// an unbaked `ScopeId`, and the theme isn't baked yet when `show_popup`
    /// runs — see `frame.rs`'s `prepare_frame` step ordering). Its own
    /// internal wrap cache is then keyed by width, so an unchanged width
    /// across frames is O(1), not a re-wrap. Reset to `None` by
    /// `theme::set_theme` — the one chokepoint every `view.theme` write goes
    /// through — since the baked styles are the one input here that *can*
    /// change out from under an open popup, unlike `text`/`syntax`.
    pub(in crate::editor) content: Option<hume_ui::popup::PopupContent>,
}

impl PopupLayer {
    /// [`Self::content`], building it on first call. `text`/`syntax` never
    /// change during a `PopupLayer`'s lifetime (a new `show-popup!` builds a
    /// fresh one), so building it once and caching it here is sound — the
    /// theme can still change under it, which is why [`Self::content`]'s own
    /// doc names the two sites that reset this back to `None` when it does.
    pub(in crate::editor) fn content_mut(
        &mut self,
        theme: &Theme,
    ) -> &mut hume_ui::popup::PopupContent {
        if self.content.is_none() {
            self.content = Some(match self.syntax.as_ref() {
                Some(syntax) => {
                    let base_style = theme.resolve_by_name(Scope(ui_scopes::POPUP));
                    let runs = syntax.styled_runs(&self.text, theme, base_style);
                    hume_ui::popup::PopupContent::styled(runs)
                }
                None => hume_ui::popup::PopupContent::plain(&self.text),
            });
        }
        self.content.as_mut().expect("populated above")
    }
}

impl Layer for PopupLayer {
    fn handler(&self) -> LayerHandler {
        popup_input
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// Gives `(show-popup! …)` its documented replace-not-stack contract:
    /// clears whichever of the two popup homes (a pushed `PopupLayer`, or
    /// the current mode layer's sticky slot) already holds one, regardless
    /// of which kind is landing now.
    fn setup(&mut self, state: &mut EditorState, _view: &EngineView) {
        state.input.clear_popups();
    }
    fn tear_down(&mut self, _state: &mut EditorState, _view: &EngineView) {}
    /// Non-modal: a popup owns nothing but Ctrl-u/d and dies on the very
    /// next key, so an async opener's staleness check
    /// (`InputStack::is_stack_settled`) must not treat one being open as
    /// "the stack moved" — same reasoning as `DrawerLayer::is_modal`.
    fn is_modal(&self) -> bool {
        false
    }
}

impl Editor {
    /// Write the current *cursor-anchored* popup content into the shared
    /// `PopupState` Arc so `PopupOverlay` can render it during this frame.
    /// Geometry (wrap width, flip/clamp position) is resolved fresh every
    /// frame against the focused pane's *current* rect — never pre-computed
    /// at `show-popup!` call time — so a resize or scroll never leaves it
    /// stale. A docked popup (`PopupLayout::Docked`) is handled by
    /// [`Self::sync_popup_band_view`] instead — this clears the popup view
    /// slot for that case, same as when no popup is open at all.
    ///
    /// Called from `prepare_frame` after `last_pane_area` is set (step 10):
    /// `EngineView::pane_rect` reads that field, so calling this any earlier
    /// would position against the previous frame's geometry.
    pub(in crate::editor) fn sync_popup_view(&mut self, ctx: &mut RenderContext) {
        let is_cursor = matches!(
            self.state.input.popup().map(|m| &m.layout),
            Some(hume_ui::popup::PopupLayout::Cursor)
        );
        if !is_cursor {
            if self.state.views.popup.read().is_some() {
                self.state.views.popup.set(None);
            }
            return;
        }

        let anchor_char = focused_cursor_char(self);
        let Some(placement) = popup_placement(self, ctx, anchor_char) else {
            self.state.views.popup.set(None);
            return;
        };
        let border = self.state.settings.popup_border;
        let theme = &self.view.theme;
        let model = self
            .state
            .input
            .popup_mut()
            .expect("popup present: is_cursor checked above");
        // Read before the `&mut` below — a second `self.state.input.popup()`
        // borrow once `content` is live would conflict with it.
        let scroll = model.scroll;
        let content = model.content_mut(theme);
        let resolved = hume_ui::popup::resolve_popup(content, placement, scroll, border);

        self.state.views.popup.set(Some(resolved));
    }

    /// Write the current *docked* popup content into the shared
    /// `PopupBandState` Arc so `PopupBandWidget` can render it during this
    /// frame — the `PopupLayout::Docked` counterpart of
    /// [`Self::sync_popup_view`]. Unlike the cursor layout, geometry isn't
    /// resolved here: only content (wrapped lines + scroll clamp), mirroring
    /// the drawer's chrome contract — the engine resolves the band's actual
    /// position/height from `height(max)` at render time.
    ///
    /// Wraps against `last_terminal_area` (the raw, un-subtracted terminal
    /// area), not `last_pane_area` — the band spans the full width the
    /// engine will actually render into (`EngineView::render`'s bottom-band
    /// block), same convention `drawer_visible_rows` already relies on for
    /// its height ceiling.
    pub(in crate::editor) fn sync_popup_band_view(&mut self) {
        let is_docked = matches!(
            self.state.input.popup().map(|m| &m.layout),
            Some(hume_ui::popup::PopupLayout::Docked)
        );
        if !is_docked {
            if self.state.views.popup_band.read().is_some() {
                self.state.views.popup_band.set(None);
            }
            return;
        }

        let area = self.view.last_terminal_area;
        let max_rows = hume_engine::pipeline::EngineView::bottom_band_max(area.height);
        let border = self.state.settings.popup_border;
        let theme = &self.view.theme;
        let model = self
            .state
            .input
            .popup_mut()
            .expect("popup present: is_docked checked above");
        let scroll = model.scroll;
        let content = model.content_mut(theme);
        let resolved = hume_ui::popup::resolve_band(content, area.width, max_rows, scroll, border);

        self.state.views.popup_band.set(Some(resolved));
    }
}

/// Handles one event while a `Scrollable` popup is open (a `PopupLayer` — a
/// `Sticky` popup lives in a mode layer's slot instead and is never
/// dispatch's target). Ctrl-u/Ctrl-d consume the key only when
/// [`scroll_popup`] finds content past one screenful; every other key, a
/// paste, and any mouse gesture retire the layer and fall through this same
/// call, so a short popup never blocks the buffer's own half-page scroll
/// and a click still moves the cursor underneath it. Unlike
/// `Confirm`/`Menu`'s mouse arm, this doesn't gate on `is_fresh_gesture`:
/// nothing opens a popup mid-gesture, so there is no release to protect the
/// way a click-opened confirm needs its own `Up` protected.
pub(in crate::editor) fn popup_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        InputEvent::Paste(_) | InputEvent::Mouse(_) => {
            ed.state.truncate_layers(&ed.view, r);
            ed.fall_through(r, ev);
            return;
        }
    };
    if key.modifiers.contains(Modifiers::CONTROL)
        && let KeyCode::Char(c @ ('u' | 'd')) = key.code
        && scroll_popup(ed, c == 'd')
    {
        return;
    }
    ed.state.truncate_layers(&ed.view, r);
    ed.fall_through(r, InputEvent::Key(key));
}

/// Scrolls the open popup half its visible content height, in the
/// direction given by `down`. Reads the visible height and total row
/// count from the *already-resolved* view for the popup's current
/// layout — this same frame's paint — rather than recomputing geometry,
/// so the scroll step always matches what's on screen. Both views are
/// re-synced every frame regardless (`Editor::sync_popup_view`/
/// `sync_popup_band_view`), so no explicit sync is needed here. Before
/// the first frame both are empty — a documented no-op, same as
/// `picker_input`'s own geometry.
///
/// Returns `true` if the popup actually has content past one screenful
/// (`max_scroll > 0`) — [`popup_input`] uses this to tell a real scroll
/// from a popup too short to scroll, so Ctrl-d/Ctrl-u fall through to their
/// usual buffer effect instead of being silently eaten.
fn scroll_popup(ed: &mut Editor, down: bool) -> bool {
    let Some(layout) = ed.state.input.popup().map(|p| &p.layout) else {
        return false;
    };
    let (inner_h, total) = match layout {
        hume_ui::popup::PopupLayout::Cursor => {
            let Some(pair) = ed
                .state
                .views
                .popup
                .read()
                .as_ref()
                .map(|s| (s.rect.height.saturating_sub(2) as usize, s.lines.len()))
            else {
                return false;
            };
            pair
        }
        hume_ui::popup::PopupLayout::Docked => {
            let Some(total) = ed
                .state
                .views
                .popup_band
                .read()
                .as_ref()
                .map(|s| s.lines.len())
            else {
                return false;
            };
            let max = hume_engine::pipeline::EngineView::bottom_band_max(
                ed.view.last_terminal_area.height,
            );
            (hume_ui::popup::band_visible_rows(total, max), total)
        }
    };
    let max_scroll = total.saturating_sub(inner_h);
    if max_scroll == 0 {
        return false;
    }
    let Some(popup) = ed.state.input.popup_mut() else {
        return false;
    };
    let half = (inner_h / 2).max(1);
    // `popup.scroll` is the model value, re-clamped for rendering only in
    // the per-frame view sync (see `PopupLayer::scroll`) — it can be
    // stale-large after the popup's content shrinks (e.g. terminal grows
    // between frames without a key event dismissing it), so clamp before
    // applying the delta rather than after.
    let clamped = popup.scroll.min(max_scroll);
    popup.scroll = if down {
        (clamped + half).min(max_scroll)
    } else {
        clamped.saturating_sub(half)
    };
    true
}
