use hume_engine::pipeline::BufferId;

use super::super::Editor;
use super::super::Severity;
use super::jump::{BufferStep, goto_buffer_in_order};
use super::{FocusedPane, jump_pane_to};
use crate::editor::buffer::DiskCheckTrigger;
use crate::editor::error::CommandError;
use crate::editor::jump_list::{JumpRule, with_jump};

// ── Multi-buffer typed commands ───────────────────────────────────────────────

/// `:e [path[:line[:col]]]`: open a file in the current window.
///
/// - No `path`: reload current file from disk (`:e!` discards unsaved changes).
///   On a new-file buffer (`:e` on a path that doesn't exist yet, not written
///   since) this is a no-op: there is nothing on disk to reload from.
/// - `path` given and already open: switch to the existing buffer.
/// - `path` given and not open: read from disk, open a new buffer, switch to it.
///   A `path` that doesn't exist on disk opens an empty buffer bound to it
///   instead of erroring: `:w` creates the file (Vim's `:e newfile` semantics).
/// - `path` may carry a trailing `:line[:col]` position, split the same way
///   as the CLI's `hume path:line:col` (`cli::split_path_position`).
///   Landing on the position goes through `jump_pane_to`, so it records a
///   jump entry (`Ctrl-o` returns) and centers the viewport, same as
///   `:goto`/`goto-location!`.
///
/// Dedup uses `find_by_path` (canonical path comparison, or best-effort for a
/// not-yet-existing path; see `Editor::resolve_buffer_path`). `force` (`!`
/// suffix) only takes effect in the no-arg reload branch: it discards unsaved
/// changes and re-reads the file from disk. When a path is given, `force` is
/// unused.
pub(in crate::editor) fn typed_edit(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    use std::path::Path;

    if let Some(raw_arg) = arg {
        // `literal_exists` is only ever probed with `raw_arg` itself (see
        // `split_path_position`), so this lookup covers both that probe and
        // the "already open, reuse it" check below without repeating it. An
        // unsaved new-file buffer literally named `notes:12` must stay
        // reachable the same way a file on disk does, so this checks open
        // buffers first, on top of `cli::literal_path_on_disk`'s disk check.
        let raw_expanded = hume_platform::path::expand(raw_arg);
        let literal_bid = find_buffer_by_path_arg(ed, raw_expanded.as_ref());
        let literal_exists = |candidate: &str| {
            literal_bid.is_some() || crate::cli::literal_path_on_disk(candidate, &ed.state.cwd)
        };
        let (path_str, pos) = crate::cli::split_path_position(raw_arg, literal_exists)
            .map_err(CommandError::transient)?;

        // If a buffer is already open for this path, reuse it without re-reading.
        // Matches Vim semantics and covers the deleted-from-disk case.
        let existing = if path_str == raw_arg {
            literal_bid
        } else {
            find_buffer_by_path_arg(ed, hume_platform::path::expand(path_str).as_ref())
        };
        let (bid, opened_msg) = if let Some(bid) = existing {
            (bid, None)
        } else {
            let (bid, is_new) = ed
                .resolve_open_path(path_str)
                .map_err(|e| CommandError::new(format!("{path_str}: {e}")))?;
            let msg = is_new.then(|| {
                let buf = ed.state.buffers.get(bid);
                Editor::new_file_open_msg(buf)
                    .unwrap_or_else(|| format!("Opened {}", buf.display_name()))
            });
            (bid, msg)
        };

        // `jump_pane_to` does the only switch when a position is given; see
        // its caller contract.
        match pos {
            Some(pos) => {
                let target = crate::editor::pane_state::line_grapheme_to_cluster(
                    ed.state.buffers.get(bid).text(),
                    pos.line,
                    pos.grapheme_col,
                );
                jump_pane_to(&mut ed.state, &mut ed.view, fp.pane(), bid, target);
            }
            None => ed.enter_buffer(fp, bid),
        }
        if let Some(msg) = opened_msg {
            ed.report(Severity::Info, msg);
        }
        Ok(())
    } else {
        // Reload current file. The history-preserving reload keeps the existing
        // Buffer (only its text + file_meta are swapped), so `path` and
        // `display_path` are retained as-is, with no need to re-seed them onto the
        // freshly read doc.
        let doc = super::doc(&ed.state, &ed.view, fp.pane());
        let Some(path) = doc.path().map(Path::to_path_buf) else {
            return Err(CommandError::transient("no file name"));
        };
        // Nothing on disk to reload from yet: a reload here would just be a
        // no-op, so short-circuit before the dirty check rather than making
        // the user add `!` to force a reload that would discard edits for no
        // reason.
        if doc.is_new_file() {
            let name = doc.display_name();
            ed.report(
                Severity::Info,
                format!("{name}: new file, nothing to reload"),
            );
            return Ok(());
        }
        if ed.state.has_unsaved_changes(fp.bid(&ed.view)) && !force {
            return Err(CommandError::transient(
                "unsaved changes (use :e! to force)",
            ));
        }
        let display = doc
            .display_path()
            .expect("path is Some ⇒ display_path is Some (Buffer::set_path)")
            .to_string();
        ed.reload_from_path(fp, &path)
            .map_err(|e| CommandError::new(format!("{display}: {e}")))
    }
}

/// `:checktime`: check every open buffer against its backing file, right
/// now, without waiting for the next automatic trigger (terminal focus,
/// buffer-enter, return from an inline shell command). Silent when nothing
/// changed; otherwise reports/prompts like any other trigger, with one
/// exception: a buffer whose change the user already declined (`[k]eep`)
/// still gets a warning here: a direct "check now" request must never come
/// back silent just because an earlier prompt was dismissed. See
/// `DiskCheckTrigger::Explicit`. `force` has no effect: force accepting a
/// reload is what the confirm's `[r]eload` choice (or `:e!`) is for.
pub(in crate::editor) fn typed_checktime(
    ed: &mut Editor,
    _fp: FocusedPane,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.check_all_disk_state(DiskCheckTrigger::Explicit);
    Ok(())
}

/// `:cd [path]`: change the working directory.
///
/// - No arg: change to `$HOME`.
/// - `path` given: `~` / env-var expansion applied first; relative paths
///   resolve against the editor's cwd.
pub(in crate::editor) fn typed_cd(
    ed: &mut Editor,
    _fp: FocusedPane,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let target = match arg.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => {
            let expanded = hume_platform::path::expand(s);
            std::path::PathBuf::from(expanded.as_ref())
        }
        None => hume_platform::dirs::home_dir().ok_or_else(|| CommandError::new("HOME not set"))?,
    };

    let resolved = ed
        .state
        .set_cwd(&target)
        .map_err(|e| CommandError::new(format!("{}: {e}", target.display())))?;
    ed.report(
        Severity::Info,
        format!("cwd: {}", hume_platform::path::display_form(&resolved)),
    );
    Ok(())
}

/// `:pwd` / `:print-working-directory`: display the current working directory.
pub(in crate::editor) fn typed_pwd(
    ed: &mut Editor,
    _fp: FocusedPane,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.report(
        Severity::Info,
        hume_platform::path::display_form(&ed.state.cwd),
    );
    Ok(())
}

/// `:bd`: delete (close) the focused buffer.
///
/// If the buffer is dirty and `force` is false, returns an error.
/// If it is the only buffer, it is replaced with a scratch buffer.
pub(in crate::editor) fn typed_buffer_delete(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    let id = fp.bid(&ed.view);
    if ed.state.has_unsaved_changes(id) && !force {
        return Err(CommandError::transient(
            "unsaved changes (use :bd! to force)",
        ));
    }
    ed.close_buffer(id);
    Ok(())
}

/// `:b` / `:buffer`: switch to an open buffer by name, prefix, index, or full path.
///
/// Accepts four argument forms (tried in order):
/// 1. Numeric 1-based index matching `:ls` output.
/// 2. Absolute path (after `~`/env-var expansion), resolved via canonicalize
///    then looked up in the store.
/// 3. Exact display-name match (basename or `*scratch*`).
/// 4. Unique basename prefix.
///
/// The `force` flag is accepted syntactically but has no effect: there is
/// nothing to force on a plain buffer switch.
pub(in crate::editor) fn typed_buffer(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let arg = arg.ok_or_else(|| CommandError::transient("usage: :b <name|#|index>"))?;
    let bid = resolve_buffer_arg(ed, arg)?;
    ed.enter_buffer(fp, bid);
    Ok(())
}

/// Find an open buffer matching a path argument.
///
/// Uses the same resolution `resolve_open_path` uses to open one
/// (`Editor::resolve_buffer_path`), so a path that doesn't yet exist on disk
/// still matches an already-open new-file buffer instead of opening a
/// duplicate, and a path whose backing file has since been deleted still
/// matches the buffer that was reading it.
///
/// Falls back to the plain lexical-absolute form (no `canonicalize` at all)
/// when that first lookup misses: a new-file buffer opened while an
/// intermediate directory in its path was missing is keyed by
/// `resolve_buffer_path`'s fully-lexical fallback at that time (see its
/// doc); once the directory appears, re-resolving the same typed string
/// canonicalizes further and no longer matches the stored key. Skipped when
/// the two forms already agree, since the common case needs no second lookup.
fn find_buffer_by_path_arg(ed: &Editor, arg: &str) -> Option<BufferId> {
    let arg_path = std::path::Path::new(arg);
    let resolved = Editor::resolve_buffer_path(arg_path, &ed.state.cwd);
    if let Some(bid) = ed.state.buffers.find_by_path(&resolved) {
        return Some(bid);
    }
    let lexical = hume_platform::path::absolute_unresolved(arg_path, &ed.state.cwd);
    if lexical != resolved {
        return ed.state.buffers.find_by_path(&lexical);
    }
    None
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
        buf.display_path()
            .map(str::to_owned)
            .unwrap_or_else(|| Buffer::SCRATCH_BUFFER_NAME.to_owned())
    };

    // 0. `#`: the alternate buffer (Vim's `<C-^>` equivalent). Resolved by
    //    ID, not by path, so pathless buffers (scratch, [messages], the
    //    [buffers] view from :ls) remain reachable as the alternate.
    if arg == "#" {
        return ed
            .state
            .buffers
            .second_most_recent()
            .ok_or_else(|| CommandError::transient("no alternate buffer"));
    }

    // 1. Numeric 1-based index.
    if let Ok(n) = arg.parse::<usize>() {
        let idx = n
            .checked_sub(1)
            .ok_or_else(|| CommandError::transient(format!("no buffer at index {n}")))?;
        return ed
            .state
            .buffers
            .iter()
            .nth(idx)
            .map(|(id, _)| id)
            .ok_or_else(|| CommandError::transient(format!("no buffer at index {n}")));
    }

    // 2. Absolute path: match an open buffer by canonical OR lexical path.
    //    Lexical fallback keeps buffers reachable after their file is deleted.
    //    `~`/env-var expansion first, matching :e's handling (typed_edit):
    //    an ambiguity message's label can be a `~`-collapsed display_path, so
    //    retyping it verbatim must resolve the same way :e would.
    let expanded = hume_platform::path::expand(arg);
    if Path::new(expanded.as_ref()).is_absolute() {
        return find_buffer_by_path_arg(ed, expanded.as_ref())
            .ok_or_else(|| CommandError::transient(format!("{arg}: not an open buffer")));
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
            return Err(CommandError::transient(format!(
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
        0 => Err(CommandError::transient(format!(
            "no buffer matching '{arg}'"
        ))),
        1 => Ok(prefix_matches[0]),
        _ => {
            let labels: Vec<String> = prefix_matches
                .iter()
                .map(|&id| label(ed.state.buffers.get(id)))
                .collect();
            Err(CommandError::transient(format!(
                "ambiguous prefix '{arg}': {}",
                labels.join(", ")
            )))
        }
    }
}

/// Take one open-order buffer step for `:bnext`/`:bprev`, recording by hand
/// the jump the mappable `goto-next-buffer`/`goto-prev-buffer` siblings get
/// from their `.jump()` meta: the `:` dispatcher reads no `CmdMeta`.
fn typed_buffer_step(
    ed: &mut Editor,
    fp: FocusedPane,
    step: BufferStep,
) -> Result<(), CommandError> {
    let t = fp.pane();
    with_jump(
        &mut ed.state,
        &mut ed.view,
        t,
        JumpRule::IfMoved,
        |state, view| {
            goto_buffer_in_order(state, view, t, step);
        },
    );
    Ok(())
}

/// `:bnext` / `:bn`: switch to the next buffer in open-order.
pub(in crate::editor) fn typed_bnext(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    typed_buffer_step(ed, fp, BufferStep::Next)
}

/// `:bprev` / `:bp`: switch to the previous buffer in open-order.
pub(in crate::editor) fn typed_bprev(
    ed: &mut Editor,
    fp: FocusedPane,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    typed_buffer_step(ed, fp, BufferStep::Prev)
}
