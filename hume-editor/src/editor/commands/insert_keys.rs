//! Insert mode's default per-key handling — what a key does when the
//! Insert keymap has no binding for it (or, via the `insert-key!` builtin,
//! when a bound command wants that same default behaviour anyway). A free
//! function over `(state, view, fp)` rather than an `impl Editor` method:
//! two of its callers, `handle_insert`'s (`input_stack/insert.rs`) own
//! unbound-key fallback and the `EditHost::insert_key` builtin, are on
//! opposite sides of the `Editor`/`EditorState` split — a
//! Steel builtin only ever holds `&mut EditorState` + `&mut EngineView` (see
//! `host_impl.rs`'s own doc), never a whole `&mut Editor`. `replay.rs`'s own
//! replay of an unbound key reuses the same function despite already
//! holding `&mut Editor`, rather than duplicating its match.

use hume_editing::changeset::ChangeSet;
use hume_editing::lines::leading_whitespace_end;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
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
    doc, effective_word_chars, pane_selections, tab_format,
};

/// Runs `key`'s Insert-mode default behaviour against `fp`'s (pane,
/// buffer). Never walks the Insert keymap: a keymap match is the caller's
/// job (`handle_insert`'s trie walk, or the fact that `insert-key!` exists
/// specifically to skip it).
///
/// Returns `true` if `key` has default Insert-mode behaviour at all (a
/// plain char, Tab, Enter, Backspace, or Delete) and that behaviour ran;
/// `false` for any other key (Esc, an arrow, an unhandled Ctrl-chord, …),
/// which have none to fall back to. `handle_insert` records a key for
/// dot-repeat only on `true`; `insert-key!` (`host_impl/edits.rs`) uses it
/// to fail fast instead of silently doing nothing for a key it names
/// explicitly.
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
            // buffer — the two skip-close branches below just move the
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
                        apply_insert_edit(state, view, fp, |b, s| {
                            insert_pair_close(b, s, open, close)
                        });
                    } else {
                        // Next char is a word char (or symmetric prev is word char):
                        // insert only the typed character.
                        apply_insert_edit(state, view, fp, |b, s| insert_char(b, s, ch));
                    }
                } else if ap_pairs.iter().any(|p| p.close == ch && !p.is_symmetric())
                    && should_skip_close(state, view, fp, ch)
                {
                    // Asymmetric close (e.g. `)`) when cursor is already on it.
                    skip_over_close(state, view, fp);
                    inserted = false;
                } else {
                    apply_insert_edit(state, view, fp, |b, s| insert_char(b, s, ch));
                }
            } else {
                apply_insert_edit(state, view, fp, |b, s| insert_char(b, s, ch));
            }
            if inserted {
                let buf = fp.bid(view);
                let lang_id = state.buffers.get(buf).language;
                // Owned, not a borrow of `state.config.languages`: `sources`
                // below already needs a borrowed `&str`, but `Trigger::Char`
                // is handed to a `&mut EditorState` method further down, so
                // its own copy can't be tied to that same borrow (see
                // `Trigger::Char`'s own doc).
                let language_owned =
                    lang_id.map(|id| state.config.languages.name_of(id).to_owned());
                let language = language_owned.as_deref();
                let sources = state.trigger_sources_for(ch, language);
                for source in &sources {
                    state.queue_event(EditorEvent::OnTriggerChar {
                        target: hume_scripting::PaneHandle::with_pane(buf, fp.pid()),
                        ch,
                        source: source.clone(),
                    });
                }
                // The hook above is for any listener (signature help); a
                // completion source's own trigger chars are looked up
                // directly against the registry
                // (`SourceRegistry::buffer_sources_for_trigger`) — no hook
                // round trip, and no dependency on
                // `register-trigger-chars!`'s separate table (`sources`
                // above is that table's own answer, used only to fire the
                // generic hook).
                state.trigger_buffer_completion(
                    view,
                    completion::Trigger::Char {
                        ch,
                        language: language_owned,
                    },
                );
            }
            true
        }

        // ── Tab ────────────────────────────────────────────────────────────
        // Governed by the `tab-style` setting: Hard inserts a literal `\t`,
        // Soft inserts spaces to the next tab stop (width from `tab-width`).
        KeyCode::Tab => {
            let (style, tw) = tab_format(doc(state, view, fp.pane()), &state.settings);
            apply_insert_edit(state, view, fp, move |b, s| insert_tab(b, s, style, tw));
            true
        }

        // ── Newline ───────────────────────────────────────────────────────
        // Auto-indent: copy the current line's leading whitespace onto the
        // new line. No smart indent.
        //
        // `allowed` (vim autoindent parity): only vacate a blank line's
        // whitespace if it's owned by an earlier auto-indent this session
        // itself made — never on the first Enter that lands on a
        // pre-existing blank line, since nothing has armed a record for it
        // yet. `arm_autoindent` after the edit records the *new* line's own
        // copied indent, so the next Enter/Esc on it trims.
        KeyCode::Enter => {
            let allowed = autoindent_owned(fp.pane().state(&state.panes.state, view));
            apply_insert_edit(state, view, fp, move |b, s| {
                insert_newline_indent(b, s, &allowed)
            });
            arm_autoindent(state, view, fp);
            true
        }

        // ── Delete ────────────────────────────────────────────────────────
        // Backspace needs no special handling to preserve autoindent
        // ownership: deleting *inside* the owned range only shrinks the
        // line's current whitespace, which stays within the recorded
        // `allowed.end` (see `is_owned_blank_line`'s containment check) —
        // matching `:help autoindent`'s own carve-out naming `<BS>` as the
        // one key that doesn't cancel a pending auto-indent trim.
        KeyCode::Backspace => {
            let overrides = &doc(state, view, fp.pane()).overrides;
            let (ap_enabled, ap_pairs) = overrides.auto_pairs_ref(&state.settings);
            let tw = overrides.tab_width(&state.settings);
            if should_dedent_backspace(state, view, fp) {
                // Dedent: snap every cursor in leading whitespace back to
                // the previous tab stop. All-or-nothing — if any cursor
                // isn't in leading ws, the whole batch falls back.
                apply_insert_edit(state, view, fp, move |b, s| dedent_tab_backward(b, s, tw));
            } else if ap_enabled && is_between_pair(state, view, fp, ap_pairs) {
                apply_insert_edit(state, view, fp, delete_pair);
            } else {
                apply_insert_edit(state, view, fp, delete_char_backward);
            }
            true
        }
        KeyCode::Delete => {
            apply_insert_edit(state, view, fp, delete_char_forward);
            true
        }

        _ => false,
    }
}

// ── Edit application ─────────────────────────────────────────────────────────

/// Applies a grouped edit on `fp`'s (pane, buffer) and, if a completion
/// session is open on that same buffer, tells it
/// (`EditorState::completion_observe_edit`: remap every token, re-rank,
/// re-invoke incomplete sources, dismiss if typed out of) — the chokepoint
/// every keystroke handler above that edits the buffer directly goes
/// through, so no such call site needs its own record-or-not decision.
fn apply_insert_edit(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
) {
    let buf = fp.bid(view);
    let cs = apply_focused_edit_grouped(state, view, fp, cmd);
    // Read after `apply_focused_edit_grouped` returns, so `text_gen` reflects
    // the edit just applied, not the buffer's state before it.
    let text_gen = state.buffers.get(buf).text_gen;
    state.completion_observe_edit(view, buf, &cs, text_gen);
}

/// Moves the cursor right past an existing closer instead of inserting a
/// duplicate — both auto-pair skip-close branches above (`"` typed while
/// sitting on a `"`, `)` typed while sitting on a `)`). A motion, not an
/// edit — bypasses `apply_insert_edit`, so `completion_observe_edit` never
/// runs and a live session's token would go untracked. Dismiss rather than
/// reintroduce a keystroke-driven refilter for a motion path that carries
/// no `ChangeSet` to remap.
fn skip_over_close(state: &mut EditorState, view: &EngineView, fp: FocusedPane) {
    apply_pane_motion(state, view, fp.pane(), |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    state.dismiss_completion(view);
}

// ── Auto-pair helpers ─────────────────────────────────────────────────────────

/// Returns `true` if every selection is a collapsed cursor sitting in a
/// line's leading whitespace (spaces/tabs), with at least one whitespace
/// char before it. All-or-nothing: if any selection doesn't qualify, the
/// whole batch falls back to plain Backspace so multi-cursor behaviour
/// stays consistent.
///
/// "In leading whitespace" means every char in `[line_start, head)` is a
/// space or tab — so a cursor on the first content char (right after the
/// indent) also qualifies, matching the dedent-to-prev-tab-stop behaviour of
/// modern editors. The boundary itself comes from the shared
/// [`leading_whitespace_end`] primitive.
fn should_dedent_backspace(state: &EditorState, view: &EngineView, fp: FocusedPane) -> bool {
    let text = doc(state, view, fp.pane()).text();
    pane_selections(state, view, fp.pane())
        .iter_sorted()
        .all(|sel| {
            if !sel.is_collapsed() {
                return false;
            }
            let p = sel.head();
            let line_idx = text.char_to_line(p);
            let line_start = text.line_to_char(line_idx.into());
            // `p > line_start` rules out char_col 0 (nothing to dedent). `p <=
            // leading_whitespace_end` keeps the all-or-nothing "in leading ws"
            // rule: at exactly the end the cursor sits on the first content
            // char and still qualifies.
            p > line_start && p <= leading_whitespace_end(text, line_idx)
        })
}

/// Returns `true` if every selection is a cursor AND the character at each
/// cursor's `head` equals `ch`.
///
/// All-or-nothing: if even one cursor doesn't match, the whole operation
/// falls back to normal insert, keeping multi-cursor behavior consistent.
fn should_skip_close(state: &EditorState, view: &EngineView, fp: FocusedPane, ch: char) -> bool {
    let text = doc(state, view, fp.pane()).text();
    pane_selections(state, view, fp.pane())
        .iter_sorted()
        .all(|sel| sel.is_collapsed() && text.char_at(sel.head()) == Some(ch))
}

/// Returns `true` if every selection is a cursor AND the pair
/// `(char_before_cursor, char_at_cursor)` matches a configured pair.
///
/// Used by Backspace to decide whether to delete both brackets or just one.
fn is_between_pair(
    state: &EditorState,
    view: &EngineView,
    fp: FocusedPane,
    pairs: &[Pair],
) -> bool {
    let text = doc(state, view, fp.pane()).text();
    pane_selections(state, view, fp.pane())
        .iter_sorted()
        .all(|sel| {
            if !sel.is_collapsed() || sel.head() == hume_rope::offset::CharOffset::new(0) {
                return false;
            }
            // prev_grapheme_boundary handles multi-codepoint clusters; bracket/quote
            // chars are always single codepoints, but using it keeps the logic uniform.
            let prev = hume_editing::grapheme::prev_grapheme_boundary(text, sel.head());
            match (text.char_at(prev), text.char_at(sel.head())) {
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
    pane_selections(state, view, fp.pane())
        .iter_sorted()
        .all(|sel| {
            !sel.is_collapsed() || should_auto_pair_at(text, sel.head(), pair, ap_pairs, chars)
        })
}
