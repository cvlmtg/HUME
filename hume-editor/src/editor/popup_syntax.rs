//! Syntax-highlighted popup content (`show-popup! #:lang`) — resolves a
//! grammar's capture spans into the flat run sequence
//! `hume_ui::popup::wrap_styled` wraps.
//!
//! Built once at `show-popup!` time (`Editor::build_markup_syntax`) — there
//! is nothing to incrementally reparse, since the content never changes
//! after this.

use hume_engine::providers::SyntaxSpans;
use hume_engine::theme::Theme;
use hume_engine::types::ResolvedStyle;

/// Synchronously-parsed highlight state for a popup's read-only text, keyed
/// by grammar name (`#:lang`). `None` where `PopupModel::syntax` would go
/// when no grammar by that name is registered, or `#:lang` wasn't
/// requested — the plain-text fallback.
///
/// Shared by the cursor and docked popup layouts so highlight resolution
/// (`styled_row`/`styled_runs`) has one implementation — only
/// wrapping/geometry (`hume_ui::popup::resolve_popup`/`resolve_band`)
/// differs between the two.
pub(in crate::editor) struct MarkupSyntax {
    pub(in crate::editor) syntax: hume_treesitter::syntax::Syntax,
    /// Same content the syntax was parsed from, wrapped as a rope-backed
    /// `BufferText` — `Syntax::spans_for_line` needs `&Rope`, and re-deriving
    /// one from the source string every frame would re-walk it on every
    /// render.
    pub(in crate::editor) text: hume_editing::text::BufferText,
}

impl MarkupSyntax {
    /// Highlight spans for one source line, pushed straight into `out` as
    /// contiguous `(text, style)` runs — one run per span plus the gaps
    /// between them, never merged: `hume_ui::popup::wrap_styled` flattens
    /// every run to per-grapheme atoms and re-coalesces adjacent same-style
    /// ones before wrapping, so merging here would just be rebuilding work
    /// it immediately undoes.
    ///
    /// `line` is the caller's own text for `line_idx` (not re-sliced from
    /// `self.text`'s rope) — byte offsets from `spans_for_line` are relative
    /// to the line start either way, so this stays exact even when the
    /// caller's line boundaries differ slightly from the rope's own (see
    /// `styled_runs`'s doc on `self.text`'s padded trailing `'\n'`).
    fn styled_row(
        &self,
        line_idx: hume_rope::line::ContentLine,
        line: &str,
        theme: &Theme,
        base_style: ResolvedStyle,
        out: &mut Vec<(String, ResolvedStyle)>,
    ) {
        let mut spans = Vec::new();
        self.syntax
            .spans_for_line(line_idx, self.text.rope(), &mut spans);

        let mut cursor = 0usize;
        for &(start, end, scope) in &spans {
            let (start, end) = (start.index(), end.index());
            if start > cursor {
                out.push((line[cursor..start].to_string(), base_style));
            }
            out.push((line[start..end].to_string(), theme.resolve(scope)));
            cursor = end;
        }
        if cursor < line.len() {
            out.push((line[cursor..].to_string(), base_style));
        }
    }

    /// Resolve every line of `text` into one flat run sequence for
    /// `hume_ui::popup::wrap_styled` — lines joined by a bare `"\n"` run so
    /// its paragraph splitting sees the exact same boundaries a plain popup
    /// would.
    ///
    /// Slices `text`'s own paragraphs (`text.split('\n')`), not `self.text`'s
    /// rope lines — `self.text` is `BufferText::from(text)`, which may have
    /// padded on a trailing `'\n'` `text` itself lacked (the buffer
    /// invariant), and iterating the padded rope would emit a spurious
    /// trailing empty row `wrap_text` on plain `text` never would.
    pub(in crate::editor) fn styled_runs(
        &self,
        text: &str,
        theme: &Theme,
        base_style: ResolvedStyle,
    ) -> Vec<(String, ResolvedStyle)> {
        // A trailing '\n' in `text` itself (as opposed to the buffer-invariant
        // padding `BufferText::from` may have added) would otherwise make
        // `split('\n')` yield one more element than `self.text` has real
        // lines — landing this loop's trusted `ContentLine` mint on the
        // phantom line. Stripping it here is what keeps that mint honest.
        let lines: Vec<&str> = text
            .strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .collect();
        let mut runs: Vec<(String, ResolvedStyle)> = Vec::new();
        for (line_idx, line) in lines.iter().enumerate() {
            // Trusted mint: `lines` is `text`'s own unpadded split (see
            // above), always within `self.text`'s real content even though
            // the latter may carry one more (phantom) line than `lines.len()`.
            let content_line = hume_rope::line::ContentLine::new(line_idx);
            self.styled_row(content_line, line, theme, base_style, &mut runs);
            if line_idx + 1 < lines.len() {
                runs.push(("\n".to_string(), base_style));
            }
        }
        runs
    }
}
