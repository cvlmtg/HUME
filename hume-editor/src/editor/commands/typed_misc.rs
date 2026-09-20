use std::time::Duration;

use hume_engine::pipeline::{BufferId, Direction, EngineView};
use hume_grid::Rgb;
use hume_scripting::host::DecorationHost;

use super::super::Editor;
use super::super::{EditorState, Severity};
use super::{cmd_redo, cmd_undo, current_jump_entry, record_jump_if_moved};
use crate::editor::buffer::Buffer;
use crate::editor::error::CommandError;
use crate::editor::host_impl::EditorHostImpl;
use crate::editor::settings::THEME_KEY;
use hume_ops::MotionMode;
use hume_ops::edit::{SortOpts, SortRefusal, sort_lines};

// ── Message log ──────────────────────────────────────────────────────────────

/// `:messages` — open the message log in a read-only buffer.
///
/// Displays all logged warnings, errors, and trace entries accumulated during
/// the session. Cursor starts at the last entry (most recent). Dismiss with
/// `:bd` or switch away with `:b#`.
pub(in crate::editor) fn typed_messages(
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
    // None parks at the bottom (most recent entry).
    let bid = ed.open_read_only_view("[messages]", &content, None);
    let spans = spans
        .into_iter()
        .map(|(start, end, scope)| (start, end, scope.to_string()))
        .collect();
    // Wholesale replace under a fixed source name — repeat `:messages` calls
    // route through set_view_content (no ChangeSet), so stale spans from a
    // prior call can't be remapped and must be overwritten here instead.
    // Goes through the same `DecorationHost::set_extra_highlights` boundary
    // `set-extra-highlights!` calls, not a hand-built `ExtraHighlightEntry`
    // list — so `:messages`' spans get the same range validation as any
    // other caller instead of a second, unvalidated construction path.
    EditorHostImpl::new(&mut ed.state, &mut ed.view)
        .set_extra_highlights("messages".to_string(), bid, spans)
        .map_err(CommandError::new)?;
    Ok(())
}

/// `:ls` / `:list-buffers` — open a read-only buffer listing every open buffer.
///
/// Each row shows: 1-based index, current (`%`) / alternate (`#`) marker,
/// dirty (`+`) flag, short name, and home-shortened absolute path.
/// Cursor is placed on the line corresponding to the currently focused buffer.
pub(in crate::editor) fn typed_list_buffers(
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
    // `line` counts emitted lines, offset by the header at content line 0 —
    // the header occupies line 0, so the Nth emitted row lands on content
    // line N. Tracked independently from the slotmap iteration index because
    // [buffers] may be skipped without a line being emitted.
    let mut line = hume_rope::line::ContentLine::new(0);
    let mut current_content_line = hume_rope::line::ContentLine::new(1);

    for (id, buf) in ed.state.buffers.iter() {
        if buffers_view_id == Some(id) {
            continue;
        }
        line = line.advance(1);

        let cur_marker = if id == current {
            '%'
        } else if matches!(alternate, Some(alt) if id == alt) {
            '#'
        } else {
            ' '
        };
        let dirty_marker = if buf.is_dirty() { '+' } else { ' ' };

        let name = buf.display_name();
        let path = buf.display_path().unwrap_or_default();

        out.push_str(&format!(
            "{:>4}  {}{}  {:<32}  {}\n",
            line.index(),
            cur_marker,
            dirty_marker,
            name,
            path
        ));

        if id == current {
            current_content_line = line;
        }
    }

    ed.open_read_only_view("[buffers]", &out, Some(current_content_line));
    Ok(())
}

/// `:plugin-status` / `:plugins` — show all declared plugins, their load
/// state, and (for still-waiting plugins) which activation entries they are waiting on.
pub(in crate::editor) fn typed_plugin_status(
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
    ed.open_read_only_view(
        "[plugin-status]",
        &out,
        Some(hume_rope::line::ContentLine::new(0)),
    );
    Ok(())
}

// ── :split / :vsplit ──────────────────────────────────────────────────────────

/// `:split [path]` — split the focused pane, stacking the new pane below it.
///
/// With no `path`, the new pane views the same buffer as the focused one.
/// With `path`, the new pane views that file instead (opened via the usual
/// dedup-on-canonical-path rule — see [`open_path_arg`]).
pub(in crate::editor) fn typed_split(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    split_focused_pane(ed, arg, Direction::Vertical)
}

/// `:vsplit [path]` — split the focused pane side by side.
pub(in crate::editor) fn typed_vsplit(
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
        return Err(CommandError::transient(super::SPLIT_TOO_SMALL_MSG));
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

// ── :tabnew / :tabclose / :tabnext / :tabprev ─────────────────────────────────

/// `:tabnew [path]` — open a new tab. With no `path`, the new tab's pane
/// views the same buffer as the source tab's focused pane; with `path`, it
/// views that file instead (same dedup-on-canonical-path rule as
/// `:split`/`:vsplit` — see [`open_path_arg`]).
pub(in crate::editor) fn typed_tabnew(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let bid = match arg {
        Some(path) => open_path_arg(ed, path)?,
        None => ed.focused_buffer_id(),
    };
    super::open_tab(&mut ed.state, &mut ed.view, bid);
    Ok(())
}

/// `:tabclose` — close the current tab and every pane it owns. Refused with
/// a status message when it's the only tab open (fail fast, no silent
/// no-op). Typed-only — unlike `:split`'s shared core (`split_pane_onto`),
/// this has no keymap-bound/`call!`-reachable native sibling, so there is no
/// `Err` counterpart to give this refusal a `call!` boolean.
pub(in crate::editor) fn typed_tabclose(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    if ed.state.tabs.len() <= 1 {
        ed.report(Severity::Info, "cannot close the last tab page".to_string());
        return Ok(());
    }
    super::close_tab(&mut ed.state, &mut ed.view);
    Ok(())
}

/// `:tabnext` / `:tabn` — switch to the next tab in display order.
pub(in crate::editor) fn typed_tabnext(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    super::goto_tab_in_order(&mut ed.state, &mut ed.view, super::TabStep::Next);
    Ok(())
}

/// `:tabprev` / `:tabp` — switch to the previous tab in display order.
pub(in crate::editor) fn typed_tabprev(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    super::goto_tab_in_order(&mut ed.state, &mut ed.view, super::TabStep::Prev);
    Ok(())
}

/// The active theme's display name: the `theme` setting, or the label standing
/// in for the compiled-in default when it is unset. Both `:theme` and
/// `:theme-debug` open with this, and they must agree on what "no theme set"
/// is called.
fn active_theme_name(ed: &Editor) -> &str {
    if ed.state.settings.theme.is_empty() {
        super::DEFAULT_THEME_LABEL
    } else {
        &ed.state.settings.theme
    }
}

/// `:theme <name>` — load a theme by name from the theme search path.
///
/// On success the engine view's theme is replaced; the next `prepare_frame`
/// re-bakes it (see `Theme::bake_if_stale`). On failure a warning is shown and
/// the current theme is left unchanged.
pub(in crate::editor) fn typed_theme(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let Some(name) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
        let current = active_theme_name(ed);
        // NLL: `current` borrow of ed.state.settings.theme ends inside format!(), before report().
        ed.report(Severity::Info, format!("Current theme: {current}"));
        return Ok(());
    };
    crate::editor::settings::ops::apply_global(&mut ed.state, &mut ed.view, THEME_KEY, name)
        .map_err(CommandError::new)
}

/// `:theme-debug` — print what the active theme resolves for key UI surfaces.
///
/// Cursor rows report the pre-resolved style the renderer reads; every other
/// row — bracket/search match, selection, cursorline, statusline — reports its
/// resolved style plus every name on its dot-notation chain the theme defines.
pub(in crate::editor) fn typed_theme_debug(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    fn color_str(c: Option<Rgb>) -> String {
        match c {
            Some(Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            None => "-".to_owned(),
        }
    }

    /// Every name in `names` the theme actually defines, in the order given —
    /// not necessarily the resolution path: both a dot-notation lookup and a
    /// cursor ladder stop at the *first* match, so a later name here is only
    /// what it would have fallen through to next. `empty_label` covers the
    /// "theme defines none of them" case, which reads differently for an
    /// ordinary scope (`"{scope} → default"`, from `fallback_chain`, which
    /// always yields `scope` itself first) than for a cursor ladder's rung
    /// list (`"default"` — the rungs are never dot-trimmed from one shared
    /// name, so there's no single name to report finding nothing for).
    fn defined_chain<'a>(
        theme: &hume_engine::theme::Theme,
        names: impl Iterator<Item = &'a str>,
        empty_label: &str,
    ) -> String {
        let chain: Vec<&str> = names.filter(|key| theme.raw_contains(key)).collect();
        if chain.is_empty() {
            empty_label.to_owned()
        } else {
            chain.join(" → ")
        }
    }

    fn style_line(label: &str, chain: &str, style: hume_engine::types::ResolvedStyle) -> String {
        format!(
            "  {label}: chain={chain} fg={} bg={}{}",
            color_str(style.fg),
            color_str(style.bg),
            if style.modifiers.is_empty() {
                String::new()
            } else {
                format!(" modifiers={:?}", style.modifiers)
            },
        )
    }

    let theme = &ed.view.theme;
    let mut lines = vec![format!("Theme: {}", active_theme_name(ed))];

    // Cursor rows report the style the renderer will actually layer, taken
    // from the same pre-resolved `ui` fields it reads, one (secondary,
    // primary) pair per mode in `CURSOR_MODES` — the single source of the
    // mode↔scope-name pairing `Theme::compute_ui` itself resolves against.
    // Their chains aren't plain dot-notation — a primary ladder reaches a key
    // dot-trimming skips (`ui.cursor.primary.insert` never trims to
    // `ui.cursor.insert`) — so each row's chain comes from the explicit rung
    // list `cursor_ladder_ids` builds.
    // Destructured irrefutably, not zipped: a fourth entry in `CURSOR_MODES`
    // has to fail to compile here — the way it already does in
    // `Theme::compute_ui`, which destructures the same const — rather than
    // being silently dropped by `zip` stopping at the shorter side and leaving
    // the new mode missing from this listing.
    let [normal, insert, select] = hume_engine::theme::CURSOR_MODES;
    let ui = &theme.ui;
    for ((label, mode_scope, primary_mode_scope), style, primary_style) in [
        (normal, ui.cursor, ui.cursor_primary),
        (insert, ui.cursor_insert, ui.cursor_insert_primary),
        (select, ui.cursor_select, ui.cursor_select_primary),
    ] {
        let (ids, primary_ids) =
            hume_engine::theme::cursor_ladder_ids(mode_scope, primary_mode_scope);
        lines.push(style_line(
            &format!("cursor ({label})"),
            &defined_chain(theme, ids.into_iter(), "default"),
            style,
        ));
        lines.push(style_line(
            &format!("cursor primary ({label})"),
            &defined_chain(theme, primary_ids.into_iter(), "default"),
            primary_style,
        ));
    }

    // Ordinary dot-notation scopes: the chain is the whole story.
    for scope in [
        hume_engine::theme::CURSOR_MATCH,
        hume_engine::theme::CURSOR_MATCH_SEARCH,
        hume_engine::theme::ui_scopes::SELECTION,
        hume_engine::theme::ui_scopes::SELECTION_PRIMARY,
        hume_engine::theme::ui_scopes::CURSORLINE_PRIMARY,
        hume_engine::theme::ui_scopes::STATUSLINE,
    ] {
        let style = theme.resolve_by_name(hume_engine::types::Scope(scope));
        lines.push(style_line(
            scope,
            &defined_chain(
                theme,
                hume_engine::theme::fallback_chain(scope),
                &format!("{scope} → default"),
            ),
            style,
        ));
    }

    ed.report(Severity::Info, lines.join("\n"));
    Ok(())
}

pub(in crate::editor) fn typed_version(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.report(Severity::Info, format!("hume {}", crate::VERSION));
    Ok(())
}

pub(in crate::editor) fn typed_tutor(
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
/// The pre-jump position is recorded in the jump list so `Ctrl-o` returns here.
/// `:42` is accepted as shorthand (the command-mode dispatcher intercepts bare
/// digit strings and routes them here before the normal registry lookup).
pub(in crate::editor) fn typed_goto_line(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let raw = arg.ok_or_else(|| CommandError::transient(":goto requires a line number"))?;
    let n: usize = raw
        .trim()
        .parse()
        .map_err(|_| CommandError::transient(format!("invalid line number: {raw}")))?;
    let line0 = hume_rope::line::ContentLine::from_number(n)
        .ok_or_else(|| CommandError::transient(crate::cli::LINE_NUMBERS_START_AT_1))?;

    // Snapshot before moving so Ctrl-o can return here — pushed only if
    // `:goto` actually lands somewhere else (record_jump_if_moved).
    let entry = current_jump_entry(&ed.state, &ed.view);

    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    crate::editor::pane_state::park_cursor_at(
        &mut ed.state.panes.state,
        &ed.state.buffers,
        pid,
        bid,
        line0,
        hume_rope::column::GraphemeCol::new(0),
    );
    record_jump_if_moved(&mut ed.state, &ed.view, entry);
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
                return Err(CommandError::transient(format!("unknown flag: {token}")));
            }
            _ if token.starts_with('-') && token.len() > 1 => {
                for ch in token[1..].chars() {
                    match ch {
                        'r' => opts.reverse = true,
                        'i' => opts.insensitive = true,
                        _ => return Err(CommandError::transient(format!("unknown flag: -{ch}"))),
                    }
                }
            }
            _ => {
                return Err(CommandError::transient(format!(
                    "unknown argument: {token}"
                )));
            }
        }
    }
    Ok(opts)
}

/// `:sort` — sort each maximal run of adjacent lines touched by a selection,
/// keyed by the selected text on that line. Flags: `-r`/`--reverse`,
/// `-i`/`--insensitive`.
///
/// Diverges deliberately from Helix's `:sort`, which permutes text *between*
/// selection slots and leaves line boundaries untouched — this permutes the
/// lines themselves, closer to `sort -k`. See `hume_ops::edit::sort` for the
/// full semantics (grouping, numeric auto-detection, selection remapping)
/// and its rejection of Kakoune's `|sort` too. The error text below still
/// says "rows" — that's the user-facing vocabulary (see
/// `user-manual/docs/command-mode.md`), deliberately left as-is even though
/// the internal type is `SortEntry`/lines.
pub(in crate::editor) fn typed_sort(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if force {
        return Err(CommandError::transient(
            "`:sort` takes no `!` — use `-r` to reverse",
        ));
    }
    let opts = parse_sort_flags(arg)?;

    // doc_ops's read-only guard is a silent no-op — check explicitly here so
    // `:sort` on a read-only buffer (e.g. `:messages`) reports why nothing happened.
    if ed.focused_buffer_read_only() {
        return Err(CommandError::transient("Buffer is read-only"));
    }

    // Computing the sort before touching the buffer is what lets a refusal
    // (`SortRefusal`) leave the buffer untouched — an identity edit would
    // still record an undo revision and mark the buffer dirty.
    let result = sort_lines(
        ed.doc().text().clone(),
        ed.current_selections().clone(),
        opts,
    );
    let triple = match result {
        Ok(triple) => triple,
        Err(SortRefusal::NoAdjacentLines) => {
            ed.report(
                Severity::Info,
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
    super::apply_focused_edit(&mut ed.state, &ed.view, move |text, sels| {
        debug_assert_eq!(
            text.len_chars(),
            pre_len,
            "sort_lines must run against the same buffer just read"
        );
        debug_assert_eq!(
            sels, pre_sels,
            "sort_lines must run against the same selections just read"
        );
        triple
    });
    Ok(())
}

// ── :earlier / :later ─────────────────────────────────────────────────────────

/// Time-travel spec shared by `:earlier` and `:later`: a revision count or a
/// relative age (`5s`/`5m`/`1h`/`2d`). Direction comes from which command runs
/// it — `:earlier` walks back, `:later` walks forward.
enum TravelSpec {
    Steps(usize),
    Age(Duration),
}

/// Parse `:earlier`/`:later`'s single argument: a bare number is a revision
/// count, a number with an `s`/`m`/`h`/`d` suffix is a relative age.
fn parse_travel_spec(raw: &str) -> Result<TravelSpec, CommandError> {
    let invalid = || CommandError::transient(format!("invalid time-travel spec: {raw}"));
    let mut tokens = raw.split_whitespace();
    let (Some(token), None) = (tokens.next(), tokens.next()) else {
        return Err(invalid());
    };
    // `as_bytes().last()` (not `.chars().last()`) makes "the suffix is
    // ASCII, so one byte" structural rather than asserted: any non-ASCII
    // final byte falls through to the bare-number arm below and fails to
    // parse as a digit, the same as any other non-digit trailing character.
    let (digits, mult) = match token.as_bytes().last() {
        Some(b's') => (&token[..token.len() - 1], Some(1)),
        Some(b'm') => (&token[..token.len() - 1], Some(60)),
        Some(b'h') => (&token[..token.len() - 1], Some(3_600)),
        Some(b'd') => (&token[..token.len() - 1], Some(86_400)),
        Some(c) if c.is_ascii_alphabetic() => return Err(invalid()),
        _ => (token, None),
    };
    let n: usize = digits.parse().map_err(|_| invalid())?;
    Ok(match mult {
        Some(mult) => TravelSpec::Age(Duration::from_secs((n as u64).saturating_mul(mult))),
        None => TravelSpec::Steps(n),
    })
}

/// `cmd_undo`/`cmd_redo` themselves — the same function `u`/`Ctrl-r` dispatch
/// to, `refuse_if_read_only` guard and `history_step`'s own exhaustion report
/// included, rather than a second hand-copied `(walk, exhausted_msg)` pair.
type TravelStepFn =
    fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>;

/// `Buffer::undo_steps_older_than`/`redo_steps_newer_than` — resolves a
/// `TravelSpec::Age` to the step count [`TravelStepFn`] takes. `Err(n)` means
/// the age is unsatisfiable; see `History::undo_steps_older_than`'s own doc.
type TravelResolveAgeFn = fn(&Buffer, Duration) -> Result<usize, usize>;

/// Direction of `:earlier`/`:later` travel — everything the two commands
/// differ in, so the shared `travel` core below stays straight-line: a
/// single match binding the three per-direction values, rather than three
/// separate one-caller-each methods.
#[derive(Clone, Copy)]
enum TravelDir {
    Earlier,
    Later,
}

impl TravelDir {
    /// `(name, step, resolve_age)` — `name` is `:earlier`/`:later`'s own
    /// command word, for the `!`-rejection error message below.
    fn parts(self) -> (&'static str, TravelStepFn, TravelResolveAgeFn) {
        match self {
            TravelDir::Earlier => ("earlier", cmd_undo, Buffer::undo_steps_older_than),
            TravelDir::Later => ("later", cmd_redo, Buffer::redo_steps_newer_than),
        }
    }
}

/// Shared `:earlier`/`:later` body: resolve the spec to a step count, then
/// travel through the same `cmd_undo`/`cmd_redo` `u`/`Ctrl-r` dispatch to, so
/// the whole walk (however many revisions an age spans) propagates to panes,
/// tree-sitter, LSP, decorations, and jumps as one composed change — see
/// `Buffer::apply_transactions` — a read-only buffer is refused identically
/// on both paths, and exhaustion (an age older than the root, or newer than
/// the tip) is reported exactly once, by `history_step` itself.
fn travel(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
    dir: TravelDir,
) -> Result<(), CommandError> {
    let (name, step, resolve_age) = dir.parts();
    if force {
        return Err(CommandError::transient(format!("`:{name}` takes no `!`")));
    }
    let spec = parse_travel_spec(arg.unwrap_or("1"))?;
    // Checked before resolving `spec` into a step count: an age spec walks
    // the buffer's whole history, work `step` below would refuse anyway on
    // a read-only buffer.
    if super::refuse_if_read_only(&mut ed.state, &ed.view) {
        return Ok(());
    }
    let steps = match spec {
        TravelSpec::Steps(n) => n,
        TravelSpec::Age(age) => {
            let buf = ed.focused_buffer_id();
            // Exhaustion is reported by `history_step`'s own
            // `taken < requested` comparison, not a second time here —
            // asking for one step more than exists is what makes that
            // comparison fire.
            resolve_age(ed.state.buffers.get(buf), age).unwrap_or_else(|avail| avail + 1)
        }
    };
    step(&mut ed.state, &mut ed.view, steps, MotionMode::Move)
}

/// `:earlier [N|age]` — step back `N` revisions (default 1), or back to the
/// state as of `age` ago (`:earlier 5m`). Clamps at the root with an Info report.
pub(in crate::editor) fn typed_earlier(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    travel(ed, arg, force, TravelDir::Earlier)
}

/// `:later [N|age]` — the mirror of `:earlier`, forward along the
/// most-recent-child chain (the same path redo takes). An age means the state
/// as of that age ago, reached by stepping forward while the next revision is
/// still older than it.
pub(in crate::editor) fn typed_later(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    travel(ed, arg, force, TravelDir::Later)
}
