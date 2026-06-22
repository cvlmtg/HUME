//! Multi-buffer Steel builtins — buffer/pane query and lifecycle ops.
//!
//! All builtins guard against init-eval context (`ctx.is_init = true`), where
//! editor refs are not available.  Calling any of these from `init.scm` raises
//! a Steel error instead of returning a meaningless default.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use super::{
    ids::{SteelBufferId, SteelPaneId, downcast_buffer_id},
    require_cmd_ctx,
};
use crate::SteelCtx;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Focus builtins ─────────────────────────────────────────────────────────────

/// `(current-buffer)` → BufferId of the focused buffer at dispatch time.
pub(crate) fn current_buffer(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "current-buffer");
    Ok(SteelBufferId(ctx.focused_buffer_id).into_steel_val())
}

/// `(current-pane)` → PaneId of the focused pane at dispatch time.
pub(crate) fn current_pane(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "current-pane");
    Ok(SteelPaneId(ctx.focused_pane_id).into_steel_val())
}

// ── Enumeration builtins ───────────────────────────────────────────────────────

/// `(buffers)` → list of all open BufferIds in open-order.
pub(crate) fn buffers(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "buffers");
    let list: Vec<SteelVal> = ctx
        .host
        .buffer_ids()
        .into_iter()
        .map(|id| SteelBufferId(id).into_steel_val())
        .collect();
    list.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(panes)` → list of all open PaneIds.
pub(crate) fn panes(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "panes");
    let list: Vec<SteelVal> = ctx
        .host
        .pane_ids()
        .into_iter()
        .map(|id| SteelPaneId(id).into_steel_val())
        .collect();
    list.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

// ── Buffer property builtins ───────────────────────────────────────────────────

/// `(buffer-path bid)` → absolute path string, or `#f` for unsaved buffers.
pub(crate) fn buffer_path(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-path");
    let id = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "buffer-path: expected buffer-id".into(),
        )
    })?;
    if !ctx.host.buffer_exists(id) {
        steel::stop!(Generic => "buffer-path: invalid buffer id {id:?}");
    }
    match ctx.host.buffer_path(id) {
        Some(p) => p
            .to_string_lossy()
            .into_owned()
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-name bid)` → display name (filename or `"*scratch*"`).
pub(crate) fn buffer_name(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-name");
    let id = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "buffer-name: expected buffer-id".into(),
        )
    })?;
    ctx.host
        .buffer_display_name(id)
        .ok_or_else(|| {
            SteelErr::new(
                ErrorKind::Generic,
                format!("buffer-name: invalid buffer id {id:?}"),
            )
        })?
        .into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(buffer-dirty? bid)` → `#t` if the buffer has unsaved edits.
pub(crate) fn buffer_dirty(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-dirty?");
    let id = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "buffer-dirty?: expected buffer-id".into(),
        )
    })?;
    let dirty = ctx.host.buffer_is_dirty(id).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::Generic,
            format!("buffer-dirty?: invalid buffer id {id:?}"),
        )
    })?;
    Ok(SteelVal::BoolV(dirty))
}

// ── Mutating builtins ─────────────────────────────────────────────────────────

/// `(open-buffer! path)` → BufferId.
///
/// Opens `path` as a new buffer and returns its `BufferId`. If the path is
/// already open, returns the existing id without opening a new buffer. Does
/// not switch the focused pane — call `(switch-to-buffer! bid)` separately
/// if desired.
pub(crate) fn open_buffer(ctx: &mut SteelCtx, path: String) -> SteelResult {
    require_cmd_ctx!(ctx, "open-buffer!");
    let bid = ctx
        .host
        .open_buffer(std::path::Path::new(&path))
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
    SteelBufferId(bid)
        .into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(close-buffer! bid)` → void.
///
/// Closes the buffer identified by `bid`. Raises a Steel error for an invalid
/// or unknown `bid`.
pub(crate) fn close_buffer(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "close-buffer!");
    let id = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "close-buffer!: expected buffer-id".into(),
        )
    })?;
    if !ctx.host.buffer_exists(id) {
        steel::stop!(Generic => "close-buffer!: invalid buffer id {id:?}");
    }
    let new_live = ctx
        .host
        .close_buffer(id)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
    ctx.live_focused_buffer_id = new_live;
    Ok(SteelVal::Void)
}

/// `(switch-to-buffer! bid)` → void.
///
/// Redirects the focused pane to the buffer identified by `bid`, recording
/// the current position in the jump list. Raises a Steel error for an invalid
/// or unknown `bid`.
pub(crate) fn switch_to_buffer(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "switch-to-buffer!");
    let target = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "switch-to-buffer!: expected buffer-id".into(),
        )
    })?;
    if !ctx.host.buffer_exists(target) {
        steel::stop!(Generic => "switch-to-buffer!: invalid buffer id {target:?}");
    }
    let current = ctx.live_focused_buffer_id;
    ctx.host
        .switch_to_buffer(current, target)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
    ctx.live_focused_buffer_id = target;
    Ok(SteelVal::Void)
}

// ── Language builtins ─────────────────────────────────────────────────────────

/// Reverse-scan `pending` for the last `set-buffer-language!` call for `id`;
/// fall back to `fallback` (the buffer's stored language).
fn effective_language(
    pending: &[(hume_engine::pipeline::BufferId, Option<String>)],
    id: hume_engine::pipeline::BufferId,
    fallback: Option<String>,
) -> Option<String> {
    pending
        .iter()
        .rev()
        .find(|(bid, _)| *bid == id)
        .map(|(_, lang)| lang.clone())
        .unwrap_or(fallback)
}

/// `(buffer-language bid)` → string or `#f`.
pub(crate) fn buffer_language(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-language");
    let id = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "buffer-language: expected buffer-id".into(),
        )
    })?;
    if !ctx.host.buffer_exists(id) {
        steel::stop!(Generic => "buffer-language: invalid buffer id {id:?}");
    }
    let fallback = ctx.host.buffer_stored_language(id);
    let lang = effective_language(&ctx.pending_language_sets, id, fallback);
    match lang {
        Some(name) => name
            .into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

// ── Live cursor/selection reads ───────────────────────────────────────────────

/// `(current-line-number)` → 1-indexed line number of the primary cursor, or `#f`.
///
/// Reads live state — reflects any synchronous edits or motions that ran
/// earlier in the same Steel eval (e.g. after `(move-left)`).
pub(crate) fn current_line_number(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "current-line-number");
    match ctx.host.current_line_number() {
        Some(n) => Ok(SteelVal::IntV(n as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(cursor-char-index)` → char-index of the primary cursor head, or `#f`.
///
/// Reads live state — reflects any synchronous edits or motions that ran
/// earlier in the same Steel eval.
pub(crate) fn cursor_char_index(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "cursor-char-index");
    match ctx.host.cursor_char_index() {
        Some(n) => Ok(SteelVal::IntV(n as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(set-buffer-language! bid lang-or-#f)` — deferred; applied after the eval returns.
pub(crate) fn set_buffer_language_steel(
    ctx: &mut SteelCtx,
    bid: SteelVal,
    lang: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "set-buffer-language!");
    let id = downcast_buffer_id(&bid).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::TypeMismatch,
            "set-buffer-language!: expected buffer-id".into(),
        )
    })?;
    let new_lang = match &lang {
        SteelVal::StringV(s) => Some(s.to_string()),
        SteelVal::BoolV(false) => None,
        _ => {
            steel::stop!(TypeMismatch => "set-buffer-language!: expected string or #f, got {:?}", lang)
        }
    };
    if !ctx.host.buffer_exists(id) {
        steel::stop!(Generic => "set-buffer-language!: invalid buffer id {id:?}");
    }
    let fallback = ctx.host.buffer_stored_language(id);
    if effective_language(&ctx.pending_language_sets, id, fallback) == new_lang {
        return Ok(SteelVal::Void);
    }
    ctx.pending_language_sets.push((id, new_lang));
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::ids::SteelBufferId;
    use crate::test_support::SteelCtxTestHarness;
    use hume_engine::pipeline::BufferId;
    use steel::rvals::IntoSteelVal;

    fn default_bid() -> SteelVal {
        SteelBufferId(BufferId::default())
            .into_steelval()
            .expect("SteelBufferId IntoSteelVal")
    }

    // ── require_cmd_ctx! guard (init mode rejection) ─────────────────────────

    /// `current-buffer` is blocked in init mode.
    ///
    /// Fail oracle: remove `require_cmd_ctx!` → `focused_buffer_id` (which is
    /// Default in init) would be returned, silently giving wrong data.
    #[test]
    fn current_buffer_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = current_buffer(&mut ctx);
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
        let mut ctx = h.ctx_init();
        assert!(current_pane(&mut ctx).is_err());
    }

    /// `buffers` is blocked in init mode.
    #[test]
    fn buffers_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(buffers(&mut ctx).is_err());
    }

    /// `panes` is blocked in init mode.
    #[test]
    fn panes_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(panes(&mut ctx).is_err());
    }

    /// `buffer-path` is blocked in init mode.
    #[test]
    fn buffer_path_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(buffer_path(&mut ctx, default_bid()).is_err());
    }

    /// `buffer-name` is blocked in init mode.
    #[test]
    fn buffer_name_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(buffer_name(&mut ctx, default_bid()).is_err());
    }

    /// `buffer-dirty?` is blocked in init mode.
    #[test]
    fn buffer_dirty_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(buffer_dirty(&mut ctx, default_bid()).is_err());
    }

    /// `set-buffer-language!` is blocked in init mode.
    #[test]
    fn set_buffer_language_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = set_buffer_language_steel(&mut ctx, default_bid(), SteelVal::BoolV(false));
        assert!(
            result.is_err(),
            "set-buffer-language! must error in init mode"
        );
    }

    /// `current-line-number` is blocked in init mode.
    #[test]
    fn current_line_number_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(current_line_number(&mut ctx).is_err());
    }

    /// `cursor-char-index` is blocked in init mode.
    #[test]
    fn cursor_char_index_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        assert!(cursor_char_index(&mut ctx).is_err());
    }

    // ── Type errors (wrong arg type) ──────────────────────────────────────────

    /// `buffer-path` rejects a non-BufferId argument.
    ///
    /// Fail oracle: remove the `downcast_buffer_id` check → any SteelVal would be
    /// accepted and a default BufferId would be used silently.
    #[test]
    fn buffer_path_wrong_type_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = buffer_path(&mut ctx, SteelVal::StringV("not-an-id".into()));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected buffer-id")
        );
    }

    /// `buffer-name` rejects a non-BufferId argument.
    #[test]
    fn buffer_name_wrong_type_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = buffer_name(&mut ctx, SteelVal::IntV(0));
        assert!(result.is_err());
    }

    /// `buffer-dirty?` rejects a non-BufferId argument.
    #[test]
    fn buffer_dirty_wrong_type_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = buffer_dirty(&mut ctx, SteelVal::BoolV(true));
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

    /// `cursor-char-index` returns `#f` when the host has no cursor (NullHost).
    #[test]
    fn cursor_char_index_returns_false_when_no_cursor() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = cursor_char_index(&mut ctx);
        assert!(matches!(result, Ok(SteelVal::BoolV(false))));
    }
}
