//! A popup's text as the box shows it (`show-popup! #:lang`, the completion
//! documentation box): parsed once, highlighted from that parse, and drawn
//! with the conceals, highlights and line backgrounds a `format-text/<lang>`
//! command returns for it, resolved into the lines
//! `hume_ui::popup::wrap_styled` wraps.
//!
//! The text never changes after it is parsed, so there is nothing to
//! reparse, and a format is asked for once per text.

use hume_decorations::{ConcealEntry, ConcealEntrySpan};
use hume_editing::text::BufferText;
use hume_engine::interval_sweep::{TieBreak, flatten_overlapping_spans};
use hume_engine::providers::{ConcealText, SyntaxSource};
use hume_engine::theme::ui_scopes;
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::{ResolvedStyle, Scope, ScopeId};
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_scripting::TextFormatSpec;
use hume_treesitter::registry::LanguageRegistry;
use hume_ui::popup::{PopupContent, StyledLine};

use hume_core::{HostToken, Severity};

use crate::editor::Editor;
use crate::editor::host_impl::{
    EditorHostImpl, conceal_entries, extra_highlight_entries, line_background_entries,
};
use crate::editor::input_stack::{BufferCompletionLayer, InputStack, PopupLayer};
use crate::editor::syntax::GrammarLoads;

/// Synchronously parses `text` through the grammar named `lang`, loading it
/// and every grammar its injections need through `loads`. `None` when there
/// is no such grammar or it fails to load, which leaves the popup rendering
/// plain.
pub(in crate::editor) fn build_markup_syntax(
    loads: &mut GrammarLoads<'_>,
    lang: &str,
    text: &str,
) -> Option<MarkupSyntax> {
    let bundle = loads.grammar_named(lang)?;
    let text = BufferText::from(text);
    let syntax = hume_treesitter::syntax::Syntax::attach_sync(bundle, &text, &mut *loads);
    Some(MarkupSyntax { syntax, text })
}

/// A popup's text, its parse, its format, and the `ui.popup` box content
/// built from them. The built content bakes the theme's colors in, so a
/// holder calls [`Self::rebuild`] when the theme changes
/// (`theme::set_theme`).
pub(in crate::editor) struct Markup {
    text: String,
    /// `None` without a grammar for the text: the popup renders plain, and
    /// no formatter is asked for. Lent out while a formatter runs
    /// ([`Self::begin_format`]).
    syntax: Option<MarkupSyntax>,
    format: Format,
    content: PopupContent,
}

/// Whether a [`Markup`]'s formatter has run.
enum Format {
    /// A formatter for the text's grammar may still apply.
    Pending,
    /// The format the formatter returned, `None` when there was no formatter
    /// or it failed.
    Done(Option<TextFormat>),
}

impl Markup {
    /// `text`, highlighted through `syntax` when there is one, unformatted.
    pub(in crate::editor) fn new(
        text: String,
        syntax: Option<MarkupSyntax>,
        theme: &mut Theme,
        registry: &ScopeRegistry,
    ) -> Self {
        let format = match syntax {
            Some(_) => Format::Pending,
            None => Format::Done(None),
        };
        let content = popup_content(&text, syntax.as_ref(), None, theme, registry);
        Self {
            text,
            syntax,
            format,
            content,
        }
    }

    #[cfg(test)]
    pub(in crate::editor) fn text(&self) -> &str {
        &self.text
    }

    /// The box content, built against the current theme.
    pub(in crate::editor) fn content(&self) -> &PopupContent {
        &self.content
    }

    #[cfg(test)]
    pub(in crate::editor) fn syntax(&self) -> Option<&MarkupSyntax> {
        self.syntax.as_ref()
    }

    /// The name of the grammar a formatter would format this text for:
    /// `None` once it has been formatted, or with no grammar to format for.
    pub(in crate::editor) fn pending_language<'r>(
        &self,
        languages: &'r LanguageRegistry,
    ) -> Option<&'r str> {
        match (&self.format, &self.syntax) {
            (Format::Pending, Some(syntax)) => {
                Some(languages.name_of(syntax.syntax.bundle().language))
            }
            _ => None,
        }
    }

    /// The parse a formatter reads, moved out of `self` until
    /// [`Self::end_format`] puts it back. `None` unless a format is pending.
    pub(in crate::editor) fn begin_format(&mut self) -> Option<MarkupSyntax> {
        match self.format {
            Format::Pending => self.syntax.take(),
            Format::Done(_) => None,
        }
    }

    /// Puts `syntax` back from [`Self::begin_format`] with the format its
    /// formatter returned, `None` for none, and rebuilds the content with a
    /// format. Without one, the content already shows the text unformatted.
    pub(in crate::editor) fn end_format(
        &mut self,
        syntax: MarkupSyntax,
        format: Option<TextFormat>,
        theme: &mut Theme,
        registry: &ScopeRegistry,
    ) {
        let formatted = format.is_some();
        self.syntax = Some(syntax);
        self.format = Format::Done(format);
        if formatted {
            self.rebuild(theme, registry);
        }
    }

    /// Builds the content again against `theme`, keeping the format.
    pub(in crate::editor) fn rebuild(&mut self, theme: &mut Theme, registry: &ScopeRegistry) {
        let format = match &self.format {
            Format::Done(format) => format.as_ref(),
            Format::Pending => None,
        };
        self.content = popup_content(&self.text, self.syntax.as_ref(), format, theme, registry);
    }
}

/// A `ui.popup` box's content for `text`: highlighted through `syntax` and
/// drawn with `format` when there is a parse, plain otherwise. Its styled
/// runs bake `theme`'s colors in.
///
/// Bakes `theme` first: the grammar interned its scopes when `syntax` was
/// built, and the format when it was checked, possibly since the last bake,
/// and resolving an unbaked scope is a bug (`Theme::resolve`).
fn popup_content(
    text: &str,
    syntax: Option<&MarkupSyntax>,
    format: Option<&TextFormat>,
    theme: &mut Theme,
    registry: &ScopeRegistry,
) -> PopupContent {
    match syntax {
        Some(syntax) => {
            theme.bake_if_stale(registry);
            let base_style = theme.resolve_by_name(Scope(ui_scopes::POPUP));
            PopupContent::styled(syntax.styled_lines(text, format, theme, base_style))
        }
        None => PopupContent::plain(text),
    }
}

/// What a `format-text/<lang>` command returned for a text, checked against
/// that text's parse as the decoration setters check theirs, scopes
/// interned, and resolved as a buffer draws its decorations.
pub(in crate::editor) struct TextFormat {
    /// In position order, none overlapping another.
    conceals: Vec<ConcealEntry>,
    /// In position order, none overlapping another: where two entries
    /// overlap, the one starting first is drawn, the first listed for the
    /// same start.
    highlights: Vec<(ExclusiveRange<CharOffset>, ScopeId)>,
    /// Each line's background scope, by line: the first entry for the line.
    line_backgrounds: Vec<Option<ScopeId>>,
}

impl TextFormat {
    /// `spec`, returned for the text `syntax` parsed, checked against it.
    /// The error names the list holding the bad entry.
    pub(in crate::editor) fn checked(
        spec: TextFormatSpec,
        syntax: &MarkupSyntax,
        registry: &mut ScopeRegistry,
    ) -> Result<Self, String> {
        let text = &syntax.text;
        let conceals = conceal_entries(text, registry, spec.conceals, spec.overlaps)
            .map_err(|e| format!("'conceals {e}"))?;
        let mut entries = extra_highlight_entries(text, registry, spec.highlights)
            .map_err(|e| format!("'highlights {e}"))?;
        // Sorted first: a tie between overlapping spans goes to the one
        // pushed first.
        entries.sort_by_key(|entry| entry.start);
        let mut raw = entries
            .into_iter()
            .map(|entry| (entry.start, entry.end, (), entry.scope))
            .collect();
        let mut highlights = Vec::new();
        flatten_overlapping_spans(
            &mut raw,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut highlights,
            TieBreak::FirstPushed,
        );
        let mut line_backgrounds = vec![None; text.content_line_count().get()];
        for entry in line_background_entries(registry, spec.line_backgrounds) {
            line_backgrounds[entry.line.index()].get_or_insert(entry.scope);
        }
        Ok(Self {
            conceals,
            highlights,
            line_backgrounds,
        })
    }
}

/// Synchronously-parsed highlight state for a popup's read-only text.
pub(in crate::editor) struct MarkupSyntax {
    pub(in crate::editor) syntax: hume_treesitter::syntax::Syntax,
    /// Same content the syntax was parsed from, wrapped as a rope-backed
    /// `BufferText`: `Syntax::spans_for_line` needs `&Rope`, and re-deriving
    /// one from the source string every frame would re-walk it on every
    /// render. The text a formatter is handed, and checked against.
    pub(in crate::editor) text: BufferText,
}

impl MarkupSyntax {
    /// Every line of `text` as styled lines for `hume_ui::popup::wrap_styled`:
    /// each line's highlight spans resolved over `base_style`, then `format`'s
    /// line background, highlights and conceals applied, in the order the
    /// buffer renderer layers them.
    ///
    /// Slices `text`'s own lines (`text.split('\n')`), not `self.text`'s
    /// rope lines: `self.text` is `BufferText::from(text)`, which may have
    /// padded on a trailing `'\n'` `text` itself lacked (the buffer
    /// invariant), and iterating the padded rope would emit a spurious
    /// trailing empty row a plain popup on `text` never would.
    fn styled_lines(
        &self,
        text: &str,
        format: Option<&TextFormat>,
        theme: &Theme,
        base_style: ResolvedStyle,
    ) -> Vec<StyledLine> {
        // A trailing '\n' in `text` itself (as opposed to the buffer-invariant
        // padding `BufferText::from` may have added) would otherwise make
        // `split('\n')` yield one more element than `self.text` has real
        // lines, landing this loop's trusted `ContentLine` mint on the
        // phantom line. Stripping it here is what keeps that mint honest.
        let mut spans = Vec::new();
        let mut line_start = 0;
        text.strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .enumerate()
            .map(|(line_idx, line)| {
                // Trusted mint: `line_idx` indexes `text`'s own unpadded
                // lines (see above), always within `self.text`'s real content.
                let line_idx = ContentLine::new(line_idx);
                // Trusted mints: offsets into `text`, which `self.text` holds
                // unchanged, its line break included.
                let start = CharOffset::new(line_start);
                let end = CharOffset::new(line_start + line.chars().count());
                line_start = end.index() + 1;
                spans.clear();
                self.syntax
                    .spans_for_line(line_idx, self.text.rope(), &mut spans);
                let band = format
                    .and_then(|f| f.line_backgrounds[line_idx.index()])
                    .and_then(|scope| theme.resolve(scope).bg);
                let base = base_style.layer(ResolvedStyle {
                    bg: band,
                    ..ResolvedStyle::default()
                });
                let mut out = LineRuns::new(theme, format, base, start, end);
                for (run, scope) in hume_rope::spanned::runs(line, &spans) {
                    let style = scope.map_or(out.base, |&s| theme.resolve(s));
                    for cluster in unicode_segmentation::UnicodeSegmentation::graphemes(run, true) {
                        out.push_cluster(cluster, style);
                    }
                }
                out.push_inserts(out.base);
                StyledLine {
                    runs: out.runs,
                    fill: band,
                }
            })
            .collect()
    }
}

/// One line's runs as [`MarkupSyntax::styled_lines`] builds them, cluster by
/// cluster.
struct LineRuns<'a> {
    theme: &'a Theme,
    /// The format's conceals on this line, in position order, an insert
    /// before the line's `\n` included.
    conceals: &'a [ConcealEntry],
    /// The format's highlights on this line that end after the next
    /// cluster, in position order.
    highlights: &'a [(ExclusiveRange<CharOffset>, ScopeId)],
    /// The style of text no highlight span covers: `ui.popup`, under the
    /// line's background when it has one.
    base: ResolvedStyle,
    /// Where the next cluster starts.
    at: CharOffset,
    runs: Vec<(String, ResolvedStyle)>,
}

impl<'a> LineRuns<'a> {
    /// The line from `start` to its `\n` at `end`, drawn with `format`.
    fn new(
        theme: &'a Theme,
        format: Option<&'a TextFormat>,
        base: ResolvedStyle,
        start: CharOffset,
        end: CharOffset,
    ) -> Self {
        let (conceals, highlights) = match format {
            Some(format) => {
                let conceals = &format.conceals;
                let first = conceals.partition_point(|c| c.chars().start < start);
                let last = conceals.partition_point(|c| c.chars().start <= end);
                let highlights = &format.highlights;
                let first_hl = highlights.partition_point(|(range, _)| range.end <= start);
                let last_hl = highlights.partition_point(|(range, _)| range.start <= end);
                (&conceals[first..last], &highlights[first_hl..last_hl])
            }
            None => (&[][..], &[][..]),
        };
        Self {
            theme,
            conceals,
            highlights,
            base,
            at: start,
            runs: Vec::new(),
        }
    }

    /// `style` with the highlight covering the next cluster, if any, layered
    /// over it.
    fn highlighted(&mut self, style: ResolvedStyle) -> ResolvedStyle {
        while let [(range, _), rest @ ..] = self.highlights
            && range.end <= self.at
        {
            self.highlights = rest;
        }
        match self.highlights {
            [(range, scope), ..] if range.start <= self.at => {
                style.layer(self.theme.resolve(*scope))
            }
            _ => style,
        }
    }

    /// `text` in `style`, its scope layered over it.
    fn push_conceal_text(&mut self, text: &ConcealText, style: ResolvedStyle) {
        let style = text
            .scope
            .map_or(style, |scope| style.layer(self.theme.resolve(scope)));
        push_run(&mut self.runs, &text.text, style);
    }

    /// The text each insert before the next cluster draws, over `style`.
    fn push_inserts(&mut self, style: ResolvedStyle) {
        let style = self.highlighted(style);
        for conceal in self.conceals {
            if let ConcealEntrySpan::Inserted { at, text } = &conceal.span
                && *at == self.at
            {
                self.push_conceal_text(text, style);
            }
        }
    }

    /// `cluster`, the next cluster, in `style`, or what the conceal hiding
    /// it draws in its place.
    fn push_cluster(&mut self, cluster: &str, style: ResolvedStyle) {
        self.push_inserts(style);
        let style = self.highlighted(style);
        let at = self.at;
        let hidden = self
            .conceals
            .iter()
            .find_map(|conceal| match &conceal.span {
                ConcealEntrySpan::Hidden { chars, text } if chars.contains(at) => {
                    Some((chars.start == at, text.as_ref()))
                }
                _ => None,
            });
        match hidden {
            None => push_run(&mut self.runs, cluster, style),
            Some((true, Some(text))) => self.push_conceal_text(text, style),
            Some(_) => {}
        }
        self.at = at.shift(cluster.chars().count() as isize);
    }
}

/// `text` in `style` appended to `runs`, merged into the last run when it
/// has the same style.
fn push_run(runs: &mut Vec<(String, ResolvedStyle)>, text: &str, style: ResolvedStyle) {
    match runs.last_mut() {
        Some((last, last_style)) if *last_style == style => last.push_str(text),
        _ => runs.push((text.to_owned(), style)),
    }
}

/// Where an open popup text lives.
#[derive(Clone, Copy)]
enum MarkupSite {
    /// The `show-popup!` popup, in either of its homes.
    Popup,
    /// The completion documentation box's text for the selected item.
    DocBox,
}

impl MarkupSite {
    fn markup(self, input: &mut InputStack) -> Option<&mut Markup> {
        match self {
            Self::Popup => input.popup_mut().map(PopupLayer::markup_mut),
            Self::DocBox => input
                .find_mut::<BufferCompletionLayer>()
                .and_then(BufferCompletionLayer::doc_markup_mut),
        }
    }
}

impl Editor {
    /// Formats every open popup text still waiting for its format: the
    /// `show-popup!` popup and the documentation box's text. Run in the
    /// settle pass, so the first frame that shows a text shows it formatted.
    pub(in crate::editor) fn format_pending_markups(&mut self) {
        self.format_markup(MarkupSite::Popup);
        self.format_markup(MarkupSite::DocBox);
    }

    /// Formats the text at `site` with its grammar's `format-text/<lang>`
    /// command, activating the lazy plugin that declares it first. The
    /// command runs read-only, lent the text's parse, and its result is
    /// checked against the text. With no such command, or a failed one,
    /// the text keeps its highlighted source; a failure is reported.
    fn format_markup(&mut self, site: MarkupSite) {
        let Some(name) = site
            .markup(&mut self.state.input)
            .and_then(|markup| markup.pending_language(&self.state.config.languages))
            .map(|lang| format!("format-text/{lang}"))
        else {
            return;
        };
        if let Some(plugin) = self.state.config.registry.lazy_owner(&name).cloned() {
            self.activate_lazy_plugin(&plugin, &name);
        }
        let proc = self.state.config.registry.steel_proc(&name);
        let Some(syntax) = site
            .markup(&mut self.state.input)
            .and_then(Markup::begin_format)
        else {
            return;
        };
        let format = match (proc, self.scripting.as_mut()) {
            (Some(proc), Some(scripting)) => {
                let token = HostToken::mint();
                let mut host =
                    EditorHostImpl::new(&mut self.state, &mut self.view).lending(token, &syntax);
                let spec = scripting.format_text(&proc, &syntax.text, token, &mut host);
                self.flush_script_messages();
                match spec
                    .and_then(|spec| TextFormat::checked(spec, &syntax, &mut self.view.registry))
                {
                    Ok(format) => Some(format),
                    Err(message) => {
                        self.report(Severity::Error, format!("{name}: {message}"));
                        None
                    }
                }
            }
            _ => None,
        };
        site.markup(&mut self.state.input)
            .expect("a read-only formatter cannot close its popup")
            .end_format(syntax, format, &mut self.view.theme, &self.view.registry);
    }
}
