//! `EditorHostImpl`'s inlay hints, signs, virtual lines, extra highlights,
//! EOL text, statusline text, and the diagnostic pull/count reads.
//!
//! The five position-validation free functions below are `pub(super)` where
//! `host_impl/tests.rs` (a sibling of this module, not a descendant) needs
//! them: the same reachable set (`host_impl` and its descendants) they had
//! as private items directly in `host_impl.rs`, just spelled differently now
//! that a caller of them lives outside this module.

use hume_core::{BufferId, DiagSeverity, Side};
use hume_rope::column::ByteCol;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::EditorState;
use crate::statusline::StatusElement;

use super::EditorHostImpl;
use hume_scripting::host::DecorationHost;

impl<'a> DecorationHost for EditorHostImpl<'a> {
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, Side)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid)?;
        let entries = hints
            .into_iter()
            .map(|(pos, hint_text, side)| {
                let pos = validate_offset(text, pos, side)?;
                Ok(hume_decorations::InlayHintEntry {
                    pos,
                    text: hint_text,
                    side,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_inlay_hints(source, bid, entries);
        Ok(())
    }

    fn register_sign_source(
        &mut self,
        name: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String> {
        self.state
            .config
            .decorations
            .register_sign_source(name, bid, priority);
        Ok(())
    }

    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String)>,
    ) -> Result<(), String> {
        if self
            .state
            .config
            .decorations
            .sign_slot(bid, &source)
            .is_none()
        {
            return Err(format!(
                "unregistered sign source {source:?}, call \
                 (register-sign-source! …) first"
            ));
        }
        let text = buffer_text(self.state, bid)?;
        let entries = signs
            .into_iter()
            .map(|(line, sign_text, scope)| {
                Ok(hume_decorations::SignEntry {
                    line: content_line(text, line)?,
                    text: sign_text.into(),
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_signs(source, bid, entries);
        Ok(())
    }

    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<hume_scripting::VirtualLineSpec>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid)?;
        let entries = lines
            .into_iter()
            .map(|spec| {
                let line = content_line(text, spec.line)?;
                let scope = match spec.scope.as_deref() {
                    Some(name) => self.view.registry.intern_runtime(name),
                    None => self
                        .view
                        .registry
                        .intern(hume_engine::theme::ui_scopes::VIRTUAL_TEXT),
                };
                let segments = virtual_line_segments_to_bytes(&spec.text, spec.segments)?
                    .into_iter()
                    .map(|(start, end, name)| {
                        (
                            ByteCol::new(start),
                            ByteCol::new(end),
                            self.view.registry.intern_runtime(&name),
                        )
                    })
                    .collect();
                Ok(hume_decorations::VirtualLineEntry {
                    line,
                    text: spec.text,
                    side: spec.side,
                    scope,
                    segments,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_virtual_lines(source, bid, entries);
        Ok(())
    }

    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid)?;
        let entries = spans
            .into_iter()
            .map(|(start, end, scope)| {
                let fail = |e: String| format!("range {e}");
                let range = crate::editor::buffer::offsets::checked_char_range(text, start, end)
                    .map_err(fail)?;
                if range.is_empty() {
                    return Err(fail(format!("{start}..{end} is empty")));
                }
                // Widened to whole clusters: the renderer paints a cluster at a time.
                let range = crate::editor::buffer::offsets::widened_to_clusters(text, range);
                Ok(hume_decorations::ExtraHighlightEntry {
                    start: range.start,
                    end: range.end,
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_extra_highlights(source, bid, entries);
        Ok(())
    }

    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, String)>,
        hide_on_insert_line: bool,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid)?;
        let entries = lines
            .into_iter()
            .map(|(line, eol_text, scope)| {
                Ok(hume_decorations::EolTextEntry {
                    line: content_line(text, line)?,
                    text: eol_text,
                    scope: self.view.registry.intern_runtime(&scope),
                    hide_on_insert_line,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_eol_text(source, bid, entries);
        Ok(())
    }

    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(usize, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid)?;
        let entries = entries
            .into_iter()
            .map(|(line, scope)| {
                Ok(hume_decorations::LineBgEntry {
                    line: content_line(text, line)?,
                    scope: self.view.registry.intern_runtime(&scope),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_line_backgrounds(source, bid, entries);
        Ok(())
    }

    fn set_statusline_text(
        &mut self,
        source: String,
        bid: BufferId,
        text: String,
    ) -> Result<(), String> {
        // Return value discarded: called purely to reuse the SSOT "unknown
        // buffer" wording every sibling setter raises for a stale `bid`.
        buffer_text(self.state, bid)?;
        // A name `StatusElement::from_str` would reject can never be placed,
        // so reject it here too, or the push silently stores an entry no
        // `steel:<name>` element can ever render.
        StatusElement::custom(&source).map_err(|e| e.to_string())?;
        // Wholesale replace, same as every sibling decoration setter
        // (`SourceStore::set`): an empty `text` is stored as-is rather than
        // pruned; `render_element`'s `Custom` arm and `render_section` both
        // already treat empty and absent identically.
        self.state
            .config
            .statusline_text
            .entry(bid)
            .or_default()
            .insert(source.into_boxed_str(), text.into_boxed_str());
        Ok(())
    }

    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<DiagSeverity>,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<hume_scripting::host::DiagnosticEntry>, String> {
        crate::editor::lsp::diagnostics::diagnostics_for_buffer(
            self.state,
            bid,
            severity_floor,
            range,
        )
    }

    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        crate::editor::lsp::introspect::diagnostic_counts(self.state, bid)
    }
}

/// Converts `segments`' char offsets into `text` to byte offsets, sorting by
/// `start` and validating in the process: the sole enforcement point for
/// `set-virtual-lines!`'s segment contract (bounds, ordering, non-overlap).
/// The Steel boundary (`virtual_line_specs` in `hume-scripting`'s
/// `builtins/decorations.rs`) only decodes shape. See
/// `VirtualLineSpec::segments`'s doc.
///
/// An edge inside a grapheme cluster widens the segment to the whole
/// cluster: the engine (`hume-engine/src/format/virtual_lines.rs`'s
/// `format_virtual_lines`) resolves each virtual grapheme's scope once per
/// cluster, at the cluster's start byte, so part of a cluster can't be
/// painted. Two segments that widen into the same cluster overlap and raise:
/// one character can't take two scopes.
pub(super) fn virtual_line_segments_to_bytes(
    text: &str,
    mut segments: Vec<(usize, usize, String)>,
) -> Result<Vec<(usize, usize, String)>, String> {
    segments.sort_by_key(|(start, _, _)| *start);

    let slice = ropey::RopeSlice::from(text);
    let char_count = slice.len_chars();

    let mut prev_end = 0usize;
    let mut out = Vec::with_capacity(segments.len());
    for (start, end, scope) in segments {
        if start >= end {
            return Err(format!(
                "segments: segment ({start}, {end}) must have start < end"
            ));
        }
        if end > char_count {
            return Err(format!(
                "segments: segment end {end} is past text's char length {char_count}"
            ));
        }
        // Trusted mint: `start < end <= char_count` is checked above.
        let chars = hume_rope::cluster::ClusterRange::covering(
            slice,
            ExclusiveRange::new(CharOffset::new(start), CharOffset::new(end)),
        )
        .expect("a non-empty range within the text covers a cluster")
        .chars();
        let start_byte = slice.char_to_byte(chars.start.index());
        let end_byte = slice.char_to_byte(chars.end.index());
        if start_byte < prev_end {
            return Err(format!(
                "segments: segments must not overlap (segment ({start}, \
                 {end}) shares a character with the previous one)"
            ));
        }
        prev_end = end_byte;
        out.push((start_byte, end_byte, scope));
    }

    Ok(out)
}

/// The live text for `bid`, or `Err` if `bid` doesn't name
/// an open buffer. Every decoration setter needs this to validate/convert
/// its Steel-facing positions. `bid`'s liveness is already checked before
/// any of these setters' bodies run (`args::LivePane`'s `BuiltinArg::resolve`
/// in the `builtins!`-registered closure), so the `Err` arm here is a
/// defensive fallback, not the primary check. It is kept because `try_get` could
/// in principle diverge from the liveness check's own `buffer_exists`.
fn buffer_text(
    state: &EditorState,
    bid: BufferId,
) -> Result<&hume_editing::text::BufferText, String> {
    state
        .buffers
        .try_get(bid)
        .map(|b| b.text())
        .ok_or_else(|| "unknown buffer".to_owned())
}

/// `line` as a content line of `text`, or `Err` if it is
/// out of range: the one place a line-anchored decoration's Steel-facing
/// line number becomes the line it is stored on.
///
/// Rejects the buffer's last *ropey* line, not just any out-of-range line:
/// the buffer invariant (every buffer ends with a structural `\n`) means
/// that last line is always the empty phantom line the trailing `\n`
/// produces: zero-width, nothing to decorate, and no `\n` of its own to
/// follow through edits. `DisplayLineMap::last_line()` never lays it out.
pub(super) fn content_line(
    text: &hume_editing::text::BufferText,
    line: usize,
) -> Result<hume_rope::line::ContentLine, String> {
    hume_rope::line::ContentLine::checked(text.rope(), line).ok_or_else(|| {
        format!(
            "line {line} is out of range (buffer has {} content lines)",
            text.content_line_count().get()
        )
    })
}

/// `pos` must address a real char in `text` (`<` its length), `Err`
/// otherwise. A `pos` inside a grapheme cluster snaps to it: a
/// `'before` hint to the cluster's first char, an `'after` one to its last,
/// so the hint sits before or after the whole character.
/// `pos == len_chars()` is refused: `visible_char_range` is half-open, so
/// that position never passes its `contains` check and the hint would not
/// render.
///
/// An 'after' hint gets a second check: the render bridge
/// (`decoration_providers.rs`'s `update_inlay_hint_providers`) anchors an
/// 'after' hint at `pos + 1`, so a hint on the buffer's last content char
/// (its trailing structural `\n`) would resolve to the trailing phantom
/// line, the same unresolvable position `content_line` already refuses
/// for the line-anchored kinds, but reachable here through a char offset
/// instead of a line number, so that check alone doesn't catch it.
pub(super) fn validate_offset(
    text: &hume_editing::text::BufferText,
    pos: usize,
    side: Side,
) -> Result<CharOffset, String> {
    if pos >= text.len_chars() {
        return Err(format!(
            "offset {pos} is out of range (buffer has {} chars)",
            text.len_chars()
        ));
    }
    // Trusted mint: `pos < len_chars()` is checked above.
    let at = CharOffset::new(pos);
    let cluster = text
        .covering(ExclusiveRange::new(at, at.shift(1)))
        .expect("a char below the text's length lies in a cluster");
    let pos = match side {
        Side::Before => cluster.start().offset(),
        Side::After => cluster.end().offset().retreat(1),
    };
    if side == Side::After {
        // `pos` is a char below `len_chars()`, so `pos` shifted by one
        // codepoint stays `<= len_chars()`: the one past-the-end position
        // `ropey_char_to_line` (ropey domain) accepts, needed to find which
        // line an 'after' hint at `pos + 1` actually lands on.
        let landing_line = text.ropey_char_to_line(pos.shift(1));
        if landing_line.to_content(text.rope()).is_none() {
            return Err(format!(
                "offset {} anchored 'after would land on the buffer's trailing \
                 empty line",
                pos.index()
            ));
        }
    }
    Ok(pos)
}
