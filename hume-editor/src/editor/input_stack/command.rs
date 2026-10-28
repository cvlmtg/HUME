//! The `Command` layer: the `:`-prompt minibuffer mode.

use hume_engine::pipeline::{EngineView, ScreenLayout};
use hume_engine::types::EditorMode;

use super::super::dispatch::unknown_command_text;
use super::super::error::CommandError;
use super::super::minibuf::history::{HistoryDir, HistoryKind};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent, PromptCompletion};
use super::super::registry::{ArgExpansion, Takes, TypedBody};
use super::super::replay::WaitingInput;
use super::super::{Editor, EditorState, commands};
use super::paste::PastedText;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, Removal};

pub(in crate::editor) struct CommandLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    /// The check this line passed as it opened.
    _input: WaitingInput,
}

impl CommandLayer {
    pub(in crate::editor) fn new(input: WaitingInput) -> Self {
        Self {
            minibuf: MiniBuffer::new(":", Some(PromptCompletion::CommandLine)),
            _input: input,
        }
    }
}

impl Layer for CommandLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        command_input
    }
    fn paste_handler(&self) -> PasteHandler {
        command_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Command)
    }
    fn tear_down(self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.history.begin_session_all();
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

impl Editor {
    /// Write the open minibuffer completion session into the shared
    /// `PopupState` Arc: same widget as [`Self::sync_menu_view`]/
    /// [`Self::sync_completion_menu_view`] (unwrapped rows, selected-row
    /// styling), but anchored at the command line in the statusline row
    /// rather than a buffer cursor, over any bottom band. `Minibuf`-target
    /// only; a `Buffer`-target session renders through
    /// [`Self::sync_completion_menu_view`] instead, into its own slot.
    ///
    /// Called from `prepare_frame`'s overlay-sync step with the frame's
    /// layout. Needs only that layout, unlike its cursor-anchored siblings,
    /// which need the current frame's scroll result too.
    pub(in crate::editor) fn sync_minibuf_completion_view(&self, screen: &ScreenLayout) {
        let is_open = self.state.input.minibuf_completion().is_some();
        // Skip the write-lock when both sides are already None: common case
        // while no popup is open.
        if !is_open && self.state.views.minibuf_completion.read().is_none() {
            return;
        }
        let area = screen.area_above_statusline();
        let view = (|| -> Option<hume_ui::popup::PopupState> {
            let session = self.state.input.minibuf_completion()?;
            let anchor_byte = session.menu_anchor_byte()?;
            let selected = session.selected();
            let anchor_x = self
                .state
                .input
                .minibuf()
                .map(|mb| mb.cursor_x_at(anchor_byte))
                .unwrap_or(0);
            let window = hume_ui::popup::menu_window(session.len(), selected, area);
            let rows = session.rows_in(window.range.clone());
            // Anchoring at the statusline row, one past `area`'s bottom, is
            // what drives `resolve_popup_geometry` into its flip-above
            // branch (`space_below` saturates to 0 there), landing the box on
            // the rows just above the command line. The -1 pulls the frame
            // left so the first label column sits under the token in the
            // input.
            Some(hume_ui::popup::resolve_menu(
                &rows,
                window,
                hume_ui::popup::PopupPlacement {
                    anchor: (anchor_x.saturating_sub(1), area.bottom()),
                    pane_rect: area,
                    content_width: area.width,
                },
                self.state.settings.popup_border,
            ))
        })();
        self.state.views.minibuf_completion.set(view);
    }
}

pub(in crate::editor) fn command_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::command_line_input(ed, r, ev) {
        handle_command_event(ed, r, event);
    }
}

fn command_paste(
    state: &mut EditorState,
    _view: &EngineView,
    _r: LayerRef,
    text: PastedText,
) -> Result<(), CommandError> {
    if minibuf::paste_into_minibuf(state, &text) {
        command_line_edited(state);
    }
    Ok(())
}

/// Demotes any active history recall back to scratch: what an edit or a
/// cursor move on the `:` line does beyond changing it.
fn command_line_edited(state: &mut EditorState) {
    state
        .history
        .get_mut(HistoryKind::Command)
        .demote_to_scratch();
}

fn handle_command_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    // No completion-dismiss calls in this function, on any arm: a
    // `MinibufCompletionLayer` sits *above* `Command` whenever a popup is open, and
    // `completion_input_minibuf` (`input_stack/completion.rs`) dismisses it
    // before falling through to whatever runs here. By the time any of
    // these arms sees an event, either no popup was open, or one already
    // was and just closed. One place enforces "any non-Tab key dismisses",
    // not one check repeated at every event site here.
    match event {
        MiniBufferEvent::Cancel
        | MiniBufferEvent::ConfirmEmpty
        | MiniBufferEvent::BackspaceOnEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(_) => {
            // Record into history and extract the input before
            // truncating: the `Command` layer (and the minibuf it
            // owns) is gone by the time `execute_command_line` runs, so the
            // string has to be taken out first, per truncate-before-
            // execute: the body below runs with the mode layer already
            // back at `Base`, so a `:cmd` that enters Insert stays in
            // Insert instead of being stomped back to Normal, and a
            // body calling `(prompt! …)` pushes `Prompt` on a clean
            // stack with no special case needed.
            let raw = ed
                .state
                .input
                .minibuf()
                .map(|mb| mb.input.clone())
                .unwrap_or_default();
            ed.state
                .history
                .get_mut(HistoryKind::Command)
                .push(raw.clone());
            ed.state.truncate_layers(&ed.view, r);
            let _ = execute_command_line(ed, &raw);
        }
        // An edit, cursor move, or Backspace that clears to empty demotes
        // any active history recall back to scratch. EmptiedByBackspace
        // keeps the minibuffer open (showing just the prompt) so a second
        // Backspace is needed to dismiss. Avoids accidental closure when
        // the user deletes a one-char typo.
        MiniBufferEvent::EmptiedByBackspace
        | MiniBufferEvent::Edited
        | MiniBufferEvent::CursorMoved => command_line_edited(&mut ed.state),
        MiniBufferEvent::HistoryPrev => {
            minibuf::recall_history(ed, HistoryKind::Command, HistoryDir::Prev);
        }
        MiniBufferEvent::HistoryNext => {
            minibuf::recall_history(ed, HistoryKind::Command, HistoryDir::Next);
        }
        MiniBufferEvent::Ignored => {}
    }
}

/// Run the `:` line `raw` (its text without the leading `:`) and report its
/// error, which is also returned. At Confirm the `Command` layer (and its
/// minibuf) is already gone by the time this runs, per
/// truncate-before-execute.
fn execute_command_line(ed: &mut Editor, raw: &str) -> Result<(), CommandError> {
    run_command_line(ed, raw.trim()).inspect_err(|e| ed.report_error(e))
}

#[cfg(test)]
impl Editor {
    /// Run the `:` line `cmd_with_arg` (e.g. `"bd"`, `"e! path"`), with
    /// `extra_arg` appended after a space when given, as Confirm on the `:`
    /// line runs it. Reports an error as `:` does, and also returns it.
    pub(in crate::editor) fn execute_typed(
        &mut self,
        cmd_with_arg: &str,
        extra_arg: Option<&str>,
    ) -> Result<(), CommandError> {
        let line = match extra_arg {
            Some(arg) => format!("{cmd_with_arg} {arg}"),
            None => cmd_with_arg.to_owned(),
        };
        execute_command_line(self, &line)
    }
}

/// Run a trimmed `:` line, returning its error unreported.
fn run_command_line(ed: &mut Editor, input: &str) -> Result<(), CommandError> {
    let (cmd, force, arg) = parse_typed_command(input);

    // Bare line number `:42`: shorthand for `:goto 42`.
    // The parser leaves `cmd = ""` for digit-only input (a name can't start
    // with a digit, so `:42` → cmd="" arg=Some("42")).
    if let Some(line) = arg.filter(|a| {
        cmd.is_empty() && !force && !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit())
    }) {
        let fp = commands::FocusedPane::current(&ed.view);
        return commands::typed_goto_line(ed, fp, line);
    }

    // `:` resolves only typed commands: an editor (key-bindable) command's
    // name is unreachable here; see `registry/mod.rs`'s module doc.
    let Some(tc) = ed.state.config.registry.get_typed(cmd) else {
        return Err(ed.unknown_command(cmd, unknown_command_text(cmd)));
    };
    let name = tc.name.clone();
    let expansion = tc.arg_expansion;
    let expand = |ed: &Editor, a: &str| match expansion {
        ArgExpansion::Paths => expand_command_arg(ed, a),
        ArgExpansion::Verbatim => Ok(a.to_owned()),
    };

    let body = match &tc.body {
        TypedBody::Native(native) => {
            let native = *native;
            let input = native
                .takes()
                .accept(&name, arg, force, |a| expand(ed, a))?;
            let fp = commands::FocusedPane::current(&ed.view);
            return native.run(ed, fp, &input);
        }
        TypedBody::Steel(body) => body.clone(),
        // Activated before the refusal, which reads the lambda's arity.
        TypedBody::Lazy(plugin) => {
            let plugin = plugin.clone();
            ed.resolve_lazy_typed(&plugin, &name)?
        }
    };
    let input = Takes::of_steel(&body).accept(&name, arg, force, |a| expand(ed, a))?;
    ed.run_typed_steel_command(&name, body, &input)
}

// ── Typed-command helpers ─────────────────────────────────────────────────────

/// The command name at the start of `input` (the grammar typed commands
/// are registered against, [`hume_scripting::command_name`]), one
/// optional trailing `!` consumed as `force`, and the byte offset one past
/// both. Shared by [`parse_typed_command`] (which also needs the trimmed
/// argument past that point) and `orchestrate.rs`'s `target_command_name`
/// (which only needs the offset, to test whether the cursor has moved past
/// the name), so completion never disagrees with what `run_command_line`
/// would actually run.
pub(in crate::editor) fn scan_command_name(input: &str) -> (&str, bool, usize) {
    let name = hume_scripting::command_name::scan(input);
    let force = input[name.len()..].starts_with('!');
    (name, force, name.len() + usize::from(force))
}

/// Parse a typed-command string into `(cmd, force, arg)`: everything past
/// [`scan_command_name`]'s own offset is the argument (whitespace-trimmed).
/// Matches Vim's ex-parser so `:b#`, `:e!/path`, `:list-buffers`, and
/// `:w foo.txt` all parse correctly.
pub(in crate::editor) fn parse_typed_command(input: &str) -> (&str, bool, Option<&str>) {
    let (cmd, force, cmd_end) = scan_command_name(input);
    let rest = input[cmd_end..].trim();
    let arg = if rest.is_empty() { None } else { Some(rest) };
    (cmd, force, arg)
}

/// Expand `%` → focused buffer's path and `#` → alternate buffer's path in a
/// typed-command argument.
///
/// Only whole tokens (separated by ASCII spaces) are substituted, so filenames
/// containing `%` or `#` as part of a longer word pass through unchanged.
/// Spacing is preserved; returns a user-facing error on the first unresolved token.
///
/// Expands to the canonical `path`, not `display_path`: the result feeds
/// straight back into path resolution, which doesn't `~`-expand.
fn expand_command_arg(ed: &Editor, arg: &str) -> Result<String, CommandError> {
    let mut out = String::with_capacity(arg.len());
    for (i, token) in arg.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match token {
            "%" => {
                let path = ed
                    .doc()
                    .path()
                    .ok_or_else(|| CommandError::transient("No file name"))?;
                out.push_str(&path.display().to_string());
            }
            "#" => {
                let alt_id = ed
                    .state
                    .buffers
                    .second_most_recent()
                    .ok_or_else(|| CommandError::transient("No alternate buffer"))?;
                let alt_path =
                    ed.state.buffers.get(alt_id).path().ok_or_else(|| {
                        CommandError::transient("Alternate buffer has no file name")
                    })?;
                out.push_str(&alt_path.display().to_string());
            }
            other => out.push_str(other),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
