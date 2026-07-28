use hume_engine::pipeline::BufferId;

use super::super::Editor;
use super::super::Severity;
use crate::editor::buffer::DiskCheckTrigger;
use crate::editor::error::CommandError;

// ── Multi-buffer typed commands ───────────────────────────────────────────────

/// `:e [path]` — open a file in the current window.
///
/// - No `path`: reload current file from disk (`:e!` discards unsaved changes).
/// - `path` given and already open: switch to the existing buffer.
/// - `path` given and not open: read from disk, open a new buffer, switch to it.
///
/// Dedup uses `find_by_path` (canonical path comparison). `force` (`!` suffix)
/// only takes effect in the no-arg reload branch: it discards unsaved changes
/// and re-reads the file from disk. When a path is given, `force` is unused.
pub fn typed_edit(ed: &mut Editor, arg: Option<&str>, force: bool) -> Result<(), CommandError> {
    use std::path::Path;

    if let Some(path_str) = arg {
        let expanded = hume_platform::path::expand(path_str);

        // If a buffer is already open for this path, switch without re-reading.
        // Matches Vim semantics and covers the deleted-from-disk case.
        if let Some(bid) = find_buffer_by_path_arg(ed, expanded.as_ref()) {
            if bid != ed.focused_buffer_id() {
                ed.switch_to_buffer_with_jump(bid);
            }
            ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);
            return Ok(());
        }

        let (bid, is_new) = ed
            .resolve_open_path(path_str)
            .map_err(|e| CommandError::new(format!("{path_str}: {e}")))?;
        if is_new {
            let name = ed.state.buffers.get(bid).display_name();
            ed.switch_to_buffer_with_jump(bid);
            ed.report(Severity::Info, format!("Opened {name}"));
        } else if bid != ed.focused_buffer_id() {
            ed.switch_to_buffer_with_jump(bid);
        }
        Ok(())
    } else {
        // Reload current file. The history-preserving reload keeps the existing
        // Buffer (only its text + file_meta are swapped), so `path` and
        // `display_path` are retained as-is — no need to re-seed them onto the
        // freshly read doc.
        let Some(path) = ed.doc().path().map(Path::to_path_buf) else {
            return Err(CommandError::new("no file name"));
        };
        if ed.doc().is_dirty() && !force {
            return Err(CommandError::new("unsaved changes (use :e! to force)"));
        }
        let doc = crate::editor::buffer::Buffer::from_file(&path)
            .map_err(|e| CommandError::new(format!("{}: {e}", path.display())))?;
        let id = ed.focused_buffer_id();
        ed.reload_buffer_in_place(id, doc);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        ed.report(Severity::Info, format!("Reloaded {name}"));
        Ok(())
    }
}

/// `:checktime` — check every open buffer against its backing file, right
/// now, without waiting for the next automatic trigger (terminal focus,
/// buffer-enter, return from an inline shell command). Silent when nothing
/// changed; otherwise reports/prompts exactly like any other trigger — see
/// `Editor::check_all_disk_state`. `force` has no effect: force accepting a
/// reload is what the confirm's `[r]eload` choice (or `:e!`) is for.
pub fn typed_checktime(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.check_all_disk_state();
    Ok(())
}

/// `:cd [path]` — change the working directory.
///
/// - No arg: change to `$HOME`.
/// - `path` given: `~` / env-var expansion applied first; relative paths
///   resolve against the current process cwd (which mirrors `editor.cwd`).
pub fn typed_cd(ed: &mut Editor, arg: Option<&str>, _force: bool) -> Result<(), CommandError> {
    let target = match arg.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => {
            let expanded = hume_platform::path::expand(s);
            std::path::PathBuf::from(expanded.as_ref())
        }
        None => hume_platform::dirs::home_dir().ok_or_else(|| CommandError::new("HOME not set"))?,
    };

    let resolved = ed
        .set_cwd(&target)
        .map_err(|e| CommandError::new(format!("{}: {e}", target.display())))?;
    ed.report(Severity::Info, format!("cwd: {}", resolved.display()));
    Ok(())
}

/// `:pwd` / `:print-working-directory` — display the current working directory.
pub fn typed_pwd(ed: &mut Editor, _arg: Option<&str>, _force: bool) -> Result<(), CommandError> {
    ed.report(
        Severity::Info,
        hume_platform::path::shorten_home(&ed.state.cwd),
    );
    Ok(())
}

/// `:bd` — delete (close) the focused buffer.
///
/// If the buffer is dirty and `force` is false, returns an error.
/// If it is the only buffer, it is replaced with a scratch buffer.
pub fn typed_buffer_delete(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if ed.doc().is_dirty() && !force {
        return Err(CommandError::new("unsaved changes (use :bd! to force)"));
    }
    let id = ed.focused_buffer_id();
    ed.close_buffer(id);
    Ok(())
}

/// `:b` / `:buffer` — switch to an open buffer by name, prefix, index, or full path.
///
/// Accepts four argument forms (tried in order):
/// 1. Numeric 1-based index matching `:ls` output.
/// 2. Absolute path — resolved via canonicalize then looked up in the store.
/// 3. Exact display-name match (basename or `*scratch*`).
/// 4. Unique basename prefix.
///
/// The `force` flag is accepted syntactically but has no effect — there is
/// nothing to force on a plain buffer switch.
pub fn typed_buffer(ed: &mut Editor, arg: Option<&str>, _force: bool) -> Result<(), CommandError> {
    let arg = arg.ok_or_else(|| CommandError::new("usage: :b <name|#|index>"))?;
    let bid = resolve_buffer_arg(ed, arg)?;
    if bid != ed.focused_buffer_id() {
        ed.switch_to_buffer_with_jump(bid);
    }
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);
    Ok(())
}

/// Find an open buffer matching a path argument.
///
/// Tries `fs::canonicalize` first (resolves symlinks, requires the file to
/// exist), then falls back to `std::path::absolute` (pure lexical: joins with
/// cwd, removes `.`/`..`, no filesystem access). The fallback keeps buffers
/// reachable after their backing file has been deleted.
fn find_buffer_by_path_arg(ed: &Editor, arg: &str) -> Option<BufferId> {
    if let Ok(canonical) = hume_platform::fs::canonicalize(std::path::Path::new(arg))
        && let Some(bid) = ed.state.buffers.find_by_path(&canonical)
    {
        return Some(bid);
    }
    let abs = std::path::absolute(arg).ok()?;
    ed.state.buffers.find_by_path(&abs)
}

/// Resolve a `:b` argument to a `BufferId`.  See [`typed_buffer`] for the
/// four-step resolution order.
fn resolve_buffer_arg(ed: &Editor, arg: &str) -> Result<BufferId, CommandError> {
    use crate::editor::buffer::Buffer;
    use std::path::Path;

    // Label used in ambiguity messages: full path when available, literal
    // `*scratch*` otherwise. Unambiguous regardless of whether the collision
    // was on basename or prefix.
    let label = |buf: &Buffer| -> String {
        buf.path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| Buffer::SCRATCH_BUFFER_NAME.to_owned())
    };

    // 0. `#` — the alternate buffer (Vim's `<C-^>` equivalent). Resolved by
    //    ID, not by path, so pathless buffers (scratch, [messages], the
    //    [buffers] view from :ls) remain reachable as the alternate.
    if arg == "#" {
        return ed
            .alternate_buffer()
            .ok_or_else(|| CommandError::new("no alternate buffer"));
    }

    // 1. Numeric 1-based index.
    if let Ok(n) = arg.parse::<usize>() {
        let idx = n
            .checked_sub(1)
            .ok_or_else(|| CommandError::new(format!("no buffer at index {n}")))?;
        return ed
            .state
            .buffers
            .iter()
            .nth(idx)
            .map(|(id, _)| id)
            .ok_or_else(|| CommandError::new(format!("no buffer at index {n}")));
    }

    // 2. Absolute path — match an open buffer by canonical OR lexical path.
    //    Lexical fallback keeps buffers reachable after their file is deleted.
    if Path::new(arg).is_absolute() {
        return find_buffer_by_path_arg(ed, arg)
            .ok_or_else(|| CommandError::new(format!("{arg}: not an open buffer")));
    }

    // 3. Exact display-name match.
    let exact: Vec<BufferId> = ed
        .state
        .buffers
        .iter()
        .filter(|(_, buf)| buf.display_name() == arg)
        .map(|(id, _)| id)
        .collect();
    match exact.len() {
        1 => return Ok(exact[0]),
        n if n > 1 => {
            let labels: Vec<String> = exact
                .iter()
                .map(|&id| label(ed.state.buffers.get(id)))
                .collect();
            return Err(CommandError::new(format!(
                "ambiguous buffer name '{arg}': {}",
                labels.join(", ")
            )));
        }
        _ => {} // fall through to prefix match
    }

    // 4. Unique basename-prefix match.
    let prefix_matches: Vec<BufferId> = ed
        .state
        .buffers
        .iter()
        .filter(|(_, buf)| buf.display_name().starts_with(arg))
        .map(|(id, _)| id)
        .collect();
    match prefix_matches.len() {
        0 => Err(CommandError::new(format!("no buffer matching '{arg}'"))),
        1 => Ok(prefix_matches[0]),
        _ => {
            let labels: Vec<String> = prefix_matches
                .iter()
                .map(|&id| label(ed.state.buffers.get(id)))
                .collect();
            Err(CommandError::new(format!(
                "ambiguous prefix '{arg}': {}",
                labels.join(", ")
            )))
        }
    }
}

/// `:bnext` / `:bn` — switch to the next buffer in open-order.
pub fn typed_bnext(ed: &mut Editor, _arg: Option<&str>, _force: bool) -> Result<(), CommandError> {
    let target = ed.state.buffers.next(ed.focused_buffer_id());
    if target != ed.focused_buffer_id() {
        ed.switch_to_buffer_with_jump(target);
    }
    Ok(())
}

/// `:bprev` / `:bp` — switch to the previous buffer in open-order.
pub fn typed_bprev(ed: &mut Editor, _arg: Option<&str>, _force: bool) -> Result<(), CommandError> {
    let target = ed.state.buffers.prev(ed.focused_buffer_id());
    if target != ed.focused_buffer_id() {
        ed.switch_to_buffer_with_jump(target);
    }
    Ok(())
}
