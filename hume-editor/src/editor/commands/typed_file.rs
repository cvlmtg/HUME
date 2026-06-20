use hume_engine::pipeline::BufferId;

use super::super::{ops, Severity};
use super::super::Editor;
use crate::editor::error::CommandError;

// ── Typed file commands ───────────────────────────────────────────────────────

pub fn typed_quit(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if !force && ed.doc().is_dirty() {
        return Err(CommandError::new("Unsaved changes (add ! to override)"));
    }

    let current = ed.focused_buffer_id();
    // Stay only for a buffer worth returning to: a real editable file, or any
    // buffer with unsaved edits (rescues a scratch the user has typed into).
    // Empty scratch buffers and read-only views (e.g. [messages]) are disposable —
    // :q exits rather than parking on them.
    let any_other_real = ed
        .state.buffers
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
        let dirty_id = ed.state.buffers.iter()
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
    ed.state.should_quit = true;
    Ok(())
}

pub fn typed_write(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    write_file(ed, arg, force)
}

pub fn typed_write_quit(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    // force applies to both write (chmod-retry on readonly targets) and quit
    // (quit even if the write fails).
    match write_file(ed, arg, force) {
        Ok(()) => {
            ed.state.should_quit = true;
            Ok(())
        }
        Err(e) if force => {
            ed.state.should_quit = true;
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
    let currently_wrapping = ed.doc().overrides.wrap_mode(&ed.state.settings).is_wrapping();
    if currently_wrapping {
        ed.doc_mut().overrides.wrap_mode = Some(WrapMode::None);
        // Horizontal offset is now meaningful; scroll stays where it is.
    } else {
        // width: 0 is the sentinel for "content width" — resolved via
        // WrapMode::resolve(content_width) at render time, so this reflows on resize.
        ed.doc_mut().overrides.wrap_mode = Some(WrapMode::Indent { width: 0 });
        ed.viewport_mut().horizontal_offset = 0;
        ed.viewport_mut().top_row_offset = 0;
    }
    let state = if currently_wrapping { "off" } else { "on" };
    ed.report(Severity::Info, format!("Soft wrap {state}"));
    Ok(())
}

pub fn typed_set(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    const USAGE: &str = "Usage: :set global|buffer key=value";
    let Some(arg) = arg else {
        return Err(CommandError::new(USAGE));
    };
    let Some((scope, rest)) = arg.split_once(' ') else {
        return Err(CommandError::new(USAGE));
    };
    let Some((key, value)) = rest.split_once('=') else {
        return Err(CommandError::new("Expected key=value"));
    };
    let bid = ed.focused_buffer_id();

    // Language is a per-buffer property, not a generic setting.
    if key == "language" {
        return match scope {
            "global" => Err(CommandError::new(
                "'language' is per-buffer — use ':set buffer language=<name>'"
            )),
            "buffer" => {
                let new_lang = if value.is_empty() { None } else { Some(value.to_owned()) };
                if let Some(ref name) = new_lang
                    && ed.state.languages.by_name(name).is_none()
                {
                    ed.report(
                        Severity::Warning,
                        format!("language '{name}' is not registered"),
                    );
                }
                ed.set_buffer_language(bid, new_lang);
                Ok(())
            }
            _ => Err(CommandError::new(format!(
                "unknown scope '{scope}': expected 'global' or 'buffer'"
            ))),
        };
    }

    let result = match scope {
        "global" => crate::settings::apply_setting(
            crate::settings::SettingScope::Global,
            key,
            value,
            &mut ed.state.settings,
            &mut ed.state.buffers.get_mut(bid).overrides,
        ),
        "buffer" => crate::settings::apply_setting(
            crate::settings::SettingScope::Text,
            key,
            value,
            &mut ed.state.settings,
            &mut ed.state.buffers.get_mut(bid).overrides,
        ),
        _ => Err(format!(
            "unknown scope '{scope}': expected 'global' or 'buffer'"
        )),
    };
    if result.is_ok() && key == "history-capacity" {
        ed.state.history.set_capacity(ed.state.settings.history_capacity);
    }
    if result.is_ok() && key == "theme" && scope == "global" && !ed.state.settings.theme.is_empty() {
        ops::load_theme_by_name(
            &mut ed.view,
            &mut ed.state.message_log,
            &mut ed.state.status_msg,
            &ed.state.settings.theme,
        );
    }
    result.map_err(CommandError::new)
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

/// Write a specific buffer to its file path. No save-as — only writes to the
/// buffer's own `file_meta` path. Used by `:wa` and the no-arg path of `:w`.
fn write_buffer_by_id(
    ed: &mut Editor,
    bid: BufferId,
    content: String,
    line_count: usize,
    force: bool,
) -> Result<(), CommandError> {
    let buf = ed.state.buffers.get(bid);
    if buf.is_read_only() {
        return Err(CommandError::new("Buffer is read-only"));
    }
    let Some(meta) = buf.file_meta.as_ref() else {
        return Err(CommandError::new("no file name"));
    };
    let retried = hume_platform::io::write_file_atomic(&content, meta, force);
    match retried {
        Ok(retried) => {
            ed.state.buffers.get_mut(bid).mark_saved();
            ed.report(write_severity(retried), write_msg(line_count, retried));
            ed.fire_hook_buffer_save(bid);
            Ok(())
        }
        Err(e) => Err(CommandError::new(e.to_string())),
    }
}

/// Serialize the buffer and write it to disk.
///
/// If `arg` is `Some(path)`, performs a save-as: writes to the specified
/// path and updates `ed.file_path` / `ed.file_meta` so that subsequent
/// `:w` (no argument) targets the same path.
///
/// If `arg` is `None`, writes to the current file. Errors with
/// "Buffer is read-only" if the focused buffer has `read_only = true`, or
/// "no file name" if the buffer is a scratch buffer with no path.
///
/// When `force` is `true`, a `PermissionDenied` rename error triggers a
/// chmod-retry: the target is made writable, the rename is retried, and the
/// status message includes "(forced)".
///
/// On success, calls `ed.doc_mut().mark_saved()` and sets a status message.
/// Returns `Ok(())` on success, `Err(CommandError)` on any error.
fn write_file(ed: &mut Editor, arg: Option<&str>, force: bool) -> Result<(), CommandError> {
    let (content, line_count) = serialize_buffer(ed, ed.focused_buffer_id());

    if let Some(path_str) = arg {
        let expanded = hume_platform::path::expand(path_str);
        let path: std::path::PathBuf = {
            let p = std::path::Path::new(expanded.as_ref());
            // Resolve relative paths against editor.cwd, not the process cwd,
            // so `:w relpath` is stable regardless of how the process cwd drifts.
            if p.is_relative() { ed.state.cwd.join(p) } else { p.to_owned() }
        };
        // Lexically-normalized absolute path without symlink resolution, recorded for
        // the FilePath statusline element so it shows the user-typed path, not the
        // canonicalized one.  `normalize_lexical` is used here because `path` was
        // already made absolute above (via cwd.join or identity).
        let display_path = hume_platform::path::normalize_lexical(&path);
        // Try to preserve existing file's permissions; if the file doesn't
        // exist yet, write_file_new creates it with default permissions.
        let result = match hume_platform::io::read_file_meta(&path) {
            Ok(meta) => hume_platform::io::write_file_atomic(&content, &meta, force)
                .map(|retried| (meta, retried)),
            Err(_) => hume_platform::io::write_file_new(&content, &path).map(|meta| (meta, false)),
        };
        match result {
            Ok((meta, retried)) => {
                // Store the canonicalized path so path and file_meta.resolved_path
                // always agree, even when the user supplied a relative or symlink path.
                // Synthetic buffers (e.g. [messages]) stay path-less after save-as —
                // the write dumps content to disk but the buffer itself is unaffected.
                if !ed.doc().is_synthetic() {
                    ed.doc_mut().set_path(Some(meta.resolved_path().to_path_buf()));
                    ed.doc_mut().set_display_path(Some(display_path));
                    ed.doc_mut().file_meta = Some(meta);
                }
                ed.doc_mut().mark_saved();
                ed.report(write_severity(retried), write_msg(line_count, retried));
                ed.fire_hook_buffer_save(ed.focused_buffer_id());
                Ok(())
            }
            Err(e) => Err(CommandError::new(e.to_string())),
        }
    } else {
        return write_buffer_by_id(ed, ed.focused_buffer_id(), content, line_count, force);
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
        .filter(|(_, buf)| buf.is_dirty() && buf.file_meta.is_some())
        .map(|(id, _)| id)
        .collect();

    if dirty_savable.is_empty() {
        return Ok(());
    }

    let mut count = 0;
    for bid in dirty_savable {
        let (content, line_count) = serialize_buffer(ed, bid);
        write_buffer_by_id(ed, bid, content, line_count, force)?;
        count += 1;
    }
    ed.report(Severity::Info, format!("Written {count} file(s)"));
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
