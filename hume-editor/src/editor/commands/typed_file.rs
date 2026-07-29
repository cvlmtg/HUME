use hume_engine::pipeline::BufferId;
use hume_platform::io::FileMeta;

use super::super::Editor;
use super::super::Severity;
use crate::editor::error::CommandError;
use crate::editor::settings_ops;

/// Shared by every stale-write refusal — `write_buffer_by_id`'s no-arg `:w`
/// path and `write_file`'s save-as-in-disguise path (see `targets_own_file`
/// below) both report exactly this, so `typed_write_all` can tell a stale
/// refusal apart from any other write failure by comparing against it.
const STALE_WRITE_MSG: &str = "file has changed on disk (add ! to override)";

/// `Some(msg)` when a non-forced write to `meta`'s file must be refused.
/// Stats the file fresh right now rather than trusting the buffer's cached
/// disk state, which only reflects whatever some earlier trigger (terminal
/// focus, buffer-enter, `:checktime`) happened to notice — nothing runs a
/// check at write time otherwise, so a change made without any of those
/// firing would otherwise go undetected and get silently overwritten.
///
/// A vanished file is *not* blocked: `write_file_atomic` simply recreates
/// it, which is recovering the user's own work, not clobbering someone
/// else's — the same reasoning that lets `disk_change_for` treat any other
/// stat error as nothing-to-act-on-now.
fn stale_write_block(meta: &FileMeta) -> Option<&'static str> {
    match hume_platform::io::read_signature(meta.resolved_path()) {
        Ok(sig) if sig != meta.signature() => Some(STALE_WRITE_MSG),
        _ => None,
    }
}

// ── Typed file commands ───────────────────────────────────────────────────────

pub fn typed_quit(ed: &mut Editor, _arg: Option<&str>, force: bool) -> Result<(), CommandError> {
    // Multiple panes open: `:q` closes the focused pane, not the editor. The
    // buffer stays open in the buffer list (no edits lost), so no dirty check —
    // that guard belongs to the single-pane path below, which actually quits.
    if ed.view.panes.len() > 1 {
        super::close_focused_pane(&mut ed.state, &mut ed.view);
        return Ok(());
    }

    if !force && ed.doc().is_dirty() {
        return Err(CommandError::new("Unsaved changes (add ! to override)"));
    }

    let current = ed.focused_buffer_id();
    // Stay only for a buffer worth returning to: a real editable file, or any
    // buffer with unsaved edits (rescues a scratch the user has typed into).
    // Empty scratch buffers and read-only views (e.g. [messages]) are disposable —
    // :q exits rather than parking on them.
    let any_other_real = ed
        .state
        .buffers
        .iter()
        .filter(|(id, _)| *id != current)
        .any(|(_, buf)| (buf.path().is_some() && !buf.is_read_only()) || buf.is_dirty());

    if !any_other_real {
        ed.state.should_quit = true;
    } else {
        ed.close_buffer(current);
    }
    Ok(())
}

pub fn typed_quit_all(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if !force {
        // Find the first dirty buffer in open-order.
        let dirty_id = ed
            .state
            .buffers
            .iter()
            .find(|(_, buf)| buf.is_dirty())
            .map(|(id, _)| id);

        if let Some(dirty_id) = dirty_id {
            // Jump to it only when the focused buffer is clean — if the user is
            // already sitting on an unsaved buffer, stay there so a save + :qa
            // cycle walks through dirty buffers one at a time.
            if !ed.doc().is_dirty() {
                ed.switch_to_buffer_with_jump(dirty_id);
            }
            let name = ed.state.buffers.get(ed.focused_buffer_id()).display_name();
            return Err(CommandError::new(format!(
                "Unsaved changes in {name} (add ! to override)"
            )));
        }
    }
    ed.state.request_quit();
    Ok(())
}

pub fn typed_write(ed: &mut Editor, arg: Option<&str>, force: bool) -> Result<(), CommandError> {
    write_file(ed, arg, force)
}

pub fn typed_write_quit(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    // force applies to both write (chmod-retry on readonly targets) and quit
    // (proceed with the quit even if the write fails). After a successful
    // write, delegate to typed_quit so :wq mirrors :q's pane/buffer-aware
    // close instead of always tearing down the whole editor.
    match write_file(ed, arg, force) {
        Ok(()) => typed_quit(ed, None, force),
        Err(e) if force => {
            typed_quit(ed, None, true).expect("force quit cannot fail: dirty check is skipped");
            Err(e)
        }
        Err(e) => Err(e),
    }
}

pub fn typed_toggle_soft_wrap(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use hume_engine::pane::WrapMode;
    let currently_wrapping = ed.focused_wrap_mode().is_wrapping();
    let target = if currently_wrapping {
        WrapMode::None
    } else {
        // saved_wrap_mode is always a wrapping variant — restores whatever
        // mode this pane last wrapped with (its `:set pane` value, or the
        // global seed), instead of hardcoding one.
        ed.view.panes[ed.state.focused_pane_id].saved_wrap_mode
    };
    ed.apply_focused_wrap_mode(target);
    let state = if currently_wrapping { "off" } else { "on" };
    ed.report(Severity::Info, format!("Soft wrap {state}"));
    Ok(())
}

pub fn typed_set(ed: &mut Editor, arg: Option<&str>, _force: bool) -> Result<(), CommandError> {
    use crate::settings::{LANGUAGE_KEY, Scope};

    const USAGE: &str = "Usage: :set global|buffer|pane key=value";
    let Some(arg) = arg else {
        return Err(CommandError::new(USAGE));
    };
    let Some((scope_str, rest)) = arg.split_once(' ') else {
        return Err(CommandError::new(USAGE));
    };
    // Tolerate stray extra whitespace before the key, matching the
    // `SetCompleter`'s tolerance (a6e5adc) — otherwise Tab can complete
    // through a double space into a command line that errors on Enter.
    let rest = rest.trim_start();
    let Some((key, value)) = rest.split_once('=') else {
        return Err(CommandError::new("Expected key=value"));
    };
    let bid = ed.focused_buffer_id();

    // `language` has no global default and no generic storage (it lives on
    // `Buffer.language`, not `EditorSettings`/`BufferOverrides`), so it has no
    // `scope:` entry in `settings::setting_scopes` — checked here first and
    // unconditionally, or it would fall through to "unknown setting" below.
    if key == LANGUAGE_KEY {
        return match scope_str {
            "buffer" => {
                let new_lang = if value.is_empty() {
                    None
                } else {
                    if ed.state.config.languages.by_name(value).is_none() {
                        ed.report(
                            Severity::Warning,
                            format!("language '{value}' is not registered"),
                        );
                    }
                    Some(ed.state.config.languages.intern(value))
                };
                ed.set_buffer_language_explicit(bid, new_lang);
                Ok(())
            }
            _ => Err(CommandError::new(
                "'language' is per-buffer — use ':set buffer language=<name>'",
            )),
        };
    }

    // Every other setting declares its valid `:set` scopes on its
    // `define_settings!` line — one data-driven check instead of per-scope
    // special-casing.
    let scopes = crate::settings::setting_scopes(key);
    if scopes.is_empty() {
        return Err(CommandError::new(format!("unknown setting '{key}'")));
    }

    // Parse the scope token *after* confirming the key is real, so an
    // invalid scope on a real key reports "wrong scope for this key" (naming
    // the key's actual valid scopes) rather than a generic "unknown scope"
    // message — the user typed a real key, so that's the more useful error.
    let parsed_scope = scope_str.parse::<Scope>().ok();
    if !parsed_scope.is_some_and(|s| scopes.contains(&s)) {
        return Err(CommandError::new(format!(
            "'{key}' cannot be set with :set {scope_str} — valid scopes: {}",
            scopes
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let scope = parsed_scope.expect("checked by is_some_and above");

    match scope {
        Scope::Global => settings_ops::apply_global(&mut ed.state, &mut ed.view, key, value)
            .map_err(CommandError::new),
        Scope::Buffer => {
            settings_ops::apply_buffer(&mut ed.state, bid, key, value).map_err(CommandError::new)
        }
        Scope::Pane => {
            // Only pane-scoped setting today — `scopes.contains(&scope)` above
            // already proved `key == "wrap-mode"` (it's the only line whose
            // `scope:` list contains `Scope::Pane`). A future pane-scoped
            // setting gets its own `if key == "..."` arm here — and
            // `every_pane_scoped_key_has_a_typed_set_arm`
            // (`settings/tests.rs`) fails immediately if one is added
            // without a matching arm.
            if key == "wrap-mode" {
                use std::str::FromStr;
                let mode =
                    hume_engine::pane::WrapMode::from_str(value).map_err(CommandError::new)?;
                ed.apply_focused_wrap_mode(mode);
                return Ok(());
            }
            unreachable!("'{key}' has scope Pane in setting_scopes() but no pane handler here")
        }
    }
}

/// Extract content and line count from a buffer by ID.
fn serialize_buffer(ed: &Editor, bid: BufferId) -> (String, usize) {
    let buf = ed.state.buffers.get(bid);
    let text = buf.text();
    let content = if text.line_ending() == hume_editing::text::LineEnding::CrLf {
        text.to_string().replace('\n', "\r\n")
    } else {
        text.to_string()
    };
    let line_count = text.len_lines().saturating_sub(1);
    (content, line_count)
}

/// Post-write side effects for a buffer that just had its own content
/// written to its own file: mark it saved, report the result, and sync LSP.
/// Shared by the no-arg `:w` path and the save-as path (when the source
/// buffer is a normal writable buffer, i.e. save-as, not export — see
/// `write_file`'s save-as branch).
fn mark_written_and_synced(ed: &mut Editor, bid: BufferId, line_count: usize, retried: bool) {
    ed.state.buffers.get_mut(bid).mark_saved();
    ed.report(write_severity(retried), write_msg(line_count, retried));
    ed.fire_hook_buffer_save(bid);
    // Flush any didChange already queued for this buffer first — a
    // save-triggered server action (e.g. lint-on-save) must see a
    // document state at least as current as the file just written,
    // not one edit behind (didSave itself carries no text).
    ed.flush_lsp_pending_changes();
    ed.lsp_did_save(bid);
}

/// Write a specific buffer to its file path. No save-as — only writes to the
/// buffer's own `file_meta` path. Used by `:wa` and the no-arg path of `:w`.
fn write_buffer_by_id(
    ed: &mut Editor,
    bid: BufferId,
    content: String,
    line_count: usize,
    force: bool,
) -> Result<(), CommandError> {
    let buf = ed.state.buffers.get_mut(bid);
    if buf.is_read_only() {
        return Err(CommandError::new("Buffer is read-only"));
    }
    let Some(meta) = buf.file_meta.as_mut() else {
        return Err(CommandError::new("no file name"));
    };
    if !force && let Some(msg) = stale_write_block(meta) {
        return Err(CommandError::new(msg));
    }
    match hume_platform::io::write_file_atomic(&content, meta, force) {
        Ok(retried) => {
            mark_written_and_synced(ed, bid, line_count, retried);
            Ok(())
        }
        Err(e) => Err(CommandError::new(e.to_string())),
    }
}

/// Serialize the buffer and write it to disk.
///
/// If `arg` is `Some(path)`, performs a save-as for a normal writable buffer:
/// writes to the specified path and updates `ed.file_path` / `ed.file_meta`
/// so that subsequent `:w` (no argument) targets the same path. For a
/// read-only or synthetic buffer (e.g. `[messages]`), `arg` is instead an
/// **export**: the content is written to the new path, but the source
/// buffer's path/`file_meta`/dirty state are left untouched — it did not
/// become the file at `path`.
///
/// If `arg` is `None`, writes to the current file. Errors with
/// "Buffer is read-only" if the focused buffer has `read_only = true`, or
/// "no file name" if the buffer is a scratch buffer with no path.
///
/// When `force` is `true`, a `PermissionDenied` rename error triggers a
/// chmod-retry: the target is made writable, the rename is retried, and the
/// status message includes "(forced)".
///
/// On success (save-as case), calls `ed.doc_mut().mark_saved()` and sets a
/// status message. Returns `Ok(())` on success, `Err(CommandError)` on any
/// error.
fn write_file(ed: &mut Editor, arg: Option<&str>, force: bool) -> Result<(), CommandError> {
    let (content, line_count) = serialize_buffer(ed, ed.focused_buffer_id());

    if let Some(path_str) = arg {
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
        // Lexically-normalized absolute path without symlink resolution, recorded for
        // the FilePath statusline element so it shows the user-typed path, not the
        // canonicalized one.  `normalize_lexical` is used here because `path` was
        // already made absolute above (via cwd.join or identity).
        let display_path = hume_platform::path::normalize_lexical(&path);
        // Try to preserve existing file's permissions; if the file doesn't
        // exist yet, write_file_new creates it with default permissions.
        let result = match hume_platform::io::read_file_meta(&path) {
            Ok(mut meta) => {
                // The stale-write guard only applies when `path` resolves to
                // the buffer's own current file — i.e. this `:w <path>` is
                // really a plain `:w` in disguise. A genuine save-as targets
                // a path this buffer never read from, so there's no staleness
                // to guard against. `meta` was just freshly stat'd above by
                // `read_file_meta`, so comparing its signature against the
                // buffer's own baseline is already a stat-at-write-time
                // check — no cached flag, no second syscall needed.
                let targets_own_file = ed.doc().path() == Some(meta.resolved_path());
                let own_baseline_differs = ed
                    .doc()
                    .file_meta
                    .as_ref()
                    .is_some_and(|own| own.signature() != meta.signature());
                if targets_own_file && !force && own_baseline_differs {
                    return Err(CommandError::new(STALE_WRITE_MSG));
                }
                hume_platform::io::write_file_atomic(&content, &mut meta, force)
                    .map(|retried| (meta, retried))
            }
            Err(_) => hume_platform::io::write_file_new(&content, &path).map(|meta| (meta, false)),
        };
        match result {
            Ok((meta, retried)) => {
                let bid = ed.focused_buffer_id();
                // A read-only or synthetic (e.g. [messages]) buffer can't
                // legitimately become the file at `path` — :w <path> on one
                // of these is an export, not a save-as: dump the content,
                // leave the source buffer's identity and dirty state alone.
                let is_save_as = !ed.doc().is_synthetic() && !ed.doc().is_read_only();
                if is_save_as {
                    // Store the canonicalized path so path and
                    // file_meta.resolved_path always agree, even when the
                    // user supplied a relative or symlink path.
                    ed.doc_mut()
                        .set_path(Some(meta.resolved_path().to_path_buf()));
                    ed.doc_mut().set_display_path(Some(display_path));
                    ed.doc_mut().file_meta = Some(meta);
                    mark_written_and_synced(ed, bid, line_count, retried);
                } else {
                    ed.report(write_severity(retried), write_msg(line_count, retried));
                }
                Ok(())
            }
            Err(e) => Err(CommandError::new(e.to_string())),
        }
    } else {
        write_buffer_by_id(ed, ed.focused_buffer_id(), content, line_count, force)
    }
}

pub fn typed_write_all(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    let dirty_savable: Vec<BufferId> = ed
        .state
        .buffers
        .iter()
        // Skip read-only buffers; write_buffer_by_id would error and abort the
        // whole batch after partial saves. A read-only dirty buffer is unusual
        // (only set-text can do it), but handle it gracefully.
        .filter(|(_, buf)| buf.is_dirty() && buf.file_meta.is_some() && !buf.is_read_only())
        .map(|(id, _)| id)
        .collect();

    if dirty_savable.is_empty() {
        return Ok(());
    }

    // A buffer whose file changed on disk is skipped, not aborted — one
    // stale buffer among several dirty ones shouldn't block saving the rest.
    // `force` (`:wa!`) writes through every one of them instead, same as a
    // per-buffer `:w!`. `write_buffer_by_id` is the single chokepoint for the
    // stale check (no separate pre-check here, which would stat every buffer
    // twice) — a stale refusal is recognized by its message and downgraded to
    // a skip; any other write error still aborts the batch.
    let mut count = 0;
    let mut skipped: Vec<String> = Vec::new();
    for bid in dirty_savable {
        let (content, line_count) = serialize_buffer(ed, bid);
        match write_buffer_by_id(ed, bid, content, line_count, force) {
            Ok(()) => count += 1,
            Err(e) if e.message() == STALE_WRITE_MSG => {
                skipped.push(ed.state.buffers.get(bid).display_name());
            }
            Err(e) => return Err(e),
        }
    }
    if count > 0 {
        ed.report(Severity::Info, format!("Written {count} file(s)"));
    }
    if !skipped.is_empty() {
        ed.report(
            Severity::Warning,
            format!("Skipped (changed on disk): {}", skipped.join(", ")),
        );
    }
    Ok(())
}

fn write_severity(forced: bool) -> Severity {
    if forced {
        Severity::Warning
    } else {
        Severity::Info
    }
}

fn write_msg(line_count: usize, forced: bool) -> String {
    if forced {
        format!("Written {line_count} lines (forced)")
    } else {
        format!("Written {line_count} lines")
    }
}
