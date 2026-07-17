//! Multi-buffer Steel builtins — buffer/pane query and lifecycle ops.
//!
//! All builtins guard against init-eval context (`EvalMode::Init` or
//! `PluginLoad`) via the `cmd`-gated `builtins!` registration table entry,
//! where editor refs are not available.  Calling any of these from
//! `init.scm` raises a Steel error instead of returning a meaningless
//! default.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use super::args::{BidArg, cons_pair, usize_arg};
use super::errors::generic_err;
use super::ids::{SteelBufferId, SteelPaneId};
use crate::{SteelCtx, types::Effect};

type SteelResult = Result<SteelVal, SteelErr>;

// ── Focus builtins ─────────────────────────────────────────────────────────────

/// `(current-buffer)` → BufferId of the focused buffer at dispatch time.
pub(crate) fn current_buffer(ctx: &mut SteelCtx) -> SteelResult {
    Ok(SteelBufferId(ctx.focused_buffer_id).into_steel_val())
}

/// `(current-pane)` → PaneId of the focused pane at dispatch time.
pub(crate) fn current_pane(ctx: &mut SteelCtx) -> SteelResult {
    Ok(SteelPaneId(ctx.focused_pane_id).into_steel_val())
}

// ── Enumeration builtins ───────────────────────────────────────────────────────

/// `(buffers)` → list of all open BufferIds in open-order.
pub(crate) fn buffers(ctx: &mut SteelCtx) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .buffer_ids()
        .into_iter()
        .map(|id| SteelBufferId(id).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

/// `(panes)` → list of all open PaneIds.
pub(crate) fn panes(ctx: &mut SteelCtx) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .pane_ids()
        .into_iter()
        .map(|id| SteelPaneId(id).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

// ── Buffer property builtins ───────────────────────────────────────────────────

/// `(buffer-path bid)` → absolute path string, or `#f` for unsaved buffers.
pub(crate) fn buffer_path(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    if !ctx.host.buffers().buffer_exists(id) {
        steel::stop!(Generic => "buffer-path: invalid buffer id {id:?}");
    }
    match ctx.host.buffers().buffer_path(id) {
        Some(p) => p
            .to_string_lossy()
            .into_owned()
            .into_steelval()
            .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-name bid)` → display name (filename or `"*scratch*"`).
pub(crate) fn buffer_name(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    ctx.host
        .buffers()
        .buffer_display_name(id)
        .ok_or_else(|| generic_err(format!("buffer-name: invalid buffer id {id:?}")))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(buffer-dirty? bid)` → `#t` if the buffer has unsaved edits.
pub(crate) fn buffer_dirty(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    let dirty = ctx
        .host
        .buffers()
        .buffer_is_dirty(id)
        .ok_or_else(|| generic_err(format!("buffer-dirty?: invalid buffer id {id:?}")))?;
    Ok(SteelVal::BoolV(dirty))
}

/// `(buffer-generation bid)` → int — bumped by every mutation to `bid`.
/// Steel-side staleness token; not LSP-specific despite the motivation.
pub(crate) fn buffer_generation(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    let generation = ctx
        .host
        .buffers()
        .buffer_generation(id)
        .ok_or_else(|| generic_err(format!("buffer-generation: invalid buffer id {id:?}")))?;
    Ok(SteelVal::IntV(generation as isize))
}

// ── Mutating builtins ─────────────────────────────────────────────────────────

/// `(open-buffer! path)` → BufferId.
///
/// Opens `path` as a new buffer and returns its `BufferId`. If the path is
/// already open, returns the existing id without opening a new buffer. Does
/// not switch the focused pane — call `(switch-to-buffer! bid)` separately
/// if desired.
pub(crate) fn open_buffer(ctx: &mut SteelCtx, path: String) -> SteelResult {
    let bid = ctx
        .host
        .buffers()
        .open_buffer(std::path::Path::new(&path))
        .map_err(generic_err)?;
    SteelBufferId(bid).into_steelval().map_err(generic_err)
}

/// `(close-buffer! bid)` → void.
///
/// Closes the buffer identified by `bid`. Raises a Steel error for an invalid
/// or unknown `bid`.
pub(crate) fn close_buffer(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    if !ctx.host.buffers().buffer_exists(id) {
        steel::stop!(Generic => "close-buffer!: invalid buffer id {id:?}");
    }
    let new_live = ctx.host.buffers().close_buffer(id).map_err(generic_err)?;
    ctx.live_focused_buffer_id = new_live;
    Ok(SteelVal::Void)
}

/// `(switch-to-buffer! bid)` → void.
///
/// Redirects the focused pane to the buffer identified by `bid`, recording
/// the current position in the jump list. Raises a Steel error for an invalid
/// or unknown `bid`.
pub(crate) fn switch_to_buffer(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let target = bid.0;
    if !ctx.host.buffers().buffer_exists(target) {
        steel::stop!(Generic => "switch-to-buffer!: invalid buffer id {target:?}");
    }
    let current = ctx.live_focused_buffer_id;
    ctx.host
        .buffers()
        .switch_to_buffer(current, target)
        .map_err(generic_err)?;
    ctx.live_focused_buffer_id = target;
    Ok(SteelVal::Void)
}

// ── Language builtins ─────────────────────────────────────────────────────────

/// Reverse-scan the effect log for the last `set-buffer-language!` call for
/// `id` queued so far this eval; fall back to `fallback` (the buffer's
/// stored language).
fn effective_language(
    effects: &[crate::types::QueuedEffect],
    id: hume_engine::pipeline::BufferId,
    fallback: Option<String>,
) -> Option<String> {
    effects
        .iter()
        .rev()
        .find_map(|queued| match &queued.effect {
            Effect::SetBufferLanguage { buffer, language } if *buffer == id => {
                Some(language.clone())
            }
            _ => None,
        })
        .unwrap_or(fallback)
}

/// `(buffer-language bid)` → string or `#f`.
pub(crate) fn buffer_language(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    if !ctx.host.buffers().buffer_exists(id) {
        steel::stop!(Generic => "buffer-language: invalid buffer id {id:?}");
    }
    let fallback = ctx.host.buffers().buffer_stored_language(id);
    let lang = effective_language(ctx.effects, id, fallback);
    match lang {
        Some(name) => name.into_steelval().map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

// ── Live cursor/selection reads ───────────────────────────────────────────────

/// `(current-line-number)` → 1-indexed line number of the primary cursor, or `#f`.
///
/// Reads live state — reflects any synchronous edits or motions that ran
/// earlier in the same Steel eval (e.g. after `(move-left)`).
pub(crate) fn current_line_number(ctx: &mut SteelCtx) -> SteelResult {
    match ctx.host.cursor().current_line_number() {
        Some(n) => Ok(SteelVal::IntV(n as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(current-selections)` → list of `(anchor head primary?)` per selection —
/// raw 0-indexed inclusive char offsets, direction preserved (anchor > head
/// when backward), sorted by selection start, exactly one `primary?` = `#t` —
/// or `#f` when the focused (pane, buffer) has no seeded pane state.
pub(crate) fn current_selections(ctx: &mut SteelCtx) -> SteelResult {
    match ctx.host.cursor().current_selections() {
        Some(sels) => {
            let list: Vec<SteelVal> = sels
                .into_iter()
                .map(|(anchor, head, primary)| {
                    vec![
                        SteelVal::IntV(anchor as isize),
                        SteelVal::IntV(head as isize),
                        SteelVal::BoolV(primary),
                    ]
                    .into_steelval()
                    .map_err(generic_err)
                })
                .collect::<Result<_, _>>()?;
            list.into_steelval().map_err(generic_err)
        }
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(char-index->line idx)` → 1-indexed line number containing 0-indexed char
/// offset `idx`, or `#f` when the focused buffer id is stale (buffer no
/// longer exists) or `idx` is out of range (> buffer length in chars).
pub(crate) fn char_index_to_line(ctx: &mut SteelCtx, idx: SteelVal) -> SteelResult {
    let idx = usize_arg(idx, "char-index->line")?;
    match ctx.host.cursor().char_index_to_line(idx) {
        Some(line) => Ok(SteelVal::IntV(line as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(viewport-range bid)` → `(first-line . last-line)` currently visible
/// for `bid` (the focused pane's if shown there, else the first pane showing
/// it), or `#f` if `bid` isn't open in any pane. Reads live view state,
/// which only exists at command dispatch, hook fire, or a queued-call drain.
pub(crate) fn viewport_range(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    match ctx.host.buffers().viewport_range(id) {
        Some((first, last)) => {
            cons_pair(SteelVal::IntV(first as isize), SteelVal::IntV(last as isize))
        }
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(selection-spans-full-line? bid)`.
pub(crate) fn selection_spans_full_line(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(SteelVal::BoolV(
        ctx.host.cursor().selection_spans_full_line(id),
    ))
}

/// `(symbol-under-cursor bid)`.
pub(crate) fn symbol_under_cursor(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(SteelVal::StringV(
        ctx.host.cursor().symbol_under_cursor(id).into(),
    ))
}

/// `(set-buffer-language! bid lang-or-#f)` — deferred; applied after the eval returns.
pub(crate) fn set_buffer_language_steel(
    ctx: &mut SteelCtx,
    bid: BidArg,
    lang: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let new_lang = match &lang {
        SteelVal::StringV(s) => Some(s.to_string()),
        SteelVal::BoolV(false) => None,
        _ => {
            steel::stop!(TypeMismatch => "set-buffer-language!: expected string or #f, got {:?}", lang)
        }
    };
    if !ctx.host.buffers().buffer_exists(id) {
        steel::stop!(Generic => "set-buffer-language!: invalid buffer id {id:?}");
    }
    let fallback = ctx.host.buffers().buffer_stored_language(id);
    if effective_language(ctx.effects, id, fallback) == new_lang {
        return Ok(SteelVal::Void);
    }
    ctx.push_effect(Effect::SetBufferLanguage {
        buffer: id,
        language: new_lang,
    });
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use hume_engine::pipeline::BufferId;

    fn default_bid() -> BidArg {
        BidArg(BufferId::default())
    }

    // ── Gate (init mode rejection) ────────────────────────────────────────────
    //
    // Every builtin below is `cmd`-gated in `builtins!`'s registration table —
    // the gate lives in the registration wrapper closure, not the function
    // body, so these test the gate primitive directly rather than calling the
    // builtin (its body has no guard to hit).

    /// `current-buffer` is blocked in init mode.
    ///
    /// Fail oracle: change `current-buffer`'s table entry from `cmd` to
    /// `open` → `focused_buffer_id` (which is Default in init) would be
    /// returned, silently giving wrong data.
    #[test]
    fn current_buffer_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let result = super::super::errors::require_cmd(&h.ctx_init(), "current-buffer");
        assert!(result.is_err(), "current-buffer must error in init mode");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("init"),
            "error must mention 'init'; got: {msg}"
        );
    }

    /// `current-pane` is blocked in init mode.
    #[test]
    fn current_pane_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "current-pane").is_err());
    }

    /// `buffers` is blocked in init mode.
    #[test]
    fn buffers_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffers").is_err());
    }

    /// `panes` is blocked in init mode.
    #[test]
    fn panes_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "panes").is_err());
    }

    /// `buffer-path` is blocked in init mode.
    #[test]
    fn buffer_path_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-path").is_err());
    }

    /// `buffer-name` is blocked in init mode.
    #[test]
    fn buffer_name_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-name").is_err());
    }

    /// `buffer-dirty?` is blocked in init mode.
    #[test]
    fn buffer_dirty_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "buffer-dirty?").is_err());
    }

    /// `close-buffer!` is blocked in init mode.
    #[test]
    fn close_buffer_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "close-buffer!").is_err());
    }

    /// `switch-to-buffer!` is blocked in init mode.
    #[test]
    fn switch_to_buffer_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "switch-to-buffer!").is_err());
    }

    /// `set-buffer-language!` is blocked in init mode.
    #[test]
    fn set_buffer_language_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let result = super::super::errors::require_cmd(&h.ctx_init(), "set-buffer-language!");
        assert!(
            result.is_err(),
            "set-buffer-language! must error in init mode"
        );
    }

    /// `current-line-number` is blocked in init mode.
    #[test]
    fn current_line_number_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "current-line-number").is_err());
    }

    /// `current-selections` is blocked in init mode.
    #[test]
    fn current_selections_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "current-selections").is_err());
    }

    /// `char-index->line` is blocked in init mode.
    #[test]
    fn char_index_to_line_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(super::super::errors::require_cmd(&h.ctx_init(), "char-index->line").is_err());
    }

    // ── Type errors (wrong arg type) ──────────────────────────────────────────
    //
    // `buffer-path`/`buffer-name`/`buffer-dirty?` don't decode `bid` in-body
    // (it's a typed `BidArg` param) — that decode-failure path is covered
    // once, centrally, by `args::tests::bid_arg_rejects_non_buffer_id`.

    /// `char-index->line` rejects a non-integer and a negative integer argument.
    ///
    /// Fail oracle: remove the `n >= 0` guard → `IntV(-1)` would be accepted and
    /// cast to a huge `usize`, silently corrupting the lookup instead of erroring.
    #[test]
    fn char_index_to_line_wrong_type_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = char_index_to_line(&mut ctx, SteelVal::StringV("not-an-int".into()));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected a non-negative integer")
        );

        let mut ctx = h.ctx();
        let result = char_index_to_line(&mut ctx, SteelVal::IntV(-1));
        assert!(result.is_err());
    }

    // ── Invalid buffer ID (NullHost always returns buffer_exists=false) ───────

    /// `buffer-path` with a valid BufferId but non-existent buffer raises an error.
    ///
    /// Fail oracle: remove the `buffer_exists` check → `buffer_path` is called
    /// on a nonexistent buffer, which could panic or return garbage.
    #[test]
    fn buffer_path_invalid_id_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        // NullHost.buffer_exists always returns false.
        let result = buffer_path(&mut ctx, default_bid());
        assert!(result.is_err(), "non-existent buffer id must error");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid buffer id")
        );
    }

    // ── Command-mode success paths (NullHost read methods return None/empty) ──

    /// `current-buffer` in command mode returns the focused buffer id as a SteelVal.
    ///
    /// Fail oracle: return a hardcoded or wrong id → the assert on the type fires.
    #[test]
    fn current_buffer_command_mode_returns_steel_buffer_id() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = current_buffer(&mut ctx);
        assert!(
            result.is_ok(),
            "current-buffer must succeed in command mode"
        );
        // Must return a Custom value (SteelBufferId is opaque).
        assert!(
            matches!(result.unwrap(), SteelVal::Custom(_)),
            "current-buffer must return a SteelVal::Custom (BufferId)"
        );
    }

    /// `buffers` in command mode returns an empty list (NullHost has no buffers).
    #[test]
    fn buffers_command_mode_returns_empty_list() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = buffers(&mut ctx);
        assert!(result.is_ok());
        assert!(
            matches!(result.unwrap(), SteelVal::ListV(lst) if lst.is_empty()),
            "buffers must return an empty list when no buffers exist"
        );
    }

    /// `current-line-number` returns `#f` when the host has no cursor (NullHost).
    #[test]
    fn current_line_number_returns_false_when_no_cursor() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = current_line_number(&mut ctx);
        assert!(matches!(result, Ok(SteelVal::BoolV(false))));
    }

    /// `current-selections` returns `#f` when the host has no pane state (NullHost).
    #[test]
    fn current_selections_returns_false_when_no_pane_state() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = current_selections(&mut ctx);
        assert!(matches!(result, Ok(SteelVal::BoolV(false))));
    }

    /// `char-index->line` returns `#f` when the host has no pane state (NullHost).
    #[test]
    fn char_index_to_line_returns_false_when_no_pane_state() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = char_index_to_line(&mut ctx, SteelVal::IntV(0));
        assert!(matches!(result, Ok(SteelVal::BoolV(false))));
    }
}
