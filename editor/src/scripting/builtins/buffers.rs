//! Read-only multi-buffer Steel builtins (Phase 3).
//!
//! All builtins guard against init-eval context (`ctx.is_init = true`), where
//! editor refs are `None`.  Calling any of these from `init.scm` raises a Steel
//! error instead of returning a meaningless default.

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{IntoSteelVal, SteelVal};

use engine::pipeline::BufferId;

use crate::scripting::SteelCtx;
use super::ids::{SteelBufferId, SteelPaneId};

type SteelResult = Result<SteelVal, SteelErr>;

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Return `Err` if we're inside an init eval (editor refs are None).
macro_rules! require_cmd_ctx {
    ($ctx:expr, $name:literal) => {
        if $ctx.is_init {
            steel::stop!(Generic => "{}: not available during init evaluation", $name);
        }
    };
}

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
        .map(|(id, _)| SteelBufferId(id).into_steelval().expect("SteelBufferId into_steelval"))
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
        .unwrap_or("*scratch*");
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

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use slotmap::SecondaryMap;

    use engine::pipeline::{BufferId, EngineView, PaneId, SharedBuffer};
    use engine::pane::Pane;
    use engine::theme::Theme;

    use crate::core::text::Text;
    use crate::core::selection::SelectionSet;
    use crate::editor::buffer::Buffer;
    use crate::editor::buffer_store::BufferStore;
    use crate::editor::pane_state::PaneBufferState;
    use crate::editor::keymap::Keymap;
    use crate::settings::EditorSettings;
    use crate::scripting::ScriptingHost;

    // ── Test fixture helpers ──────────────────────────────────────────────────

    fn host() -> ScriptingHost { ScriptingHost::new() }

    /// Create a minimal one-buffer, one-pane editor state for tests.
    fn one_buf_state() -> (
        BufferStore,
        EngineView,
        SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        PaneId,
        BufferId,
    ) {
        let mut ev = EngineView::new(Theme::default());
        let buffer_id = ev.buffers.insert(SharedBuffer::new());
        let pane_id = ev.panes.insert(Pane::new(buffer_id));
        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, Buffer::new(Text::from("hello\n"), SelectionSet::default()));
        let mut pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
        pane_state.insert(pane_id, SecondaryMap::new());
        pane_state[pane_id].insert(buffer_id, PaneBufferState::default());
        (buffers, ev, pane_state, pane_id, buffer_id)
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
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

        // A command that succeeds only if current-buffer returns a buffer-id.
        h.eval_source(
            r#"(define-command! "check-buf" ""
                 (lambda () (if (buffer-id? (current-buffer)) (call! "move-right") (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-buf", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "current-buffer must return a buffer-id");
    }

    #[test]
    fn current_pane_returns_pane_id() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-pane" ""
                 (lambda () (if (pane-id? (current-pane)) (call! "move-right") (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-pane", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
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
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

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
            "%hume-cmd-check-bufs", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "(buffers) must return a list of one buffer-id");
    }

    #[test]
    fn panes_returns_list_of_pane_ids() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

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
            "%hume-cmd-check-panes", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
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
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();
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
            "%hume-cmd-check-path-scratch", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-path of scratch should be #f");
    }

    #[test]
    fn buffer_path_returns_path_string() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();
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
            "%hume-cmd-check-path", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-path should return a string for named buffer");
    }

    // ── (buffer-name bid) ─────────────────────────────────────────────────────

    #[test]
    fn buffer_name_returns_scratch_for_unnamed() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-name-scratch" ""
                 (lambda ()
                   (if (equal? (buffer-name (current-buffer)) "*scratch*")
                       (call! "move-right")
                       (call! "move-left"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-name-scratch", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-name of scratch should be *scratch*");
    }

    #[test]
    fn buffer_name_returns_filename() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();
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
            "%hume-cmd-check-name", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "buffer-name should return the filename");
    }

    // ── (buffer-dirty? bid) ───────────────────────────────────────────────────

    #[test]
    fn buffer_dirty_false_for_clean_buffer() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

        h.eval_source(
            r#"(define-command! "check-dirty" ""
                 (lambda ()
                   (if (buffer-dirty? (current-buffer))
                       (call! "move-left")
                       (call! "move-right"))))"#,
            &mut s, &mut km,
        ).unwrap();

        let (queue, _) = h.call_steel_cmd(
            "%hume-cmd-check-dirty", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"], "new buffer should not be dirty");
    }

    // ── invalid buffer id ─────────────────────────────────────────────────────

    #[test]
    fn buffer_path_errors_on_invalid_id() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let (mut bufs, mut ev, mut ps, pane_id, buf_id) = one_buf_state();

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
            "%hume-cmd-check-invalid", None, None, &s,
            pane_id, buf_id, Some(&mut bufs), Some(&mut ev), Some(&mut ps),
        ).unwrap_err();
        assert!(err.contains("invalid buffer id"), "got: {err}");
    }
}
