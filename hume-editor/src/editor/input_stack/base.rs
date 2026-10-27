//! The `Base` layer (the always-present layer at index 0, presenting
//! Normal/Extend mode), plus [`handle_normal`], the Normal-mode key handler
//! `base_input` dispatches into.

use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_core::{BindMode, BufferId};
use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::MotionMode;
use hume_scripting::host::MAX_COUNT;

use super::super::commands::cmd_clear_search;
use super::super::error::CommandError;
use super::super::keymap::WalkResult;
use super::super::{Editor, EditorState};
use super::char::{CharRead, MacroRead, read_char};
use super::paste::PastedText;
use super::popup::PopupLayer;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler};

/// The always-present layer at index 0. Never removed. Carries `extend`
/// (Extend is a flag on `Base`, never its own layer) and `sticky_popup`, the
/// home for a `Sticky` popup shared with `InsertLayer`, see
/// [`Layer::sticky_popup_slot`]'s doc.
pub(in crate::editor) struct BaseLayer {
    pub(in crate::editor) extend: bool,
    pub(in crate::editor) sticky_popup: Option<PopupLayer>,
}

/// The keys of the current multi-key Normal/Extend sequence (max depth 3),
/// whether it began with a kitty one-shot Ctrl key that resolved to a
/// prefix node, and the buffer its first key was pressed in. The three are
/// one state: clearing the keys drops the extend intent and the buffer too,
/// so an abandoned sequence never extends the next one. At Leaf resolution
/// the intent applies only if the command is extendable.
#[derive(Default)]
pub(in crate::editor) struct PendingSequence {
    keys: Vec<KeyEvent>,
    ctrl_extend: bool,
    /// `Some` while `keys` is non-empty, `None` otherwise.
    buffer: Option<BufferId>,
}

impl PendingSequence {
    pub(in crate::editor) fn keys(&self) -> &[KeyEvent] {
        &self.keys
    }

    /// Add `key`, pressed in `buffer`, which must be where the sequence
    /// started.
    pub(in crate::editor) fn push(&mut self, key: KeyEvent, buffer: BufferId) {
        debug_assert!(self.buffer.is_none_or(|started| started == buffer));
        self.buffer = Some(buffer);
        self.keys.push(key);
    }

    /// Whether the sequence started in a buffer other than `buffer`. Its
    /// keys were resolved against that buffer's keymaps, so it can't go on
    /// here.
    pub(in crate::editor) fn started_outside(&self, buffer: BufferId) -> bool {
        self.buffer.is_some_and(|started| started != buffer)
    }

    pub(in crate::editor) fn ctrl_extend(&self) -> bool {
        self.ctrl_extend
    }

    fn mark_ctrl_extend(&mut self) {
        self.ctrl_extend = true;
    }

    pub(in crate::editor) fn clear(&mut self) {
        self.keys.clear();
        self.ctrl_extend = false;
        self.buffer = None;
    }
}

impl Layer for BaseLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        base_input
    }
    fn paste_handler(&self) -> PasteHandler {
        paste_over_selections
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(if self.extend {
            EditorMode::Extend
        } else {
            EditorMode::Normal
        })
    }
    fn sticky_popup_slot(&self) -> Option<&Option<PopupLayer>> {
        Some(&self.sticky_popup)
    }
    fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        Some(&mut self.sticky_popup)
    }
}

/// The base layer's own policy. `r` is unused: `Base` never falls
/// through further (there is nothing below it) and never truncates
/// itself (it is never removed). Only ever runs for Normal/Extend:
/// every other mode is its own layer type, routed directly by
/// `dispatch_at`; [`handle_normal`] still reads `state.mode()` internally
/// to tell the two apart.
pub(in crate::editor) fn base_input(ed: &mut Editor, _r: LayerRef, ev: InputEvent) {
    match ev {
        InputEvent::Key(key) => handle_normal(ed, key),
        InputEvent::Mouse(mouse) => ed.base_mouse(mouse),
    }
}

/// Bulk-insert `text`'s values at the selections in the focused buffer as
/// one edit: the `Base` layer's paste policy (Normal and Extend alike).
fn paste_over_selections(
    state: &mut EditorState,
    view: &EngineView,
    _r: LayerRef,
    text: PastedText,
) -> Result<(), CommandError> {
    let focused = view.focused_pane();
    let buf = view.panes[focused].buffer_id();
    let (buffers, mut stores) = state.edit_parts();
    super::super::doc_ops::apply_doc_edit(buffers, &mut stores, focused, buf, |s| {
        hume_ops::edit::insert_values(s, text.values())
    })
}

// ── Normal mode ───────────────────────────────────────────────────────────

fn handle_normal(ed: &mut Editor, key: KeyEvent) {
    // ── Kitty SHIFT normalization ─────────────────────────────────────────
    // The kitty keyboard protocol reports shifted keys as Char('Q') / Char(':')
    // / Char('$') with Modifiers::SHIFT. DISAMBIGUATE_ESCAPE_CODES keeps the
    // physically-held SHIFT in the modifier field; REPORT_ALTERNATE_KEYS is what
    // makes compliant terminals strip it again. WezTerm and other terminals
    // enable DISAMBIGUATE but do not fully honor REPORT_ALTERNATE_KEYS, so
    // shifted punctuation (`:`, `$`, `?`, `{`, …) arrives as Char(x) + SHIFT and
    // misses its Char(x) + NONE trie binding, silently swallowing `:`, `$`,
    // `?` etc. in Normal/Extend mode.
    //
    // For HUME's purposes the shifted-ness is already encoded in the char
    // itself (every printable binding in the keymap is stored as Char(x) +
    // NONE; no binding distinguishes via SHIFT), so the SHIFT bit is redundant
    // for any Char and is stripped here. This covers both kitty's letter
    // reporting and the shifted-punctuation gap on partially-compliant terminals.
    //
    // Only strip SHIFT when it is the *only* modifier. Ctrl-Shift combinations
    // (e.g. Ctrl-X, Ctrl-}) keep CONTROL so they match their explicit Ctrl
    // bindings; Shift-Tab arrives as KeyCode::BackTab (not Char), so it is
    // untouched and keeps its SHIFT bit for the completion back-cycle.
    let key = if key.modifiers == Modifiers::SHIFT {
        if let KeyCode::Char(_) = key.code {
            KeyEvent::new(key.code, Modifiers::NONE)
        } else {
            key
        }
    } else {
        key
    };

    let buffer = ed.focused_buffer_id();
    if ed.state.pending.started_outside(buffer) {
        ed.state.pending.clear();
        ed.state.count = None;
    }

    // ── Hard-reset on Esc ─────────────────────────────────────────────────
    if key.code == KeyCode::Escape {
        ed.state.pending.clear();
        ed.state.count = None;
        ed.state.register_prefix = None; // drop a "<reg> prefix not yet used
        // Esc exits Extend mode; Normal is the reset state.
        ed.state.input.set_extend(false);
        let t = crate::editor::commands::FocusedPane::current(&ed.view).pane();
        cmd_clear_search(
            &mut ed.state,
            &mut ed.view,
            t,
            crate::editor::registry::Count::new(None),
            MotionMode::Move,
        )
        .expect("clear-search is infallible");
        return;
    }

    // ── Count prefix accumulation ─────────────────────────────────────────
    // Only accumulate when we're at the trie root (no pending sequence)
    // and no modifiers are held (Ctrl-4 is not a count digit).
    // `0` without an existing count is not a digit: it falls through to the
    // trie (unbound by default; core:vim-keybind binds it to goto-line-start).
    // `Q1`/`q1` read `1` as a register name: the char layer above this
    // handler takes the key before it gets here.
    // Clamped to MAX_COUNT; see its doc comment for why.
    if ed.state.pending.keys().is_empty() && key.modifiers == Modifiers::NONE {
        match key.code {
            KeyCode::Char(d @ '0'..='9') if d != '0' || ed.state.count.is_some() => {
                let digit = d as usize - '0' as usize;
                let n = ed.state.count.unwrap_or(0) * 10 + digit;
                ed.state.count = Some(n.min(MAX_COUNT));
                return;
            }
            _ => {}
        }
    }

    // ── `Q` / `q` / `"` intercepts (bare key, at trie root, no modifiers) ──
    // `Q` toggles recording; `q` triggers replay. Recording uses uppercase
    // because you do it once; replay uses lowercase because you do it often.
    // Both are ignored while a replay is in progress, and `q` while recording,
    // so a macro cannot nest.
    // `"` triggers the register-prefix: the next char names the target register.
    if ed.state.pending.keys().is_empty() && key.modifiers == Modifiers::NONE {
        match key.code {
            KeyCode::Char('Q') => {
                if let Some((reg, keys)) = ed.state.macros.stop_recording() {
                    ed.state.registers.write_macro(reg, keys);
                } else if !ed.state.macros.is_replaying() {
                    // A count before `Q<reg>` has no meaning for recording,
                    // and would otherwise reach the first recorded key.
                    ed.state.count = None;
                    read_char(
                        &mut ed.state,
                        &ed.view,
                        CharRead::macro_register(MacroRead::Record),
                    );
                }
                return;
            }
            KeyCode::Char('q') => {
                if !ed.state.macros.is_replaying() && ed.state.macros.recording_register().is_none()
                {
                    let count = ed.state.count.take().unwrap_or(1);
                    read_char(
                        &mut ed.state,
                        &ed.view,
                        CharRead::macro_register(MacroRead::Replay { count }),
                    );
                }
                return;
            }
            KeyCode::Char('"') => {
                // Register prefix: `"<reg>` selects the named register for the
                // upcoming yank/delete/change/paste. A count typed before it
                // is kept for that command.
                read_char(&mut ed.state, &ed.view, CharRead::register());
                return;
            }
            _ => {}
        }
    }

    // ── Stage 1: Ctrl-key normalisation + one-shot extend ────────────────
    //
    // Two categories of CONTROL keys:
    // 1. Explicit Ctrl bindings (Ctrl-c, Ctrl-r, Ctrl-,, Ctrl-x, Ctrl-X):
    //    have a dedicated trie entry, used as-is regardless of kitty mode.
    // 2. Implicit Ctrl-motion (Ctrl-h/j/k/l/w/b, …): no explicit binding.
    //    With kitty enabled, one-shot extend: strip CONTROL, look up the
    //    bare key, dispatch extend=true. Without kitty, no-op: legacy
    //    terminals can't reliably distinguish Ctrl-letter from control
    //    codes.
    //
    // Detection: try the key as-is first; on NoMatch with CONTROL set,
    // strip it and retry only if kitty is enabled.
    //
    // REPORT_ALTERNATE_KEYS (enabled at init) makes the terminal send the
    // shifted character directly. The decoder swaps in the alternate
    // keycode and strips SHIFT, so Ctrl-} arrives as Char('}') with just
    // CONTROL, and stripping CONTROL gives the correct bare key,
    // independent of layout.

    // Trie walk + Ctrl normalisation in one pass.
    //
    // For Ctrl keys at the trie root, walk once to check for an explicit
    // binding. If found, reuse that result directly (no second walk).
    // If NoMatch, strip CONTROL and re-walk only on kitty terminals.
    let (resolved, ctrl_extend) =
        if key.modifiers.contains(Modifiers::CONTROL) && ed.state.pending.keys().is_empty() {
            let explicit = ed.resolve_keys(BindMode::Normal, &[key]);
            match explicit.walk {
                WalkResult::NoMatch if ed.kitty_enabled => {
                    // Kitty mode: strip CONTROL, re-walk as extend. Only proceed if the
                    // resolved command is extendable. This prevents e.g. Ctrl-u running
                    // "undo" (not a motion) as a one-shot extend.
                    let bare = KeyEvent::new(key.code, Modifiers::NONE);
                    ed.state.pending.push(bare, buffer);
                    let resolved = ed.resolve_keys(BindMode::Normal, ed.state.pending.keys());
                    let ctrl_extend = match &resolved.walk {
                        // Prefix key (g, m, z…): persist extend intent for the
                        // remaining keys in the sequence. Extendability is
                        // checked at Leaf resolution, not here.
                        WalkResult::Interior => {
                            ed.state.pending.mark_ctrl_extend();
                            true
                        }
                        WalkResult::Leaf(c) => ed
                            .state
                            .config
                            .registry
                            .get_mappable(c.name.as_ref())
                            .is_some_and(|r| r.is_extendable()),
                        WalkResult::NoMatch => false,
                    };
                    if !ctrl_extend {
                        ed.state.pending.clear();
                        ed.state.count = None;
                        return;
                    }
                    (resolved, true)
                }
                WalkResult::NoMatch => return, // Legacy: no-op.
                // Explicit Ctrl-letter binding. Pressing Ctrl is not a
                // request to extend here: only the binding's own
                // force_extend (e.g. Ctrl-x → select-line) extends it, and
                // Stage 2 applies that to every binding alike.
                // Interior: the Ctrl-key starts a multi-key sequence (e.g.
                // Ctrl-p → pane prefix); save it as pending so the
                // follow-up keypress can complete the trie walk.
                WalkResult::Leaf(_) => (explicit, false),
                WalkResult::Interior => {
                    ed.state.pending.push(key, buffer);
                    (explicit, false)
                }
            }
        } else {
            // A Ctrl key never takes an Extend-mode override.
            let mode = if ed.state.mode() == EditorMode::Extend
                && !key.modifiers.contains(Modifiers::CONTROL)
            {
                BindMode::Extend
            } else {
                BindMode::Normal
            };
            ed.state.pending.push(key, buffer);
            (
                ed.resolve_keys(mode, ed.state.pending.keys()),
                ed.state.pending.ctrl_extend(),
            )
        };

    // ── Stage 2: Final extend merge ───────────────────────────────────────
    //
    // Three inputs: sticky extend from editor mode, one-shot extend from the
    // Ctrl path (ctrl_extend), and the binding's own force_extend. Merge them
    // here. `extend` is passed as a parameter; no mode transition occurs. An
    // Extend-mode override is a command of its own, not an extend variant of
    // a normal one, so it never extends.
    match resolved.walk {
        WalkResult::Leaf(cmd) => {
            ed.state.pending.clear();
            // Only extend if the command supports it.
            let extend = !resolved.extend_override
                && (ed.state.mode() == EditorMode::Extend || ctrl_extend || cmd.force_extend)
                && ed
                    .state
                    .config
                    .registry
                    .get_mappable(cmd.name.as_ref())
                    .is_some_and(|r| r.is_extendable());
            let count = ed.state.count.take();
            ed.execute_keymap_command(cmd.name, count, extend);
        }
        WalkResult::Interior => {
            // More keys needed. The pending sequence stays populated.
        }
        WalkResult::NoMatch => {
            ed.state.pending.clear();
            ed.state.count = None;
        }
    }
}
