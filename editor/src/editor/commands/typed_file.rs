use super::super::{ops, Severity};
use super::super::Editor;
use editing::error::CommandError;

// ── Typed file commands ───────────────────────────────────────────────────────

pub fn typed_quit(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if !force && ed.doc().is_dirty() {
        return Err(CommandError("Unsaved changes (add ! to override)".into()));
    }

    let current = ed.focused_buffer_id();
    // "Real" = has a backing file OR is editable (a writable scratch buffer).
    // Pure read-only view buffers like [messages] are ephemeral — not worth staying for.
    let any_other_real = ed
        .buffers
        .iter()
        .filter(|(id, _)| *id != current)
        .any(|(_, buf)| buf.path().is_some() || !buf.is_read_only());

    if !any_other_real {
        ed.should_quit = true;
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
    if !force && ed.buffers.iter().any(|(_, buf)| buf.is_dirty()) {
        return Err(CommandError(
            "Unsaved changes in open buffers (add ! to override)".into(),
        ));
    }
    ed.should_quit = true;
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
            ed.should_quit = true;
            Ok(())
        }
        Err(e) if force => {
            ed.should_quit = true;
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
    use engine::pane::WrapMode;
    let currently_wrapping = ed.doc().overrides.wrap_mode(&ed.settings).is_wrapping();
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
        return Err(CommandError(USAGE.into()));
    };
    let Some((scope, rest)) = arg.split_once(' ') else {
        return Err(CommandError(USAGE.into()));
    };
    let Some((key, value)) = rest.split_once('=') else {
        return Err(CommandError("Expected key=value".into()));
    };
    let bid = ed.focused_buffer_id();

    // Language is a per-buffer property, not a generic setting.
    if key == "language" {
        return match scope {
            "global" => Err(CommandError(
                "'language' is per-buffer — use ':set buffer language=<name>'".into(),
            )),
            "buffer" => {
                let new_lang = if value.is_empty() { None } else { Some(value.to_owned()) };
                if let Some(ref name) = new_lang
                    && ed.languages.by_name(name).is_none()
                {
                    ed.report(
                        Severity::Warning,
                        format!("language '{name}' is not registered"),
                    );
                }
                ed.set_buffer_language(bid, new_lang);
                Ok(())
            }
            _ => Err(CommandError(format!(
                "unknown scope '{scope}': expected 'global' or 'buffer'"
            ))),
        };
    }

    let result = match scope {
        "global" => crate::settings::apply_setting(
            crate::settings::SettingScope::Global,
            key,
            value,
            &mut ed.settings,
            &mut ed.buffers.get_mut(bid).overrides,
        ),
        "buffer" => crate::settings::apply_setting(
            crate::settings::SettingScope::Text,
            key,
            value,
            &mut ed.settings,
            &mut ed.buffers.get_mut(bid).overrides,
        ),
        _ => Err(format!(
            "unknown scope '{scope}': expected 'global' or 'buffer'"
        )),
    };
    if result.is_ok() && key == "history-capacity" {
        ed.history.set_capacity(ed.settings.history_capacity);
    }
    if result.is_ok() && key == "theme" && scope == "global" && !ed.settings.theme.is_empty() {
        ops::load_theme_by_name(
            &mut ed.engine_view,
            &mut ed.message_log,
            &mut ed.status_msg,
            &ed.settings.theme,
        );
    }
    result.map_err(CommandError)
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
    let (content, line_count) = {
        let buf = ed.doc().text();
        // The rope is always stored LF-normalized; restore CRLF for files that
        // originally used it so we don't silently change line endings on save.
        let content = if buf.line_ending() == editing::text::LineEnding::CrLf {
            buf.to_string().replace('\n', "\r\n")
        } else {
            buf.to_string()
        };
        // The buffer always ends with a structural '\n', so len_lines() returns
        // one more than the number of visible lines (ropey counts the empty
        // string after the final newline as an extra line).
        let line_count = buf.len_lines().saturating_sub(1);
        (content, line_count)
    };

    if let Some(path_str) = arg {
        let expanded = platform::path::expand(path_str);
        let path: std::path::PathBuf = {
            let p = std::path::Path::new(expanded.as_ref());
            // Resolve relative paths against editor.cwd, not the process cwd,
            // so `:w relpath` is stable regardless of how the process cwd drifts.
            if p.is_relative() { ed.cwd.join(p) } else { p.to_owned() }
        };
        // Try to preserve existing file's permissions; if the file doesn't
        // exist yet, write_file_new creates it with default permissions.
        let result = match platform::io::read_file_meta(&path) {
            Ok(meta) => platform::io::write_file_atomic(&content, &meta, force)
                .map(|retried| (meta, retried)),
            Err(_) => platform::io::write_file_new(&content, &path).map(|meta| (meta, false)),
        };
        match result {
            Ok((meta, retried)) => {
                // Store the canonicalized path so path and file_meta.resolved_path
                // always agree, even when the user supplied a relative or symlink path.
                // Synthetic buffers (e.g. [messages]) stay path-less after save-as —
                // the write dumps content to disk but the buffer itself is unaffected.
                if !ed.doc().is_synthetic() {
                    ed.doc_mut().set_path(Some(meta.resolved_path.clone()));
                    ed.doc_mut().file_meta = Some(meta);
                }
                ed.doc_mut().mark_saved();
                ed.report(write_severity(retried), write_msg(line_count, retried));
                ed.fire_hook_buffer_save(ed.focused_buffer_id());
                Ok(())
            }
            Err(e) => Err(CommandError(e.to_string())),
        }
    } else {
        if ed.doc().is_read_only() {
            return Err(CommandError("Buffer is read-only".into()));
        }
        // Write to the current file.
        let Some(meta) = ed.doc().file_meta.as_ref() else {
            return Err(CommandError("no file name".into()));
        };
        match platform::io::write_file_atomic(&content, meta, force) {
            Ok(retried) => {
                ed.doc_mut().mark_saved();
                ed.report(write_severity(retried), write_msg(line_count, retried));
                ed.fire_hook_buffer_save(ed.focused_buffer_id());
                Ok(())
            }
            Err(e) => Err(CommandError(e.to_string())),
        }
    }
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
