use hume_engine::pipeline::BufferId;
use hume_platform::io::FileMeta;
use hume_scripting::host::OptionInput;

use super::super::Editor;
use super::super::Severity;
use super::super::before_save::{AfterWrite, SAVE_IN_PROGRESS};
use super::FocusedPane;
use crate::editor::buffer::Buffer;
use crate::editor::error::CommandError;
use crate::editor::settings::ops as settings_ops;
use crate::editor::settings::{WRAP_MODE_KEY, parse_setting_text};

const STALE_WRITE_MSG: &str = "file has changed on disk (add ! to override)";

/// Why a write command's write did not land.
pub(in crate::editor) enum WriteError {
    /// The buffer's own file changed on disk since it was read or written;
    /// `!` writes anyway.
    Stale,
    /// Not attempted: the buffer already has a write waiting on its
    /// handlers.
    InProgress,
    Failed(CommandError),
}

impl From<CommandError> for WriteError {
    fn from(e: CommandError) -> Self {
        Self::Failed(e)
    }
}

impl From<WriteError> for CommandError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::Stale => CommandError::transient(STALE_WRITE_MSG),
            WriteError::InProgress => CommandError::transient(SAVE_IN_PROGRESS),
            WriteError::Failed(e) => e,
        }
    }
}

/// Whether a non-forced write to `meta`'s file must be refused.
/// Stats the file fresh right now rather than trusting the buffer's cached
/// disk state, which only reflects whatever some earlier trigger (terminal
/// focus, buffer-enter, `:checktime`) happened to notice. Nothing runs a
/// check at write time otherwise, so a change made without any of those
/// firing would otherwise go undetected and get silently overwritten.
///
/// A vanished file is *not* blocked: `write_file_atomic` simply recreates
/// it, which is recovering the user's own work, not clobbering someone
/// else's: the same reasoning that lets `disk_change_for` treat any other
/// stat error as nothing-to-act-on-now.
fn changed_on_disk(meta: &FileMeta) -> bool {
    matches!(
        hume_platform::io::read_signature(meta.resolved_path()),
        Ok(sig) if sig != meta.signature()
    )
}

// ── Typed file commands ───────────────────────────────────────────────────────

pub(in crate::editor) fn typed_quit(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    // When the active tab has more than one pane, `:q` closes the focused
    // pane, not the tab or the editor. The buffer stays open (no edits lost),
    // so no dirty check. That guard belongs to the steps below, which
    // actually close something the user can't get back without saving. A
    // pipe run the pane started stops with it, as on any pane close.
    if super::close_focused_pane(&mut ed.state, &mut ed.view).is_ok() {
        return Ok(());
    }

    // The active tab is down to its last pane, but other tabs remain open:
    // `:q` closes the tab (Vim's `:q` on a window closes it; the last window
    // of a tab closes the tab, not the editor). Same no-dirty-check reasoning:
    // the buffer stays open in whichever other tab(s) show it, or in the
    // buffer list if none do.
    if super::close_tab(&mut ed.state, &mut ed.view).is_ok() {
        return Ok(());
    }

    let current = fp.bid(&ed.view);
    if !force && ed.state.has_unsaved_changes(current) {
        return Err(CommandError::transient(
            "Unsaved changes (add ! to override)",
        ));
    }

    // Stay only for a buffer worth returning to: a real editable file, or any
    // buffer with unsaved edits (rescues a scratch the user has typed into).
    // Empty scratch buffers and read-only views (e.g. [messages]) are disposable:
    // :q exits rather than parking on them.
    let stays = ed
        .state
        .buffers
        .iter()
        .filter(|(id, _)| *id != current)
        .any(|(id, buf)| {
            (buf.path().is_some() && !buf.is_read_only()) || ed.state.has_unsaved_changes(id)
        });
    // Exiting stops every pipe run; closing the buffer stops its own.
    let stopped_run = if stays {
        Some(current).filter(|&id| ed.state.buffers.get(id).is_busy())
    } else {
        ed.state.buffers.first_busy()
    };
    if !force && let Some(busy) = stopped_run {
        return Err(super::pipe_running_refusal(&ed.state, busy));
    }

    if stays {
        ed.close_buffer(current);
    } else {
        ed.state.should_quit = true;
    }
    Ok(())
}

pub(in crate::editor) fn typed_quit_all(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if !force {
        // Find the first dirty buffer in open-order.
        let dirty_id = ed
            .state
            .buffers
            .iter()
            .find(|(id, _)| ed.state.has_unsaved_changes(*id))
            .map(|(id, _)| id);

        if let Some(dirty_id) = dirty_id {
            // Jump to it only when the focused buffer is clean. If the user is
            // already sitting on an unsaved buffer, stay there so a save + :qa
            // cycle walks through dirty buffers one at a time.
            if !ed.state.has_unsaved_changes(fp.bid(&ed.view)) {
                ed.switch_to_buffer_with_jump(fp, dirty_id);
            }
            // `super::doc` reads the pane's buffer live: the dirty buffer if
            // the switch above ran, the already-dirty focused one otherwise.
            let name = super::doc(&ed.state, &ed.view, fp.pane()).display_name();
            // Stays Error, not transient: `EditorState::message_logged_this_input`
            // (lifecycle.rs) keys off `message_log.totals()` moving, and
            // `Editor::confirm_permit` (input_stack/confirm.rs) reads that flag to refuse a
            // disk-change reload confirm from popping over this exact message
            // right after :qa's focus-move names the dirty buffer; a
            // transient report wouldn't move totals() and would let the
            // confirm steal the next keystroke.
            return Err(CommandError::new(format!(
                "Unsaved changes in {name} (add ! to override)"
            )));
        }
        if let Some(busy) = ed.state.buffers.first_busy() {
            return Err(super::pipe_running_refusal(&ed.state, busy));
        }
    }
    ed.state.request_quit();
    Ok(())
}

pub(in crate::editor) fn typed_write(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    ed.request_write(fp.bid(&ed.view), fp, arg, force, AfterWrite::Nothing)
}

pub(in crate::editor) fn typed_write_quit(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    let quit = AfterWrite::Quit(fp.handle(&ed.view));
    ed.request_write(fp.bid(&ed.view), fp, arg, force, quit)
}

/// Writes `bid`, then carries out `after`. The commit step behind
/// `Editor::request_write`, run at once or when a parked write is released.
pub(in crate::editor) fn write_and_continue(
    ed: &mut Editor,
    bid: BufferId,
    kind: WriteKind<'_>,
    force: bool,
    after: AfterWrite,
) -> Result<(), CommandError> {
    let written = write_file(ed, bid, kind, force);
    conclude_write(ed, bid, written, force, after)
}

/// Carries out `after` for a write attempt that ended in `written`.
///
/// `force` applies to the quit too (proceed with the quit even if the write
/// fails). A quit whose pane is not the focused pane showing `bid` any more
/// (the write was parked while focus moved) is cancelled: the write still
/// happened. A `:wa` write is recorded in its batch, and a hard failure ends
/// the batch.
pub(in crate::editor) fn conclude_write(
    ed: &mut Editor,
    bid: BufferId,
    written: Result<(), WriteError>,
    force: bool,
    after: AfterWrite,
) -> Result<(), CommandError> {
    let handle = match after {
        AfterWrite::Nothing => return written.map_err(Into::into),
        AfterWrite::Batch(batch) => return ed.record_batch_write(batch, bid, written),
        AfterWrite::Quit(handle) => handle,
    };
    let Ok(pane) = FocusedPane::resolve(&ed.state, &ed.view, handle) else {
        if written.is_ok() {
            ed.report(
                Severity::Info,
                "Written; quit cancelled (focus moved)".to_owned(),
            );
        }
        return written.map_err(Into::into);
    };
    match written {
        Ok(()) => typed_quit(ed, pane, None, force),
        Err(WriteError::InProgress) => Err(WriteError::InProgress.into()),
        Err(e) if force => {
            typed_quit(ed, pane, None, true)
                .expect("force quit cannot fail: it skips the unsaved-changes and pipe checks");
            Err(e.into())
        }
        Err(e) => Err(e.into()),
    }
}

pub(in crate::editor) fn typed_toggle_soft_wrap(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let now_wrapping = ed.toggle_wrap(fp);
    let state = if now_wrapping { "on" } else { "off" };
    ed.report(Severity::Info, format!("Soft wrap {state}"));
    Ok(())
}

pub(in crate::editor) fn typed_set(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use crate::editor::settings::Scope;
    use hume_scripting::host::{BufferIntrinsicOption, language_option_value};

    const USAGE: &str = "Usage: :set global|buffer|pane key=value";
    let Some(arg) = arg else {
        return Err(CommandError::transient(USAGE));
    };
    let Some((scope_str, rest)) = arg.split_once(' ') else {
        return Err(CommandError::transient(USAGE));
    };
    // Tolerate stray extra whitespace before the key, matching
    // `complete_set`'s tolerance. Otherwise Tab can complete
    // through a double space into a command line that errors on Enter.
    let rest = rest.trim_start();
    let Some((key, value)) = rest.split_once('=') else {
        return Err(CommandError::transient("Expected key=value"));
    };
    let bid = fp.bid(&ed.view);

    // Buffer-intrinsic options live on the buffer, not in `EditorSettings`/
    // `BufferOverrides`, so they have no `scope:` entry in
    // `settings::setting_scopes`. Checked here first, or they would fall
    // through to "unknown setting" below.
    if let Some(opt) = BufferIntrinsicOption::from_key(key) {
        if scope_str.parse::<Scope>() != Ok(Scope::Buffer) {
            return Err(CommandError::transient(format!(
                "'{key}' is per-buffer: use ':set buffer {key}=<value>'"
            )));
        }
        return match opt {
            BufferIntrinsicOption::Language => {
                ed.set_buffer_language_by_name(bid, language_option_value(value));
                Ok(())
            }
            BufferIntrinsicOption::LineEnding => {
                settings_ops::set_line_ending(&mut ed.state, bid, value).map_err(CommandError::new)
            }
        };
    }

    // Every other setting declares its valid `:set` scopes on its
    // `define_settings!` line: one data-driven check instead of per-scope
    // special-casing.
    let scopes = crate::editor::settings::setting_scopes(key);
    if scopes.is_empty() {
        return Err(CommandError::transient(format!("unknown setting '{key}'")));
    }

    // Parse the scope token *after* confirming the key is real, so an
    // invalid scope on a real key reports "wrong scope for this key" (naming
    // the key's actual valid scopes) rather than a generic "unknown scope"
    // message: the user typed a real key, so that's the more useful error.
    let parsed_scope = scope_str.parse::<Scope>().ok();
    if !parsed_scope.is_some_and(|s| scopes.contains(&s)) {
        return Err(CommandError::transient(format!(
            "'{key}' cannot be set with :set {scope_str} (valid scopes: {})",
            scopes
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let scope = parsed_scope.expect("checked by is_some_and above");

    let input = OptionInput::Text(value.to_string());
    match scope {
        Scope::Global => settings_ops::apply_global(&mut ed.state, &mut ed.view, key, &input)
            .map_err(CommandError::new),
        Scope::Buffer => {
            settings_ops::apply_buffer(&mut ed.state, bid, key, &input).map_err(CommandError::new)
        }
        Scope::Pane => {
            // Only pane-scoped setting today: `scopes.contains(&scope)` above
            // already proved `key == "wrap-mode"` (it's the only line whose
            // `scope:` list contains `Scope::Pane`). A future pane-scoped
            // setting gets its own `if key == "..."` arm here, and
            // `every_pane_scoped_key_has_a_typed_set_arm`
            // (`settings/tests.rs`) fails immediately if one is added
            // without a matching arm.
            if key == WRAP_MODE_KEY {
                let mode = parse_setting_text::<hume_engine::pane::WrapMode>(WRAP_MODE_KEY, value)
                    .map_err(CommandError::new)?;
                ed.set_wrap_override(fp, mode);
                return Ok(());
            }
            unreachable!("'{key}' has scope Pane in setting_scopes() but no pane handler here")
        }
    }
}

/// Extract content and line count from a buffer by ID.
fn serialize_buffer(ed: &Editor, bid: BufferId) -> (String, hume_rope::line::ContentLineCount) {
    let buf = ed.state.buffers.get(bid);
    (buf.serialized(), buf.text().content_line_count())
}

/// Post-write side effects for a buffer that just had its own content
/// written to its own file: mark it saved, report the result, and sync LSP.
fn mark_written_and_synced(
    ed: &mut Editor,
    bid: BufferId,
    line_count: hume_rope::line::ContentLineCount,
    retried: bool,
) {
    ed.state.buffers.get_mut(bid).mark_saved();
    ed.report(write_severity(retried), write_msg(line_count, retried));
    ed.queue_buffer_save(bid);
    // Flush any didChange already queued for this buffer first: a
    // save-triggered server action (e.g. lint-on-save) must see a
    // document state at least as current as the file just written,
    // not one edit behind (didSave itself carries no text).
    ed.state.lsp_flush_pending();
    ed.state.lsp_did_save(bid);
    ed.state.lsp_repull_diagnostics_all(bid);
}

impl Editor {
    /// Gives `bid` the path a write resolved to. A buffer whose path
    /// changes is a different document to its language servers: the
    /// reconcile closes it under the old URI and reopens it under the new
    /// one, keeping each instance, so `didSave` afterwards names a document
    /// every attached server has open.
    fn rekey_buffer_path(
        &mut self,
        bid: BufferId,
        meta: hume_platform::io::FileMeta,
        typed_path: Option<std::path::PathBuf>,
    ) {
        let changed = self.state.buffers.get(bid).path() != Some(meta.resolved_path());
        let buf = self.state.buffers.get_mut(bid);
        buf.set_path(Some(meta.resolved_path().to_path_buf()));
        buf.set_typed_path(typed_path);
        buf.file_meta = Some(meta);
        if changed {
            self.state.lsp_reconcile_buffer(&self.view, bid);
        }
    }
}

/// Write a specific buffer to its file path. No save-as: only writes to the
/// buffer's own path. The buffer is a [`WriteKind::Save`]: writable, with a
/// path.
///
/// `file_meta` (not `path`) is the SSOT for "has this buffer ever touched
/// disk" (see `Buffer::is_new_file`): a buffer with a `path` but no
/// `file_meta` is a `Buffer::new_file` (`:e` on a path that didn't exist yet)
/// and takes the create branch below instead of `write_file_atomic`, which
/// needs an existing baseline to atomically replace.
fn write_buffer_by_id(
    ed: &mut Editor,
    bid: BufferId,
    content: String,
    line_count: hume_rope::line::ContentLineCount,
    force: bool,
) -> Result<(), WriteError> {
    let buf = ed.state.buffers.get_mut(bid);
    if buf.file_meta.is_none() {
        let path = buf
            .path()
            .map(std::path::Path::to_path_buf)
            .expect("a Save has a path");
        // Stat fresh rather than assuming the path is still missing: a
        // new-file buffer carries no baseline to compare against
        // (`changed_on_disk` needs a `FileMeta`, which this buffer has
        // none of), so a file that appeared since `:e` would otherwise be
        // silently clobbered by `write_file_new`. Same
        // stat-at-write-time reasoning as `changed_on_disk`, just
        // discovered via existence instead of a signature mismatch.
        let write_result = match hume_platform::io::read_file_meta(&path) {
            Ok(_) if !force => return Err(WriteError::Stale),
            // Forced: write through `write_file_atomic`, not
            // `write_file_new`: the file now exists, so its
            // permissions/ownership/symlink target should be preserved
            // like any other overwrite, not replaced with new-file defaults.
            Ok(mut meta) => hume_platform::io::write_file_atomic(&content, &mut meta, force)
                .map(|retried| (meta, retried)),
            Err(_) => hume_platform::io::write_file_new(&content, &path).map(|meta| (meta, false)),
        };
        return match write_result {
            Ok((meta, retried)) => {
                // Re-key: `path` was `resolve_buffer_path`'s best-effort form
                // (parent canonicalized, or fully lexical when even the
                // parent didn't exist yet). `meta.resolved_path()` is the
                // fully resolved post-write truth, which can differ, and
                // must become the buffer's identity so `find_by_path` dedup
                // keeps working. The typed path is preserved across the
                // `set_path` re-derivation, matching the save-as branch below.
                let typed_path = ed
                    .state
                    .buffers
                    .get(bid)
                    .typed_path()
                    .map(std::path::Path::to_path_buf);
                ed.rekey_buffer_path(bid, meta, typed_path);
                mark_written_and_synced(ed, bid, line_count, retried);
                Ok(())
            }
            Err(e) => Err(CommandError::new(e.to_string()).into()),
        };
    }
    let meta = buf
        .file_meta
        .as_mut()
        .expect("checked file_meta.is_none() above");
    if !force && changed_on_disk(meta) {
        return Err(WriteError::Stale);
    }
    match hume_platform::io::write_file_atomic(&content, meta, force) {
        Ok(retried) => {
            mark_written_and_synced(ed, bid, line_count, retried);
            Ok(())
        }
        Err(e) => Err(CommandError::new(e.to_string()).into()),
    }
}

/// What a write command does with a buffer.
#[derive(Clone, Copy)]
pub(in crate::editor) enum WriteKind<'a> {
    /// `:w`: the buffer's own file. A new-file buffer (a path, no
    /// `file_meta`) is created. A read-only buffer or one with no path is
    /// `Refused` instead.
    Save,
    /// `:w <path>` on a writable buffer: it becomes the file at the path.
    SaveAs(&'a str),
    /// `:w <path>` on a read-only or synthetic buffer (e.g. `[messages]`): the
    /// content is written to the path, but the buffer's path, `file_meta` and
    /// dirty state are left alone, since it did not become that file.
    Export(&'a str),
    /// No write happens; the message says why.
    Refused(&'static str),
}

impl<'a> WriteKind<'a> {
    pub(in crate::editor) fn classify(buf: &Buffer, arg: Option<&'a str>) -> Self {
        match arg {
            Some(path) if buf.is_read_only() || buf.is_synthetic() => Self::Export(path),
            Some(path) => Self::SaveAs(path),
            None if buf.is_read_only() => Self::Refused("Buffer is read-only"),
            None if buf.path().is_none() => Self::Refused("no file name"),
            None => Self::Save,
        }
    }

    /// Whether the write lands in the buffer's own file, once it is written.
    pub(in crate::editor) fn reaches_own_file(self) -> bool {
        matches!(self, Self::Save | Self::SaveAs(_))
    }
}

/// Serialize the buffer and write it to disk.
///
/// What the write does is `kind`, from [`WriteKind::classify`]: a save-as updates
/// the buffer's path and `file_meta` so that a later `:w` targets the same
/// path, an export leaves the buffer untouched, and a refusal is the error.
///
/// When `force` is `true`, a `PermissionDenied` rename error triggers a
/// chmod-retry: the target is made writable, the rename is retried, and the
/// status message includes "(forced)".
///
/// On success (save-as case), marks `bid` saved and sets a status message.
/// Returns `Ok(())` on success, `Err(CommandError)` on any error.
fn write_file(
    ed: &mut Editor,
    bid: BufferId,
    kind: WriteKind<'_>,
    force: bool,
) -> Result<(), WriteError> {
    if let WriteKind::Refused(msg) = kind {
        return Err(CommandError::transient(msg).into());
    }
    let (content, line_count) = serialize_buffer(ed, bid);

    if let WriteKind::SaveAs(path_str) | WriteKind::Export(path_str) = kind {
        let expanded = hume_platform::path::expand(path_str);
        let path: std::path::PathBuf = {
            let p = std::path::Path::new(expanded.as_ref());
            // Resolve relative paths against editor.cwd, not the process cwd,
            // so `:w relpath` is stable regardless of how the process cwd drifts.
            if p.is_relative() {
                ed.state.cwd.join(p)
            } else {
                p.to_owned()
            }
        };
        // The user-typed path (not the canonicalized one), recorded for
        // user-facing display. `normalize_lexical` is used here (rather than
        // `absolute_unresolved`) because `path` was already made absolute
        // above (via cwd.join or identity).
        let typed_path = hume_platform::path::normalize_lexical(&path);
        // Try to preserve existing file's permissions; if the file doesn't
        // exist yet, write_file_new creates it with default permissions.
        let result = match hume_platform::io::read_file_meta(&path) {
            Ok(mut meta) => {
                // The stale-write guard only applies when `path` resolves to
                // the buffer's own current file, i.e. this `:w <path>` is
                // really a plain `:w` in disguise. A genuine save-as targets
                // a path this buffer never read from, so there's no staleness
                // to guard against. `meta` was just freshly stat'd above by
                // `read_file_meta`, so comparing its signature against the
                // buffer's own baseline is already a stat-at-write-time
                // check: no cached flag, no second syscall needed.
                let own = ed.state.buffers.get(bid);
                let targets_own_file = own.path() == Some(meta.resolved_path());
                // A new-file buffer (`file_meta: None`) has no baseline at
                // all. If this write targets its own path and a file now
                // exists there, that content was never read by this buffer,
                // so it counts as "differs" the same as a genuine signature
                // mismatch would.
                let own_baseline_differs = match own.file_meta.as_ref() {
                    Some(own) => own.signature() != meta.signature(),
                    None => true,
                };
                if targets_own_file && !force && own_baseline_differs {
                    return Err(WriteError::Stale);
                }
                hume_platform::io::write_file_atomic(&content, &mut meta, force)
                    .map(|retried| (meta, retried))
            }
            Err(_) => hume_platform::io::write_file_new(&content, &path).map(|meta| (meta, false)),
        };
        match result {
            Ok((meta, retried)) => {
                if matches!(kind, WriteKind::SaveAs(_)) {
                    // Store the canonicalized path so path and
                    // file_meta.resolved_path always agree, even when the
                    // user supplied a relative or symlink path.
                    ed.rekey_buffer_path(bid, meta, Some(typed_path));
                    mark_written_and_synced(ed, bid, line_count, retried);
                } else {
                    ed.report(write_severity(retried), write_msg(line_count, retried));
                }
                Ok(())
            }
            Err(e) => Err(CommandError::new(e.to_string()).into()),
        }
    } else {
        write_buffer_by_id(ed, bid, content, line_count, force)
    }
}

pub(in crate::editor) fn typed_write_all(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    let dirty_savable: Vec<BufferId> = ed
        .state
        .buffers
        .iter()
        // A buffer `:w` would refuse is skipped: erroring would abort the
        // batch after partial saves.
        .filter(|(id, buf)| {
            ed.state.has_unsaved_changes(*id)
                && matches!(WriteKind::classify(buf, None), WriteKind::Save)
        })
        .map(|(id, _)| id)
        .collect();

    if dirty_savable.is_empty() {
        return Ok(());
    }

    // The writes form one batch, reported once every member has finished,
    // which may be later than this command when handlers hold some. A buffer
    // whose file changed on disk, or whose write is already waiting, is
    // skipped: one stale buffer among several dirty ones shouldn't block
    // saving the rest. `force` (`:wa!`) writes through the stale ones
    // instead, same as a per-buffer `:w!`. Any other write failure ends the
    // batch.
    let batch = ed.state.pending_writes.open_batch();
    for bid in dirty_savable {
        ed.request_write(bid, fp, None, force, AfterWrite::Batch(batch))?;
    }
    ed.state.pending_writes.seal(batch);
    ed.report_finished_write_batches();
    Ok(())
}

fn write_severity(forced: bool) -> Severity {
    if forced {
        Severity::Warning
    } else {
        Severity::Info
    }
}

fn write_msg(line_count: hume_rope::line::ContentLineCount, forced: bool) -> String {
    let line_count = line_count.get();
    if forced {
        format!("Written {line_count} lines (forced)")
    } else {
        format!("Written {line_count} lines")
    }
}
