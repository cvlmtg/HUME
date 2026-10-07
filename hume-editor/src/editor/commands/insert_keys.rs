//! Insert mode's default per-key handling: what a key does when the
//! Insert keymap has no binding for it (or, via the `insert-key!` builtin,
//! when a bound command wants that same default behaviour anyway). A free
//! function over `(state, view, fp)` rather than an `impl Editor` method:
//! two of its callers, `handle_insert`'s own unbound-key fallback and the
//! `EditHost::insert_key` builtin, are on opposite sides of the
//! `Editor`/`EditorState` split: a Steel builtin only ever holds
//! `&mut EditorState` + `&mut EngineView` (see `host_impl.rs`'s own doc),
//! never a whole `&mut Editor`.

use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::auto_pairs::{Pair, delete_pair, insert_pair_close, should_auto_pair_at};
use hume_ops::edit::{
    dedent_tab_backward, delete_char_backward, delete_char_forward, insert_char,
    insert_newline_indent, insert_tab,
};
use hume_ops::motion::cmd_move_right;

use crate::editor::EditorState;
use crate::editor::completion;
use crate::editor::event::EditorEvent;

use super::{
    FocusedPane, apply_focused_edit_grouped, apply_pane_motion, arm_autoindent, autoindent_owned,
    doc, effective_word_chars, pane_view, tab_format,
};

/// Runs `key`'s Insert-mode default behaviour against `fp`'s (pane,
/// buffer). Never walks the Insert keymap: a keymap match is the caller's
/// job (`handle_insert`'s trie walk, or the fact that `insert-key!` exists
/// specifically to skip it).
///
/// Returns `true` if `key` has default Insert-mode behaviour at all (a
/// plain char, Tab, Enter, Backspace, or Delete) and that behaviour ran;
/// `false` for any other key (Esc, an arrow, an unhandled Ctrl-chord, …),
/// which have none to fall back to.
pub(in crate::editor) fn insert_default_key(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    key: KeyEvent,
) -> bool {
    match key.code {
        KeyCode::Char(ch) if !key.modifiers.contains(Modifiers::CONTROL) => {
            let (ap_enabled, ap_pairs) = doc(state, view, fp.pane())
                .overrides
                .auto_pairs_ref(&state.settings);
            // `OnTriggerChar` only fires when `ch` actually landed in the
            // buffer. The two skip-close branches below just move the
            // cursor past an existing closer, inserting nothing.
            let mut inserted = true;
            if ap_enabled {
                if let Some(pair) = ap_pairs.iter().find(|p| p.open == ch) {
                    let (open, close, symmetric) = (pair.open, pair.close, pair.is_symmetric());
                    if symmetric && should_skip_close(state, view, fp, ch) {
                        // e.g. typing `"` when cursor already sits on `"`.
                        skip_over_close(state, view, fp);
                        inserted = false;
                    } else if should_auto_pair(state, view, fp, pair, ap_pairs) {
                        // Context is clear: insert open+close or wrap selection.
                        apply_focused_edit_grouped(state, view, fp, |s| {
                            insert_pair_close(s, open, close)
                        });
                    } else {
                        // Next char is a word char (or symmetric prev is word char):
                        // insert only the typed character.
                        apply_focused_edit_grouped(state, view, fp, |s| insert_char(s, ch));
                    }
                } else if ap_pairs.iter().any(|p| p.close == ch && !p.is_symmetric())
                    && should_skip_close(state, view, fp, ch)
                {
                    // Asymmetric close (e.g. `)`) when cursor is already on it.
                    skip_over_close(state, view, fp);
                    inserted = false;
                } else {
                    apply_focused_edit_grouped(state, view, fp, |s| insert_char(s, ch));
                }
            } else {
                apply_focused_edit_grouped(state, view, fp, |s| insert_char(s, ch));
            }
            if inserted {
                let buf = fp.bid(view);
                let triggered = state.triggered_by(ch, buf);
                for source in triggered.hooks {
                    state.queue_event(EditorEvent::OnTriggerChar {
                        target: hume_scripting::PaneHandle::with_pane(buf, fp.pid()),
                        ch,
                        source: source.to_string(),
                    });
                }
                // The hooks above are for any listener (signature help); a
                // completion source's own trigger chars invoke it directly,
                // with no hook round trip.
                state.trigger_buffer_completion(
                    view,
                    completion::Trigger::Sources(triggered.completions),
                );
            }
            true
        }

        // ── Tab ────────────────────────────────────────────────────────────
        // Governed by the `tab-style` setting: Hard inserts a literal `\t`,
        // Soft inserts spaces to the next tab stop (width from `tab-width`).
        KeyCode::Tab => {
            let (style, tw) = tab_format(doc(state, view, fp.pane()), &state.settings);
            apply_focused_edit_grouped(state, view, fp, move |s| insert_tab(s, style, tw));
            true
        }

        // ── Newline ───────────────────────────────────────────────────────
        // Auto-indent: copy the current line's leading whitespace onto the
        // new line. No smart indent.
        //
        // `allowed` (vim autoindent parity): only vacate a blank line's
        // whitespace if it's owned by an earlier auto-indent this session
        // itself made, never on the first Enter that lands on a
        // pre-existing blank line, since nothing has armed a record for it
        // yet. `arm_autoindent` after the edit records the *new* line's own
        // copied indent, so the next Enter/Esc on it trims.
        KeyCode::Enter => {
            let allowed = autoindent_owned(
                fp.pane().state(&state.panes.state, view),
                doc(state, view, fp.pane()).text(),
            );
            apply_focused_edit_grouped(state, view, fp, move |s| {
                insert_newline_indent(s, &allowed)
            });
            arm_autoindent(state, view, fp);
            true
        }

        // ── Delete ────────────────────────────────────────────────────────
        // Backspace needs no special handling to preserve autoindent
        // ownership: deleting *inside* the owned range only shrinks the
        // line's current whitespace, which stays within the recorded
        // `allowed.end` (see `owned_indent`'s containment check),
        // matching `:help autoindent`'s own carve-out naming `<BS>` as the
        // one key that doesn't cancel a pending auto-indent trim.
        KeyCode::Backspace => {
            let overrides = &doc(state, view, fp.pane()).overrides;
            let (ap_enabled, ap_pairs) = overrides.auto_pairs_ref(&state.settings);
            let tw = overrides.tab_width(&state.settings);
            if should_dedent_backspace(state, view, fp) {
                // Dedent: snap every cursor in leading whitespace back to
                // the previous tab stop. All-or-nothing: if any cursor
                // isn't in leading ws, the whole batch falls back.
                apply_focused_edit_grouped(state, view, fp, move |s| dedent_tab_backward(s, tw));
            } else if ap_enabled && is_between_pair(state, view, fp, ap_pairs) {
                apply_focused_edit_grouped(state, view, fp, delete_pair);
            } else {
                apply_focused_edit_grouped(state, view, fp, delete_char_backward);
            }
            true
        }
        KeyCode::Delete => {
            apply_focused_edit_grouped(state, view, fp, delete_char_forward);
            true
        }

        _ => false,
    }
}

// ── Skip-close ───────────────────────────────────────────────────────────────

/// Moves the cursor right past an existing closer instead of inserting a
/// duplicate: both auto-pair skip-close branches above (`"` typed while
/// sitting on a `"`, `)` typed while sitting on a `)`). A motion, not an
/// edit.
fn skip_over_close(state: &mut EditorState, view: &EngineView, fp: FocusedPane) {
    apply_pane_motion(state, view, fp.pane(), |st| {
        cmd_move_right(st, 1, MotionMode::Move)
    });
}

// ── Auto-pair helpers ─────────────────────────────────────────────────────────

/// Returns `true` if every selection is a collapsed cursor sitting in a
/// line's leading whitespace (spaces/tabs), with at least one whitespace
/// char before it. All-or-nothing: if any selection doesn't qualify, the
/// whole batch falls back to plain Backspace so multi-cursor behaviour
/// stays consistent.
///
/// "In leading whitespace" means every char in `[line_start, head)` is a
/// space or tab, so a cursor on the first content char (right after the
/// indent) also qualifies, matching the dedent-to-prev-tab-stop behaviour of
/// modern editors. The boundary itself comes from the shared
/// [`leading_whitespace_end`] primitive.
fn should_dedent_backspace(state: &EditorState, view: &EngineView, fp: FocusedPane) -> bool {
    let text = doc(state, view, fp.pane()).text();
    pane_view(state, view, fp.pane()).iter().all(|sel| {
        if !sel.is_cursor() {
            return false;
        }
        let p = sel.head();
        let line_idx = text.char_to_line(p.offset());
        let line_start = text.line_to_char(line_idx.into());
        // `p > line_start` rules out char_col 0 (nothing to dedent). `p <=
        // leading_whitespace_end` keeps the all-or-nothing "in leading ws"
        // rule: at the whitespace end the cursor sits on the first content
        // char and still qualifies.
        p.offset() > line_start && p <= text.lines().indent_end(line_idx)
    })
}

/// Returns `true` if every selection is a cursor AND the character at each
/// cursor's `head` equals `ch`.
///
/// All-or-nothing: if even one cursor doesn't match, the whole operation
/// falls back to normal insert, keeping multi-cursor behavior consistent.
fn should_skip_close(state: &EditorState, view: &EngineView, fp: FocusedPane, ch: char) -> bool {
    let text = doc(state, view, fp.pane()).text();
    pane_view(state, view, fp.pane())
        .iter()
        .all(|sel| sel.is_cursor() && text.char_at(sel.head().offset()) == Some(ch))
}

/// Returns `true` if every selection is a cursor AND the pair
/// `(char_before_cursor, char_at_cursor)` matches a configured pair.
fn is_between_pair(
    state: &EditorState,
    view: &EngineView,
    fp: FocusedPane,
    pairs: &[Pair],
) -> bool {
    let text = doc(state, view, fp.pane()).text();
    pane_view(state, view, fp.pane()).iter().all(|sel| {
        if !sel.is_cursor() {
            return false;
        }
        let Some(prev) = text.clusters().prev(sel.head().into()) else {
            return false;
        };
        match (
            text.char_at(prev.offset()),
            text.char_at(sel.head().offset()),
        ) {
            (Some(before), Some(at)) => pairs.iter().any(|p| p.open == before && p.close == at),
            _ => false,
        }
    })
}

/// Returns `true` if auto-pairing `pair` is appropriate given the current
/// selections. All-or-nothing: every collapsed selection must satisfy the
/// context rules; non-collapsed selections always pass (they wrap).
fn should_auto_pair(
    state: &EditorState,
    view: &EngineView,
    fp: FocusedPane,
    pair: &Pair,
    ap_pairs: &[Pair],
) -> bool {
    let buf = doc(state, view, fp.pane());
    let text = buf.text();
    let chars = effective_word_chars(buf, &state.settings);
    pane_view(state, view, fp.pane()).iter().all(|sel| {
        !sel.is_cursor() || should_auto_pair_at(text, sel.head().into(), pair, ap_pairs, chars)
    })
}
