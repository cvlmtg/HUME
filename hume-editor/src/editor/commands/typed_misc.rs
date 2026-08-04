use hume_engine::pipeline::{BufferId, Direction};

use super::super::Editor;
use super::super::Severity;
use super::current_jump_entry;
use crate::editor::error::CommandError;
use crate::settings::THEME_KEY;
use hume_ops::edit::{SortOpts, SortRefusal, sort_rows};

// ── Message log ──────────────────────────────────────────────────────────────

/// `:messages` — open the message log in a read-only buffer.
///
/// Displays all logged warnings, errors, and trace entries accumulated during
/// the session. Cursor starts at the last entry (most recent). Dismiss with
/// `:bd` or switch away with `:b#`.
pub(crate) fn typed_messages(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let (content, spans) = ed.state.message_log.format_with_spans();
    if content.is_empty() {
        ed.report(Severity::Info, "No messages".to_string());
        return Ok(());
    }
    ed.state.message_log.mark_all_seen();
    // open_read_only_view clamps cursor_line to the last content line — pass
    // usize::MAX so it always positions at the bottom (most recent entry).
    let bid = ed.open_read_only_view("[messages]", &content, usize::MAX);
    // Wholesale replace under a fixed source name — repeat `:messages` calls
    // route through set_view_content (no ChangeSet), so stale spans from a
    // prior call can't be remapped and must be overwritten here instead.
    ed.state
        .config
        .decorations
        .set_extra_highlights("messages".to_string(), bid, spans);
    Ok(())
}

/// `:ls` / `:list-buffers` — open a read-only buffer listing every open buffer.
///
/// Each row shows: 1-based index, current (`%`) / alternate (`#`) marker,
/// dirty (`+`) flag, short name, and home-shortened absolute path.
/// Cursor is placed on the row corresponding to the currently focused buffer.
pub(crate) fn typed_list_buffers(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let current = ed.focused_buffer_id();
    let alternate = ed.alternate_buffer();

    let header = format!("{:>4}      {:<32}  {}\n", "buf", "name", "path");
    let mut out = header;
    // The [buffers] view buffer (if it already exists from a prior :ls) must not
    // appear in its own listing. All other buffers — including [messages] and
    // [plugin-status] — are listed normally.
    let buffers_view_id = ed.state.buffers.find_by_label("[buffers]");
    // `row` counts emitted rows (1-based, offset by the header at rope line 0).
    // Tracked independently from the slotmap iteration index because [buffers]
    // may be skipped without a row being emitted.
    let mut row: usize = 0;
    let mut current_rope_line: usize = 1;

    for (id, buf) in ed.state.buffers.iter() {
        if buffers_view_id == Some(id) {
            continue;
        }
        row += 1;

        let cur_marker = if id == current {
            '%'
        } else if matches!(alternate, Some(alt) if id == alt) {
            '#'
        } else {
            ' '
        };
        let dirty_marker = if buf.is_dirty() { '+' } else { ' ' };

        let name = buf
            .path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(buf.label.as_deref().unwrap_or("*scratch*"));
        let path = buf.display_path().unwrap_or_default();

        out.push_str(&format!(
            "{:>4}  {}{}  {:<32}  {}\n",
            row, cur_marker, dirty_marker, name, path
        ));

        if id == current {
            current_rope_line = row; // rope line = header(0) + emitted rows(1-based)
        }
    }

    ed.open_read_only_view("[buffers]", &out, current_rope_line);
    Ok(())
}

/// `:plugin-status` / `:plugins` — show all declared plugins, their load
/// state, and (for still-waiting plugins) which activation entries they are waiting on.
pub(crate) fn typed_plugin_status(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let out = if let Some(host) = ed.scripting.as_ref() {
        host.lazy_status_string(&ed.state.config.registry.lazy_stubs())
    } else {
        ed.report(Severity::Info, "Scripting disabled".to_string());
        return Ok(());
    };
    if out.is_empty() {
        ed.report(Severity::Info, "No plugins declared".to_string());
        return Ok(());
    }
    ed.open_read_only_view("[plugin-status]", &out, 0);
    Ok(())
}

// ── :split / :vsplit ──────────────────────────────────────────────────────────

/// `:split [path]` — split the focused pane, stacking the new pane below it.
///
/// With no `path`, the new pane views the same buffer as the focused one.
/// With `path`, the new pane views that file instead (opened via the usual
/// dedup-on-canonical-path rule — see [`open_path_arg`]).
pub(crate) fn typed_split(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    split_focused_pane(ed, arg, Direction::Vertical)
}

/// `:vsplit [path]` — split the focused pane side by side.
pub(crate) fn typed_vsplit(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    split_focused_pane(ed, arg, Direction::Horizontal)
}

/// Split the focused pane and move focus to the new pane.
///
/// `direction` is the engine's split axis, which is *inverted* from the Vim
/// command names: `Direction::Vertical` divides height (stacked panes, what
/// `:split` means), `Direction::Horizontal` divides width (side by side,
/// `:vsplit`) — see `LayoutTree::collect_rects_into`'s use of `split_rect`.
///
/// Checks `fits_split` up front, before resolving `arg`, so a too-small pane
/// rejects the split without the side effect of opening a path argument's
/// file. `split_pane_onto` (the shared core with the keymap-bound
/// `pane-split`/`pane-vsplit` commands) checks again once `bid` is known —
/// redundant here but the only guard on the no-arg keymap path.
fn split_focused_pane(
    ed: &mut Editor,
    arg: Option<&str>,
    direction: Direction,
) -> Result<(), CommandError> {
    if !super::fits_split(&ed.state, &ed.view, direction) {
        ed.report(Severity::Warning, super::SPLIT_TOO_SMALL_MSG.to_string());
        return Ok(());
    }
    let bid = match arg {
        Some(path) => open_path_arg(ed, path)?,
        None => ed.focused_buffer_id(),
    };
    super::split_pane_onto(&mut ed.state, &mut ed.view, bid, direction)
}

/// Resolve a `:split`/`:vsplit` path argument to a `BufferId`, opening the
/// file if it isn't already open. Thin wrapper over the shared
/// resolve-dedup-open sequence in [`Editor::resolve_open_path`].
fn open_path_arg(ed: &mut Editor, path_str: &str) -> Result<BufferId, CommandError> {
    let (bid, _) = ed
        .resolve_open_path(path_str)
        .map_err(|e| CommandError::new(format!("{path_str}: {e}")))?;
    Ok(bid)
}

/// `:theme <name>` — load a theme by name from the theme search path.
///
/// On success the engine view's theme is replaced; the next `prepare_frame`
/// re-bakes it (see `Theme::bake_if_stale`). On failure a warning is shown and
/// the current theme is left unchanged.
pub(crate) fn typed_theme(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let Some(name) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
        let current: &str = if ed.state.settings.theme.is_empty() {
            super::DEFAULT_THEME_LABEL
        } else {
            &ed.state.settings.theme
        };
        // NLL: `current` borrow of ed.state.settings.theme ends inside format!(), before report().
        ed.report(Severity::Info, format!("Current theme: {current}"));
        return Ok(());
    };
    crate::editor::settings_ops::apply_global(&mut ed.state, &mut ed.view, THEME_KEY, name)
        .map_err(CommandError::new)
}

/// `:theme-debug` — print the resolved style chain for key UI scopes.
///
/// Reports the scope name, resolution chain, and final fg/bg/modifiers for
/// the cursor, selection, and cursorline scopes from the active theme.
pub(crate) fn typed_theme_debug(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use ratatui::style::Color;

    fn color_str(c: Option<Color>) -> String {
        match c {
            Some(Color::Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            Some(other) => format!("{other:?}"),
            None => "-".to_owned(),
        }
    }

    fn scope_chain(theme: &hume_engine::theme::Theme, scope: &str) -> String {
        // Walk the dot-notation prefix chain and collect names that have entries.
        let mut chain: Vec<&str> = Vec::new();
        let mut cur = scope;
        loop {
            if theme.raw_contains(cur) {
                chain.push(cur);
            }
            match cur.rfind('.') {
                Some(dot) => cur = &cur[..dot],
                None => break,
            }
        }
        if chain.is_empty() {
            format!("{scope} → default")
        } else {
            chain.join(" → ")
        }
    }

    let theme = &ed.view.theme;
    let name = if ed.state.settings.theme.is_empty() {
        super::DEFAULT_THEME_LABEL
    } else {
        &ed.state.settings.theme
    };

    let scopes = [
        "ui.cursor.primary",
        "ui.cursor",
        "ui.cursor.insert",
        "ui.selection",
        "ui.cursorline",
        "ui.statusline",
    ];

    let mut lines = vec![format!("Theme: {name}")];
    for scope in scopes {
        let style = theme.resolve_by_name(hume_engine::types::Scope(scope));
        let chain = scope_chain(theme, scope);
        lines.push(format!(
            "  {scope}: chain={chain} fg={} bg={}{}",
            color_str(style.fg),
            color_str(style.bg),
            if style.modifiers.is_empty() {
                String::new()
            } else {
                format!(" modifiers={:?}", style.modifiers)
            },
        ));
    }

    ed.report(Severity::Info, lines.join("\n"));
    Ok(())
}

pub(crate) fn typed_version(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.report(Severity::Info, format!("hume {}", crate::VERSION));
    Ok(())
}

pub(crate) fn typed_tutor(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    // Resolve the install source. Fail fast on missing runtime or file.
    let Some(runtime) = hume_platform::dirs::runtime_dir() else {
        return Err(CommandError::new(
            "runtime directory not found (set HUME_RUNTIME to override)",
        ));
    };
    let source_path = runtime.join("tutor.rst");
    let source = std::fs::canonicalize(&source_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CommandError::new(format!(
                "tutor.rst not found at {} (set HUME_RUNTIME to override)",
                source_path.display()
            ))
        } else {
            CommandError::new(format!("could not access tutor.rst: {e}"))
        }
    })?;

    // Compute a per-process tmp path so `:w` never touches the install source.
    // Canonicalize the parent dir (which we create) so the path matches what
    // BufferStore stores on macOS (/private/var/... vs /var/...).
    let tmp_dir = std::env::temp_dir().join(format!("hume-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| CommandError::new(format!("could not create tutor tmp dir: {e}")))?;
    let canonical_tmp = std::fs::canonicalize(&tmp_dir)
        .map_err(|e| CommandError::new(format!("could not canonicalize tutor tmp dir: {e}")))?
        .join("tutor.rst");

    // If a buffer is already open at the tmp path, switch — no re-copy so that
    // unsaved in-memory edits are preserved.
    if let Some(bid) = ed.state.buffers.find_by_path(&canonical_tmp) {
        ed.switch_to_buffer_with_jump(bid);
        return Ok(());
    }

    // No live buffer at tmp. Copy fresh source content (overwrites any stale
    // file from a prior `:bd!`), then open the copy.
    std::fs::copy(&source, &canonical_tmp)
        .map_err(|e| CommandError::new(format!("could not copy tutor.rst to tmp: {e}")))?;
    let (bid, _) = ed
        .open_or_dedup(&canonical_tmp)
        .map_err(|e| CommandError::new(format!("could not open tutor copy: {e}")))?;
    ed.switch_to_buffer_with_jump(bid);
    Ok(())
}

// ── Go-to-line ────────────────────────────────────────────────────────────────

/// `:goto N` — jump to 1-based line `N`, clamped to the last content line.
///
/// The pre-jump position is recorded in the jump list so `Ctrl+o` returns here.
/// `:42` is accepted as shorthand (the command-mode dispatcher intercepts bare
/// digit strings and routes them here before the normal registry lookup).
pub(crate) fn typed_goto_line(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use hume_editing::selection::{Selection, SelectionSet};

    let raw = arg.ok_or_else(|| CommandError::new(":goto requires a line number"))?;
    let n: usize = raw
        .trim()
        .parse()
        .map_err(|_| CommandError::new(format!("invalid line number: {raw}")))?;
    let line0 = n
        .checked_sub(1)
        .ok_or_else(|| CommandError::new("line numbers start at 1"))?;

    // len_lines() counts the ghost line after the trailing '\n', so the last
    // real content line is always at index len_lines() - 2.
    let last = ed.doc().text().len_lines().saturating_sub(2);
    let target = line0.min(last);
    let char_pos = ed.doc().text().line_to_char(target);

    // Record pre-jump position before moving so Ctrl+O can return here.
    let entry = current_jump_entry(&ed.state, &ed.view);
    ed.state.panes.jumps[ed.state.focused_pane_id].push(entry);

    ed.set_current_selections(SelectionSet::single(Selection::collapsed(char_pos)));
    Ok(())
}

// ── Sort ──────────────────────────────────────────────────────────────────────

/// Parse `:sort`'s flag string: `-r`/`--reverse`, `-i`/`--insensitive`, and
/// bundled shorts (`-ri`, `-ir`). No positional arguments are accepted.
fn parse_sort_flags(arg: Option<&str>) -> Result<SortOpts, CommandError> {
    let mut opts = SortOpts::default();
    let Some(arg) = arg else { return Ok(opts) };
    for token in arg.split_whitespace() {
        match token {
            "-r" | "--reverse" => opts.reverse = true,
            "-i" | "--insensitive" => opts.insensitive = true,
            // Any other `--`-prefixed token is a long flag, just not one we
            // recognize — report it as a flag, not a positional argument.
            _ if token.starts_with("--") => {
                return Err(CommandError::new(format!("unknown flag: {token}")));
            }
            _ if token.starts_with('-') && token.len() > 1 => {
                for ch in token[1..].chars() {
                    match ch {
                        'r' => opts.reverse = true,
                        'i' => opts.insensitive = true,
                        _ => return Err(CommandError::new(format!("unknown flag: -{ch}"))),
                    }
                }
            }
            _ => return Err(CommandError::new(format!("unknown argument: {token}"))),
        }
    }
    Ok(opts)
}

/// `:sort` — sort each maximal run of adjacent rows touched by a selection,
/// keyed by the selected text on that row. Flags: `-r`/`--reverse`,
/// `-i`/`--insensitive`.
///
/// Diverges deliberately from Helix's `:sort`, which permutes text *between*
/// selection slots and leaves row boundaries untouched — this permutes the
/// rows themselves, closer to `sort -k`. See `hume_ops::edit::sort` for the
/// full semantics (grouping, numeric auto-detection, selection remapping)
/// and its rejection of Kakoune's `|sort` too.
pub(crate) fn typed_sort(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if force {
        return Err(CommandError::new(
            "`:sort` takes no `!` — use `-r` to reverse",
        ));
    }
    let opts = parse_sort_flags(arg)?;

    // doc_ops's read-only guard is a silent no-op — check explicitly here so
    // `:sort` on a read-only buffer (e.g. `:messages`) reports why nothing happened.
    if ed.focused_buffer_read_only() {
        return Err(CommandError::new("Buffer is read-only"));
    }

    // Computing the sort before touching the buffer is what lets a refusal
    // (`SortRefusal`) leave the buffer untouched — an identity edit would
    // still record an undo revision and mark the buffer dirty.
    let result = sort_rows(
        ed.doc().text().clone(),
        ed.current_selections().clone(),
        opts,
    );
    let triple = match result {
        Ok(triple) => triple,
        Err(SortRefusal::NoAdjacentRows) => {
            ed.report(
                Severity::Warning,
                "sort needs at least two adjacent rows".to_string(),
            );
            return Ok(());
        }
        Err(SortRefusal::AlreadySorted) => {
            ed.report(Severity::Info, "already sorted".to_string());
            return Ok(());
        }
    };

    let pre_len = ed.doc().text().len_chars();
    let pre_sels = ed.current_selections().clone();
    super::apply_focused_edit(&mut ed.state, &ed.view, move |buf, sels| {
        debug_assert_eq!(
            buf.len_chars(),
            pre_len,
            "sort_rows must run against the same buffer just read"
        );
        debug_assert_eq!(
            sels, pre_sels,
            "sort_rows must run against the same selections just read"
        );
        triple
    });
    Ok(())
}
