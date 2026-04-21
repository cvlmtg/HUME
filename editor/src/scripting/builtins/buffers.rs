//! Multi-buffer Steel builtins — read-only (Phase 3) and mutating (Phase 4).
//!
//! All builtins guard against init-eval context (`ctx.is_init = true`), where
//! editor refs are `None`.  Calling any of these from `init.scm` raises a Steel
//! error instead of returning a meaningless default.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use engine::pipeline::BufferId;

use crate::editor::buffer::Buffer;
use crate::scripting::SteelCtx;
use super::{ids::{SteelBufferId, SteelPaneId}, require_cmd_ctx};

type SteelResult = Result<SteelVal, SteelErr>;

/// Extract the inner `BufferId` from a `SteelVal::Custom(SteelBufferId)`.
fn extract_buffer_id(val: &SteelVal) -> Option<BufferId> {
    if let SteelVal::Custom(v) = val {
        use steel::gc::ShareableMut as _;
        v.read().as_any_ref().downcast_ref::<SteelBufferId>().map(|b| b.0)
    } else {
        None
    }
}

// ── Focus builtins ─────────────────────────────────────────────────────────────

/// `(current-buffer)` → BufferId of the focused buffer at dispatch time.
pub(crate) fn current_buffer(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "current-buffer");
    SteelBufferId(ctx.focused_buffer_id).into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(current-pane)` → PaneId of the focused pane at dispatch time.
pub(crate) fn current_pane(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "current-pane");
    SteelPaneId(ctx.focused_pane_id).into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

// ── Enumeration builtins ───────────────────────────────────────────────────────

/// `(buffers)` → list of all open BufferIds in open-order.
pub(crate) fn buffers(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "buffers");
    let store = ctx.buffers.as_deref().expect("buffers is Some when is_init = false");
    let list: Vec<SteelVal> = store.iter()
        .map(|(id, _)| SteelBufferId(id).into_steel_val())
        .collect();
    list.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(panes)` → list of all open PaneIds.
pub(crate) fn panes(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "panes");
    let ev = ctx.engine_view.as_deref().expect("engine_view is Some when is_init = false");
    let list: Vec<SteelVal> = ev.panes.iter()
        .map(|(id, _)| SteelPaneId(id).into_steelval().expect("SteelPaneId into_steelval"))
        .collect();
    list.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

// ── Buffer property builtins ───────────────────────────────────────────────────

/// `(buffer-path bid)` → absolute path string, or `#f` for unsaved buffers.
pub(crate) fn buffer_path(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-path");
    let id = extract_buffer_id(&bid)
        .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "buffer-path: expected buffer-id".into()))?;
    let store = ctx.buffers.as_deref().expect("buffers is Some when is_init = false");
    let buf = store.try_get(id)
        .ok_or_else(|| SteelErr::new(ErrorKind::Generic, format!("buffer-path: invalid buffer id {id:?}")))?;
    match buf.path.as_deref() {
        Some(p) => p.to_string_lossy().into_owned().into_steelval()
            .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string())),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-name bid)` → display name (filename or `"*scratch*"`).
pub(crate) fn buffer_name(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-name");
    let id = extract_buffer_id(&bid)
        .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "buffer-name: expected buffer-id".into()))?;
    let store = ctx.buffers.as_deref().expect("buffers is Some when is_init = false");
    let buf = store.try_get(id)
        .ok_or_else(|| SteelErr::new(ErrorKind::Generic, format!("buffer-name: invalid buffer id {id:?}")))?;
    let name = buf.path.as_deref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or(Buffer::SCRATCH_BUFFER_NAME);
    name.into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(buffer-dirty? bid)` → `#t` if the buffer has unsaved edits.
pub(crate) fn buffer_dirty(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "buffer-dirty?");
    let id = extract_buffer_id(&bid)
        .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "buffer-dirty?: expected buffer-id".into()))?;
    let store = ctx.buffers.as_deref().expect("buffers is Some when is_init = false");
    let buf = store.try_get(id)
        .ok_or_else(|| SteelErr::new(ErrorKind::Generic, format!("buffer-dirty?: invalid buffer id {id:?}")))?;
    Ok(SteelVal::BoolV(buf.is_dirty()))
}

// ── Mutating builtins ─────────────────────────────────────────────────────────

/// `(open-buffer! path)` → BufferId.
///
/// Opens `path` as a new buffer and returns its `BufferId`. If the path is
/// already open (`find_by_path` dedup), returns the existing id without
/// opening a new buffer. Does not switch the focused pane — call
/// `(switch-to-buffer! bid)` separately if desired.
pub(crate) fn open_buffer(ctx: &mut SteelCtx, path: String) -> SteelResult {
    require_cmd_ctx!(ctx, "open-buffer!");
    let p = std::path::Path::new(&path);
    let canonical = crate::os::fs::canonicalize(p)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, format!("open-buffer!: {}: {e}", p.display())))?;
    let focused_pane_id = ctx.focused_pane_id;
    let ev   = ctx.engine_view.as_deref_mut().expect("engine_view is Some when is_init = false");
    let bufs = ctx.buffers.as_deref_mut().expect("buffers is Some when is_init = false");
    let ps   = ctx.pane_state.as_deref_mut().expect("pane_state is Some when is_init = false");
    let (bid, _) = crate::editor::ops::open_or_dedup(ev, bufs, ps, focused_pane_id, &canonical)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, format!("open-buffer!: {}: {e}", canonical.display())))?;
    SteelBufferId(bid).into_steelval()
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e.to_string()))
}

/// `(close-buffer! bid)` → void.
///
/// Closes the buffer identified by `bid`. If it is the only open buffer,
/// replaces it in-place with a fresh scratch buffer. Raises a Steel error for
/// an invalid or unknown `bid`.
pub(crate) fn close_buffer(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "close-buffer!");
    let id = extract_buffer_id(&bid)
        .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "close-buffer!: expected buffer-id".into()))?;
    if ctx.buffers.as_deref()
        .expect("buffers is Some when is_init = false")
        .try_get(id)
        .is_none()
    {
        steel::stop!(Generic => "close-buffer!: invalid buffer id {id:?}");
    }
    let focused_pane_id = ctx.focused_pane_id;
    let ev    = ctx.engine_view.as_deref_mut().expect("engine_view is Some when is_init = false");
    let bufs  = ctx.buffers.as_deref_mut().expect("buffers is Some when is_init = false");
    let ps    = ctx.pane_state.as_deref_mut().expect("pane_state is Some when is_init = false");
    let jumps = ctx.pane_jumps.as_deref_mut().expect("pane_jumps is Some when is_init = false");
    let new_live = crate::editor::ops::close_buffer(ev, bufs, ps, jumps, focused_pane_id, id);
    ctx.live_focused_buffer_id = new_live;
    Ok(SteelVal::Void)
}

/// `(switch-to-buffer! bid)` → void.
///
/// Redirects the focused pane to the buffer identified by `bid`, recording the
/// current position in the jump list. Raises a Steel error for an invalid or
/// unknown `bid`.
pub(crate) fn switch_to_buffer(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "switch-to-buffer!");
    let target = extract_buffer_id(&bid)
        .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "switch-to-buffer!: expected buffer-id".into()))?;
    if ctx.buffers.as_deref()
        .expect("buffers is Some when is_init = false")
        .try_get(target)
        .is_none()
    {
        steel::stop!(Generic => "switch-to-buffer!: invalid buffer id {target:?}");
    }
    let focused_pane_id = ctx.focused_pane_id;
    let current = ctx.live_focused_buffer_id;
    let ev    = ctx.engine_view.as_deref_mut().expect("engine_view is Some when is_init = false");
    let bufs  = ctx.buffers.as_deref_mut().expect("buffers is Some when is_init = false");
    let ps    = ctx.pane_state.as_deref_mut().expect("pane_state is Some when is_init = false");
    let jumps = ctx.pane_jumps.as_deref_mut().expect("pane_jumps is Some when is_init = false");
    crate::editor::ops::switch_to_buffer_with_jump(ev, bufs, ps, jumps, focused_pane_id, current, target);
    ctx.live_focused_buffer_id = target;
    Ok(SteelVal::Void)
}

// ── Pane stubs (Phase 5 — reserved for M9+) ──────────────────────────────────

fn pane_stub(builtin_name: &str) -> SteelResult {
    steel::stop!(Generic => "{}: pane operations require :split, deferred to M9+", builtin_name)
}

/// `(open-pane! bid)` — reserved; pane split operations land in M9+.
pub(crate) fn open_pane(_ctx: &mut SteelCtx, _bid: SteelVal) -> SteelResult { pane_stub("open-pane!") }

/// `(close-pane! pid)` — reserved; pane split operations land in M9+.
pub(crate) fn close_pane(_ctx: &mut SteelCtx, _pid: SteelVal) -> SteelResult { pane_stub("close-pane!") }

/// `(focus-pane! pid)` — reserved; pane split operations land in M9+.
pub(crate) fn focus_pane(_ctx: &mut SteelCtx, _pid: SteelVal) -> SteelResult { pane_stub("focus-pane!") }

/// `(pane-buffer pid)` — reserved; pane split operations land in M9+.
pub(crate) fn pane_buffer(_ctx: &mut SteelCtx, _pid: SteelVal) -> SteelResult { pane_stub("pane-buffer") }

/// `(pane-set-buffer! pid bid)` — reserved; pane split operations land in M9+.
pub(crate) fn pane_set_buffer(_ctx: &mut SteelCtx, _pid: SteelVal, _bid: SteelVal) -> SteelResult { pane_stub("pane-set-buffer!") }

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use slotmap::SecondaryMap;

    use engine::pipeline::{BufferId, EngineView, PaneId, SharedBuffer};
    use engine::pane::Pane;
    use engine::theme::Theme;

    use crate::core::jump_list::JumpList;
    use crate::core::text::Text;
    use crate::core::selection::SelectionSet;
    use crate::editor::buffer::Buffer;
    use crate::editor::buffer_store::BufferStore;
    use crate::editor::pane_state::PaneBufferState;
    use crate::editor::keymap::Keymap;
    use crate::settings::EditorSettings;
    use crate::scripting::{EditorSteelRefs, ScriptingHost};

    // ── Test fixture helpers ──────────────────────────────────────────────────

    type BufState = (
        BufferStore,
        EngineView,
        SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        SecondaryMap<PaneId, JumpList>,
        PaneId,
        BufferId,
    );

    fn host() -> ScriptingHost { ScriptingHost::new() }

    #[allow(clippy::too_many_arguments)]
    fn mb_refs<'a>(
        s: &'a mut EditorSettings,
        km: &'a mut Keymap,
        pane_id: PaneId,
        buf_id: BufferId,
        bufs: &'a mut BufferStore,
        ev: &'a mut EngineView,
        ps: &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        pj: Option<&'a mut SecondaryMap<PaneId, JumpList>>,
    ) -> EditorSteelRefs<'a> {
        EditorSteelRefs {
            settings:          s,
            keymap:            km,
            focused_pane_id:   pane_id,
            focused_buffer_id: buf_id,
            buffers:           Some(bufs),
            engine_view:       Some(ev),
            pane_state:        Some(ps),
            pane_jumps:        pj,
        }
    }

    /// Create a minimal one-buffer, one-pane editor state for tests.
    fn one_buf_state() -> BufState {
        let mut ev = EngineView::new(Theme::default());
        let buffer_id = ev.buffers.insert(SharedBuffer::new());
        let pane_id = ev.panes.insert(Pane::new(buffer_id));
        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, Buffer::new(Text::from("hello\n"), SelectionSet::default()));
        let mut pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
        pane_state.insert(pane_id, SecondaryMap::new());
        pane_state[pane_id].insert(buffer_id, PaneBufferState::default());
        let mut pane_jumps: SecondaryMap<PaneId, JumpList> = SecondaryMap::new();
        pane_jumps.insert(pane_id, JumpList::new(100));
        (buffers, ev, pane_state, pane_jumps, pane_id, buffer_id)
    }

    // ── (current-buffer) / (current-pane) ────────────────────────────────────

    #[test]
    fn current_buffer_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(current-buffer)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn current_pane_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(current-pane)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn current_buffer_returns_buffer_id() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        // A command that succeeds only if current-buffer returns a buffer-id.
        h.eval_source(
            r#"(define-command! "check-buf" ""
                 (lambda () (if (buffer-id? (current-buffer)) (call! "move-right") (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-buf", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "current-buffer must return a buffer-id");
    }

    #[test]
    fn current_pane_returns_pane_id() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-pane" ""
                 (lambda () (if (pane-id? (current-pane)) (call! "move-right") (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-pane", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "current-pane must return a pane-id");
    }

    // ── (buffers) / (panes) ───────────────────────────────────────────────────

    #[test]
    fn buffers_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(buffers)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn buffers_returns_list_of_buffer_ids() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-bufs" ""
                 (lambda ()
                   (let ((bs (buffers)))
                     (if (and (= (length bs) 1) (buffer-id? (car bs)))
                         (call! "move-right")
                         (call! "move-left")))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-bufs", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "(buffers) must return a list of one buffer-id");
    }

    #[test]
    fn panes_returns_list_of_pane_ids() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-panes" ""
                 (lambda ()
                   (let ((ps (panes)))
                     (if (and (= (length ps) 1) (pane-id? (car ps)))
                         (call! "move-right")
                         (call! "move-left")))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-panes", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "(panes) must return a list of one pane-id");
    }

    // ── (buffer-path bid) ─────────────────────────────────────────────────────

    #[test]
    fn buffer_path_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // Calling with any non-buffer arg also errors in init mode (init check fires first).
        let err = h.eval_source("(buffer-path #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn buffer_path_returns_false_for_scratch() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();
        // scratch buffer: path = None

        h.eval_source(
            r#"(define-command! "check-path-scratch" ""
                 (lambda ()
                   (if (equal? (buffer-path (current-buffer)) #f)
                       (call! "move-right")
                       (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-path-scratch", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-path of scratch should be #f");
    }

    #[test]
    fn buffer_path_returns_path_string() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();
        // Set a path on the buffer.
        bufs.get_mut(buf_id).path = Some(Arc::new(std::path::PathBuf::from("/tmp/test.txt")));

        h.eval_source(
            r#"(define-command! "check-path" ""
                 (lambda ()
                   (let ((p (buffer-path (current-buffer))))
                     (if (string? p) (call! "move-right") (call! "move-left")))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-path", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-path should return a string for named buffer");
    }

    // ── (buffer-name bid) ─────────────────────────────────────────────────────

    #[test]
    fn buffer_name_returns_scratch_for_unnamed() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-name-scratch" ""
                 (lambda ()
                   (if (equal? (buffer-name (current-buffer)) "*scratch*")
                       (call! "move-right")
                       (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-name-scratch", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-name of scratch should be *scratch*");
    }

    #[test]
    fn buffer_name_returns_filename() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();
        bufs.get_mut(buf_id).path = Some(Arc::new(std::path::PathBuf::from("/tmp/hello.rs")));

        h.eval_source(
            r#"(define-command! "check-name" ""
                 (lambda ()
                   (if (equal? (buffer-name (current-buffer)) "hello.rs")
                       (call! "move-right")
                       (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-name", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-name should return the filename");
    }

    // ── (buffer-dirty? bid) ───────────────────────────────────────────────────

    #[test]
    fn buffer_dirty_false_for_clean_buffer() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-dirty" ""
                 (lambda ()
                   (if (buffer-dirty? (current-buffer))
                       (call! "move-left")
                       (call! "move-right"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-dirty", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "new buffer should not be dirty");
    }

    // ── invalid buffer id ─────────────────────────────────────────────────────

    #[test]
    fn buffer_path_errors_on_invalid_id() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, _pj, pane_id, buf_id) = one_buf_state();

        // Pass a valid buffer-id? type but stale (not in the store).
        // We use the default (null) key which is never registered.
        h.eval_source(
            r#"(define-command! "check-invalid" ""
                 (lambda () (buffer-path (current-buffer))))"#,
            &mut s, &mut km,
        ).unwrap();

        // Close the buffer then call — buf_id becomes stale.
        bufs.close(buf_id);

        let err = h.call_steel_cmd(
            "%hume-cmd-check-invalid", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, None),
        ).unwrap_err();
        assert!(err.contains("invalid buffer id"), "got: {err}");
    }

    // ── (open-buffer! path) ───────────────────────────────────────────────────

    #[test]
    fn open_buffer_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(open-buffer! \"/tmp/no.txt\")", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn open_buffer_opens_new_file() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, mut pj, pane_id, buf_id) = one_buf_state();

        let path = std::env::temp_dir().join("hume_test_open_buffer.txt");
        std::fs::write(&path, "test content\n").unwrap();

        let escaped = path.display().to_string().replace('\\', "\\\\");
        h.eval_source(
            &format!(r#"(define-command! "do-open" "" (lambda () (open-buffer! "{escaped}")))"#),
            &mut s, &mut km,
        ).unwrap();

        let (_, _) = h.call_steel_cmd(
            "%hume-cmd-do-open", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, Some(&mut pj)),
        ).unwrap();

        assert_eq!(bufs.len(), 2, "(open-buffer!) should add a second buffer");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_buffer_dedup_returns_existing() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, mut pj, pane_id, _buf_id) = one_buf_state();

        let path = std::env::temp_dir().join("hume_test_open_dedup.txt");
        std::fs::write(&path, "dedup\n").unwrap();
        let canonical = std::fs::canonicalize(&path).unwrap();
        let canonical_str = canonical.display().to_string().replace('\\', "\\\\");

        // Open the file once to pre-seed the store.
        let first_id = crate::editor::ops::open_buffer(
            &mut ev, &mut bufs, &mut ps, pane_id,
            crate::editor::buffer::Buffer::from_file(&canonical).unwrap(),
        );

        h.eval_source(
            &format!(r#"
(define-command! "open-twice" ""
  (lambda ()
    (let ((b1 (open-buffer! "{canonical_str}"))
          (b2 (open-buffer! "{canonical_str}")))
      (if (equal? b1 b2) (call! "move-right") (call! "move-left")))))"#),
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-open-twice", None, None,
            mb_refs(&mut s, &mut km, pane_id, first_id, &mut bufs, &mut ev, &mut ps, Some(&mut pj)),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "dedup: same path must return same BufferId");
        assert_eq!(bufs.len(), 2, "no extra buffer should be created on dedup");
        let _ = std::fs::remove_file(&path);
    }

    // ── (close-buffer! bid) ───────────────────────────────────────────────────

    #[test]
    fn close_buffer_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(close-buffer! #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn close_buffer_removes_second_buffer() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, mut pj, pane_id, _buf_id) = one_buf_state();

        // Add a second buffer via ops so we have something to close.
        let second_id = crate::editor::ops::open_buffer(
            &mut ev, &mut bufs, &mut ps, pane_id,
            Buffer::new(Text::from("second\n"), SelectionSet::default()),
        );
        assert_eq!(bufs.len(), 2, "precondition: 2 buffers");

        h.eval_source(
            r#"(define-command! "do-close" "" (lambda () (close-buffer! (current-buffer))))"#,
            &mut s, &mut km,
        ).unwrap();

        // Dispatch with second_id as the focused buffer.
        h.call_steel_cmd(
            "%hume-cmd-do-close", None, None,
            mb_refs(&mut s, &mut km, pane_id, second_id, &mut bufs, &mut ev, &mut ps, Some(&mut pj)),
        ).unwrap();

        assert_eq!(bufs.len(), 1, "(close-buffer!) must remove the buffer");
    }

    #[test]
    fn close_last_buffer_becomes_scratch() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, mut pj, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "do-close-last" "" (lambda () (close-buffer! (current-buffer))))"#,
            &mut s, &mut km,
        ).unwrap();

        h.call_steel_cmd(
            "%hume-cmd-do-close-last", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, Some(&mut pj)),
        ).unwrap();

        // The only buffer becomes scratch — count stays at 1.
        assert_eq!(bufs.len(), 1, "last buffer must be replaced by scratch, not removed");
        assert!(bufs.get(buf_id).path.is_none(), "scratch buffer has no path");
    }

    // ── (switch-to-buffer! bid) ───────────────────────────────────────────────

    #[test]
    fn switch_to_buffer_errors_in_init_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(switch-to-buffer! #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("not available during init"), "got: {err}");
    }

    #[test]
    fn switch_to_buffer_changes_pane_state() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, mut pj, pane_id, buf_id) = one_buf_state();

        // Add a second buffer.
        let second_id = crate::editor::ops::open_buffer(
            &mut ev, &mut bufs, &mut ps, pane_id,
            Buffer::new(Text::from("second\n"), SelectionSet::default()),
        );

        // Command that switches to the second buffer in open-order.
        // (buffers) = [buf_id, second_id]; (car (cdr ...)) = second element.
        h.eval_source(
            r#"(define-command! "do-switch" ""
                 (lambda ()
                   (let ((bs (buffers)))
                     (when (= (length bs) 2)
                       (switch-to-buffer! (car (cdr bs)))))))"#,
            &mut s, &mut km,
        ).unwrap();

        h.call_steel_cmd(
            "%hume-cmd-do-switch", None, None,
            mb_refs(&mut s, &mut km, pane_id, buf_id, &mut bufs, &mut ev, &mut ps, Some(&mut pj)),
        ).unwrap();

        assert_eq!(
            ev.panes[pane_id].buffer_id, second_id,
            "pane must be viewing second buffer after switch-to-buffer!",
        );
    }

    // ── Pane stubs (Phase 5) ──────────────────────────────────────────────────

    #[test]
    fn open_pane_returns_deferred_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(open-pane! #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn close_pane_returns_deferred_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(close-pane! #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn focus_pane_returns_deferred_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(focus-pane! #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn pane_buffer_returns_deferred_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(pane-buffer #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }

    #[test]
    fn pane_set_buffer_returns_deferred_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(pane-set-buffer! #f #f)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("deferred to M9+"), "got: {err}");
    }
}
