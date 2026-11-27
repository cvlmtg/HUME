use std::time::Duration;

use hume_core::{BufferId, Severity, TruncateEnd};
use hume_engine::pipeline::{Cycle, Direction, EngineView};
use hume_engine::types::ResolvedStyle;
use hume_grid::Rgb;
use hume_platform::path::AbsolutePath;
use hume_rope::line::ContentLine;
use hume_rope::width::{chrome_width, truncate_marked};

use super::super::Editor;
use super::super::EditorState;
use super::{CommandPane, FocusedPane, redo_steps, undo_steps};
use crate::editor::buffer::{Buffer, EditorView, ViewLabel, lifecycle};
use crate::editor::error::CommandError;
use crate::editor::input_stack::DrawerLayer;
use crate::editor::registry::ArgError;
use crate::editor::settings::{THEME, TableKey};
use crate::editor::syntax::{SyntaxUse, ensure_syntax_current};
use hume_ops::edit::{SortOpts, SortRefusal, sort_lines};

// ── Message log ──────────────────────────────────────────────────────────────

/// `:messages`: open the message log in a read-only buffer.
///
/// Displays the entries logged during the session at or above
/// `messages-min-severity`; `Editor::follow_message_log` keeps it current
/// while shown. Cursor starts at the last entry (most recent); every other
/// pane showing the buffer keeps its cursor. The entries count as seen once
/// a frame draws the end. Dismiss with `:bd` or switch away with `:b#`.
pub(in crate::editor) fn typed_messages(
    ed: &mut Editor,
    fp: FocusedPane,
) -> Result<(), CommandError> {
    if !ed
        .state
        .message_log
        .shows_any(ed.state.settings.messages_min_severity)
    {
        ed.report(Severity::Info, "No messages");
        return Ok(());
    }
    let bid = ed.sync_messages_view();
    // None parks at the bottom (most recent entry).
    lifecycle::present_view(&mut ed.state, &mut ed.view, fp.pane(), bid, None);
    Ok(())
}

/// `:ls` / `:list-buffers`: list every open buffer in the bottom drawer.
///
/// Each row shows: 1-based index (the one `:b <n>` takes), current (`%`) /
/// alternate (`#`) marker, dirty (`+`) flag, short name, and home-shortened
/// absolute path. The focused buffer's row is selected; `Enter` on a row
/// switches to that buffer.
pub(in crate::editor) fn typed_list_buffers(
    ed: &mut Editor,
    fp: FocusedPane,
) -> Result<(), CommandError> {
    let bid = fp.bid(&ed.view);
    let drawer = DrawerLayer::buffer_list(&ed.state, bid, &mut ed.view.registry);
    ed.state.open_drawer(&ed.view, drawer);
    Ok(())
}

/// `:plugin-status` / `:plugins`: show all declared plugins, their load
/// state, and (for still-waiting plugins) which activation entries they are waiting on.
pub(in crate::editor) fn typed_plugin_status(
    ed: &mut Editor,
    fp: FocusedPane,
) -> Result<(), CommandError> {
    let out = if let Some(host) = ed.scripting.as_ref() {
        host.lazy_status_string(&ed.state.config.registry.lazy_stubs())
    } else {
        ed.report(Severity::Info, "Scripting disabled");
        return Ok(());
    };
    if out.is_empty() {
        ed.report(Severity::Info, "No plugins declared");
        return Ok(());
    }
    ed.open_read_only_view(
        fp,
        &ViewLabel::editor(EditorView::PluginStatus),
        &out,
        Some(hume_rope::line::ContentLine::new(0)),
    );
    Ok(())
}

// ── :split / :vsplit ──────────────────────────────────────────────────────────

/// `:split [path]`: split the focused pane, stacking the new pane below it.
///
/// With no `path`, the new pane views the same buffer as the focused one.
/// With `path`, the new pane views that file instead (opened via the usual
/// dedup-on-canonical-path rule, see [`open_path_arg`]).
pub(in crate::editor) fn typed_split(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    split_focused_pane(ed, fp, arg, Direction::Vertical)
}

/// `:vsplit [path]`: split the focused pane side by side.
pub(in crate::editor) fn typed_vsplit(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    split_focused_pane(ed, fp, arg, Direction::Horizontal)
}

/// Split the focused pane and move focus to the new pane.
///
/// `direction` is the engine's split axis, which is *inverted* from the Vim
/// command names: `Direction::Vertical` divides height (stacked panes, what
/// `:split` means), `Direction::Horizontal` divides width (side by side,
/// `:vsplit`); see `hume_engine::pipeline::Split::direction`. A split that
/// does not fit opens no path argument.
fn split_focused_pane(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    direction: Direction,
) -> Result<(), CommandError> {
    super::split_pane_onto(
        &mut ed.state,
        &mut ed.view,
        fp,
        direction,
        |state, view| match arg {
            Some(path) => open_path_arg(state, view, path),
            None => Ok(fp.bid(view)),
        },
    )
}

/// Resolve a `:split`/`:vsplit` path argument to a `BufferId`, opening the
/// file if it isn't already open.
fn open_path_arg(
    state: &mut EditorState,
    view: &mut EngineView,
    path_str: &str,
) -> Result<BufferId, CommandError> {
    let (bid, _) = AbsolutePath::from_typed(path_str, &state.cwd)
        .and_then(|typed| {
            crate::editor::buffer::lifecycle::open_or_dedup_and_notify(view, state, typed)
        })
        .map_err(|e| CommandError::new(format!("{path_str}: {e}")))?;
    Ok(bid)
}

// ── :tabnew / :tabclose / :tabnext / :tabprev ─────────────────────────────────

/// `:tabnew [path]`: open a new tab. With no `path`, the new tab's pane
/// views the same buffer as the source tab's focused pane; with `path`, it
/// views that file instead (same dedup-on-canonical-path rule as
/// `:split`/`:vsplit`, see [`open_path_arg`]).
pub(in crate::editor) fn typed_tabnew(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    let bid = match arg {
        Some(path) => open_path_arg(&mut ed.state, &mut ed.view, path)?,
        None => fp.bid(&ed.view),
    };
    super::pane::open_pane_as_new_tab(&mut ed.state, &mut ed.view, bid);
    Ok(())
}

/// `:tabclose`: close the current tab and every pane it owns. Refused with
/// a status message when it's the only tab open (fail fast, no silent
/// no-op). Typed-only: unlike `:split`'s shared core (`split_pane_onto`),
/// this has no keymap-bound/`call!`-reachable native sibling, so there is no
/// `Err` counterpart to give the last-tab refusal a `call!` boolean.
/// Refused without `!` while a pane in it started a pipe run still in
/// flight, which closing would stop.
pub(in crate::editor) fn typed_tabclose(
    ed: &mut Editor,
    _fp: FocusedPane,
    force: bool,
) -> Result<(), CommandError> {
    match ed.run_stopping_pipes(force, |ed| super::close_tab(&mut ed.state, &mut ed.view)) {
        Ok(()) => Ok(()),
        Err(super::CloseRefused::Last) => {
            ed.report(Severity::Info, "cannot close the last tab page");
            Ok(())
        }
        Err(super::CloseRefused::PipeRunning(running)) => Err(super::pipe_running_refusal(
            &ed.state,
            &ed.view,
            running,
            ":tabclose!",
        )),
    }
}

/// `:tabnext` / `:tabn`: switch to the next tab in display order.
pub(in crate::editor) fn typed_tabnext(
    ed: &mut Editor,
    _fp: FocusedPane,
) -> Result<(), CommandError> {
    super::goto_tab_in_order(&mut ed.state, &mut ed.view, Cycle::Next);
    Ok(())
}

/// `:tabprev` / `:tabp`: switch to the previous tab in display order.
pub(in crate::editor) fn typed_tabprev(
    ed: &mut Editor,
    _fp: FocusedPane,
) -> Result<(), CommandError> {
    super::goto_tab_in_order(&mut ed.state, &mut ed.view, Cycle::Prev);
    Ok(())
}

/// The active theme's display name: the `theme` setting, or the label standing
/// in for the compiled-in default when it is unset. `:theme` with no argument
/// reports this.
fn active_theme_name(ed: &Editor) -> &str {
    if ed.state.settings.theme.is_empty() {
        super::FALLBACK_THEME_LABEL
    } else {
        &ed.state.settings.theme
    }
}

/// `:theme <name>`: load a theme by name from the theme search path.
///
/// On success the engine view's theme is replaced; the next `prepare_frame`
/// re-bakes it (see `Theme::bake_if_stale`). On failure a warning is shown and
/// the current theme is left unchanged.
pub(in crate::editor) fn typed_theme(
    ed: &mut Editor,
    _fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    let Some(name) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
        let current = active_theme_name(ed);
        ed.report(Severity::Info, format!("Current theme: {current}"));
        return Ok(());
    };
    let name = hume_core::OptionInput::Text(name.to_string());
    crate::editor::settings::ops::apply_global(
        &mut ed.state,
        &mut ed.view,
        TableKey::Global(THEME),
        &name,
        hume_core::ConfigOrigin::User,
    )
    .map_err(CommandError::new)
}

/// `:theme-debug`: name the theme key that colors the text under the cursor.
///
/// One statusline line, strongest highlight first. Each tier prints its
/// fallback chain up to the first key with an entry, then that entry's style
/// as `<fg>/<bg>[ modifiers][ underline=<shape>[/#rrggbb]]`; `-` is a channel
/// the key leaves unset. A tier whose chain has no entry prints the whole
/// chain and `(no entry)`. Entries are cut to the statusline's text area,
/// marked with `…`, so the weakest tiers are the ones lost. Only highlight
/// tiers are reported; the cursor, selection and cursorline paint are not.
pub(in crate::editor) fn typed_theme_debug(
    ed: &mut Editor,
    fp: FocusedPane,
) -> Result<(), CommandError> {
    const SEP: &str = " | ";

    fn color_str(c: Option<Rgb>) -> String {
        match c {
            Some(Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            None => "-".to_owned(),
        }
    }

    fn style_str(style: ResolvedStyle) -> String {
        let mut text = format!("{}/{}", color_str(style.fg), color_str(style.bg));
        let modifiers: Vec<&str> = hume_engine::theme::modifier_names(style.modifiers).collect();
        if !modifiers.is_empty() {
            text.push(' ');
            text.push_str(&modifiers.join(","));
        }
        if let Some(shape) = hume_engine::theme::underline_name(style.underline) {
            text.push_str(" underline=");
            text.push_str(shape);
            if let Some(color) = style.underline_color {
                text.push('/');
                text.push_str(&color_str(Some(color)));
            }
        }
        text
    }

    /// `scope`'s fallback chain up to and including `key`, most specific
    /// first: the keys a theme edit may touch, ending at the one that applies.
    fn chain_through<'a>(scope: &'a str, key: &'a str) -> impl Iterator<Item = &'a str> {
        hume_engine::theme::fallback_chain(scope)
            .take_while(move |name| *name != key)
            .chain(std::iter::once(key))
    }

    let bid = fp.bid(&ed.view);
    let pane = fp.pane().pid();
    // A position read goes through the layers, so a reparse the worker has
    // pending has to land before the spans it would answer with are read.
    ensure_syntax_current(
        &mut ed.state,
        &mut ed.view.registry,
        bid,
        SyntaxUse::Highlights,
    );
    let active = {
        let buf = ed.state.buffers.get(bid);
        let text = buf.text();
        let head = super::pane_view(&ed.state, &ed.view, fp.pane())
            .primary()
            .head();
        let (ropey_line, byte) = hume_rope::lines::char_to_line_byte(text.rope(), head.offset());
        // The head is a real cluster, never the phantom trailing line.
        let line = ContentLine::new(ropey_line.index());
        hume_engine::style::scopes_at_byte(
            line,
            byte,
            buf.syntax_source(),
            &ed.view.panes[pane].providers,
            text.rope(),
        )
    };
    if active.is_empty() {
        ed.report(Severity::Info, "no highlight scope at cursor");
        return Ok(());
    }

    // The status message fills the statusline's text area, which the renderer
    // clips at the same right edge.
    let statusline = ed.state.screen_layout(&ed.view).statusline_area();
    let capacity = usize::from(crate::statusline::text_area(statusline).width);
    let mut msg = String::new();
    for (_, id) in active.into_iter().rev() {
        let name = ed.view.registry.name_of(id);
        let entry = match ed.view.theme.matching_entry(name) {
            Some((key, style)) => format!(
                "{} {}",
                chain_through(name, key).collect::<Vec<_>>().join(" → "),
                style_str(style)
            ),
            None => format!(
                "{} (no entry)",
                hume_engine::theme::fallback_chain(name)
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
        };
        let sep = if msg.is_empty() { 0 } else { chrome_width(SEP) };
        let remaining = capacity.saturating_sub(chrome_width(&msg) + sep);
        let fitted = truncate_marked(&entry, remaining, TruncateEnd::Tail);
        if fitted.is_empty() {
            break;
        }
        if !msg.is_empty() {
            msg.push_str(SEP);
        }
        let cut = fitted.len() < entry.len();
        msg.push_str(&fitted);
        if cut {
            break;
        }
    }
    ed.report(Severity::Info, msg);
    Ok(())
}

pub(in crate::editor) fn typed_version(
    ed: &mut Editor,
    _fp: FocusedPane,
) -> Result<(), CommandError> {
    ed.report(
        Severity::Info,
        format!("hume {}", hume_platform::version::DISPLAY),
    );
    Ok(())
}

pub(in crate::editor) fn typed_tutor(ed: &mut Editor, fp: FocusedPane) -> Result<(), CommandError> {
    // Resolve the install source. Fail fast on missing runtime or file.
    let Some(runtime) = ed.state.dirs.runtime.as_deref() else {
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
    let Some(tmp_base) = ed.state.dirs.tmp.as_deref() else {
        return Err(CommandError::new("temp directory not available"));
    };
    let tmp_dir = tmp_base.join(format!("hume-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| CommandError::new(format!("could not create tutor tmp dir: {e}")))?;
    let canonical_tmp = AbsolutePath::canonicalize(&tmp_dir)
        .and_then(|dir| AbsolutePath::from_path(std::path::Path::new("tutor.rst"), &dir))
        .map_err(|e| CommandError::new(format!("could not canonicalize tutor tmp dir: {e}")))?;

    // If a buffer is already open at the tmp path, switch. No re-copy, so that
    // unsaved in-memory edits are preserved.
    if let Some(bid) = ed.state.buffers.find_by_path(&canonical_tmp) {
        ed.enter_buffer(fp, bid);
        return Ok(());
    }

    // No live buffer at tmp. Copy fresh source content (overwrites any stale
    // file from a prior `:bd!`), then open the copy.
    std::fs::copy(&source, &canonical_tmp)
        .map_err(|e| CommandError::new(format!("could not copy tutor.rst to tmp: {e}")))?;
    let (bid, _) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
        &mut ed.view,
        &mut ed.state,
        canonical_tmp,
    )
    .map_err(|e| CommandError::new(format!("could not open tutor copy: {e}")))?;
    ed.enter_buffer(fp, bid);
    Ok(())
}

// ── Go-to-line ────────────────────────────────────────────────────────────────

/// `:goto N`: jump to 1-based line `N`, clamped to the last content line.
///
/// The pre-jump position is recorded in the jump list so `Ctrl-o` returns here.
/// `:42` runs as `:goto 42`.
pub(in crate::editor) fn typed_goto_line(
    ed: &mut Editor,
    fp: FocusedPane,
    raw: &str,
) -> Result<(), ArgError> {
    let n: usize = raw
        .trim()
        .parse()
        .map_err(|_| CommandError::transient(format!("invalid line number: {raw}")))?;
    let line0 = hume_rope::line::ContentLine::from_number(n)
        .ok_or_else(|| CommandError::transient(crate::cli::LINE_NUMBERS_START_AT_1))?;

    let t = fp.pane();
    crate::editor::pane_state::park_cursor_at(
        &mut ed.state.panes.entries,
        &ed.state.buffers,
        &ed.view.panes,
        t.pid(),
        t.bid(&ed.view),
        line0,
        hume_rope::column::GraphemeCol::new(0),
    );
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
            // recognize. Report it as a flag, not a positional argument.
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

/// `:sort`: sort each maximal run of adjacent lines touched by a selection,
/// keyed by the selected text on that line. Flags: `-r`/`--reverse`,
/// `-i`/`--insensitive`.
///
/// Diverges from Helix's `:sort`, which permutes text *between*
/// selection slots and leaves line boundaries untouched. This permutes the
/// lines themselves, closer to `sort -k`. See `hume_ops::edit::sort` for the
/// full semantics (grouping, numeric auto-detection, selection remapping)
/// and its rejection of Kakoune's `|sort` too. The error text below still
/// says "rows": that's the user-facing vocabulary (see
/// `user-manual/docs/command-mode.md`), kept even though
/// the internal type is `SortEntry`/lines.
pub(in crate::editor) fn typed_sort(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    let opts = parse_sort_flags(arg)?;

    let t = fp.pane();
    super::ensure_writable(&mut ed.state, &ed.view, t)?;

    // Computing the sort before touching the buffer is what lets a refusal
    // (`SortRefusal`) leave the buffer untouched: an identity edit would
    // still record an undo revision and mark the buffer dirty.
    let result = sort_lines(
        hume_editing::state::EditState::bind(
            super::doc(&ed.state, &ed.view, t).text().clone(),
            super::pane_selections(&ed.state, &ed.view, t).clone(),
        ),
        opts,
    );
    let edited = match result {
        Ok(edited) => edited,
        Err(SortRefusal::NoAdjacentLines) => {
            ed.report(Severity::Info, "sort needs at least two adjacent rows");
            return Ok(());
        }
        Err(SortRefusal::AlreadySorted) => {
            ed.report(Severity::Info, "already sorted");
            return Ok(());
        }
    };

    super::apply_pane_edit(&mut ed.state, &ed.view, t, move |_| edited)?;
    Ok(())
}

// ── :earlier / :later ─────────────────────────────────────────────────────────

/// Time-travel spec shared by `:earlier` and `:later`: a revision count or a
/// relative age (`5s`/`5m`/`1h`/`2d`). Direction comes from which command runs
/// it: `:earlier` walks back, `:later` walks forward.
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

/// [`undo_steps`]/[`redo_steps`] themselves, the same walk `u`/`Ctrl-r`
/// dispatch to, read-only refusal and exhaustion report included, rather
/// than a second hand-copied `(walk, exhausted_msg)` pair.
type TravelStepFn =
    fn(&mut EditorState, &EngineView, CommandPane, usize) -> Result<(), CommandError>;

/// `Buffer::undo_steps_older_than`/`redo_steps_newer_than`: resolves a
/// `TravelSpec::Age` to the step count [`TravelStepFn`] takes. `Err(n)` means
/// the age is unsatisfiable; see `History::undo_steps_older_than`'s own doc.
type TravelResolveAgeFn = fn(&Buffer, Duration) -> Result<usize, usize>;

/// Shared `:earlier`/`:later` body: resolve the spec to a step count, then
/// travel through the same `undo_steps`/`redo_steps` `u`/`Ctrl-r` dispatch to, so
/// the whole walk (however many revisions an age spans) propagates to panes,
/// tree-sitter, LSP, decorations, and jumps as one composed change (see
/// `Buffer::apply_transactions`), a read-only buffer is refused identically
/// on both paths, and exhaustion (an age older than the root, or newer than
/// the tip) is reported exactly once, by `history_step` itself.
fn travel(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    step: TravelStepFn,
    resolve_age: TravelResolveAgeFn,
) -> Result<(), CommandError> {
    let spec = parse_travel_spec(arg.unwrap_or("1"))?;
    let t = fp.pane();
    let steps = match spec {
        TravelSpec::Steps(n) => n,
        TravelSpec::Age(age) => {
            let buf = t.bid(&ed.view);
            // Exhaustion is reported by `history_step`'s own
            // `taken < requested` comparison, not a second time here:
            // asking for one step more than exists is what makes that
            // comparison fire.
            resolve_age(ed.state.buffers.get(buf), age).unwrap_or_else(|avail| avail + 1)
        }
    };
    step(&mut ed.state, &ed.view, t, steps)
}

/// `:earlier [N|age]`: step back `N` revisions (default 1), or back to the
/// state as of `age` ago (`:earlier 5m`). Clamps at the root with an Info report.
pub(in crate::editor) fn typed_earlier(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    travel(ed, fp, arg, undo_steps, Buffer::undo_steps_older_than)
}

/// `:later [N|age]`: the mirror of `:earlier`, forward along the
/// most-recent-child chain (the same path redo takes). An age means the state
/// as of that age ago, reached by stepping forward while the next revision is
/// still older than it.
pub(in crate::editor) fn typed_later(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
) -> Result<(), CommandError> {
    travel(ed, fp, arg, redo_steps, Buffer::redo_steps_newer_than)
}
