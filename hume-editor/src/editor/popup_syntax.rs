//! A popup's text as the box shows it (`show-popup! #:lang`, the completion
//! documentation box): parsed once, highlighted from that parse, and drawn
//! with the conceals, highlights and line backgrounds a `format-text/<lang>`
//! command returns for it, through the layout and style stages a pane draws
//! its lines with, into the lines `hume_ui::popup::wrap_styled` wraps.
//!
//! The text never changes after it is parsed, so there is nothing to
//! reparse, and a format is asked for once per text.

use hume_decorations::build_text_providers;
use hume_editing::text::{BufferText, normalize_line_endings_owned};
use hume_engine::providers::ProviderSet;
use hume_engine::text_lines::styled_lines;
use hume_engine::theme::ui_scopes;
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::Scope;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_scripting::TextFormatSpec;
use hume_treesitter::registry::LanguageRegistry;
use hume_ui::popup::PopupContent;

use hume_core::{HostToken, Severity};

use crate::editor::Editor;
use crate::editor::decoration_providers::{
    conceals_by_line, extra_highlight_spans, first_per_line,
};
use crate::editor::host_impl::{
    EditorHostImpl, conceal_entries, line_background_entries, scoped_highlight_entries,
};
use crate::editor::input_stack::{BufferCompletionLayer, InputStack, PopupLayer};
use crate::editor::syntax::GrammarLoads;

/// Synchronously parses `text` through the grammar named `lang`, loading it
/// and every grammar its injections need through `loads`. `None` when there
/// is no such grammar or it fails to load, which leaves the popup rendering
/// plain.
fn build_markup_syntax(
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
/// holder calls [`Self::rebuild`] when the theme's styles change
/// (`theme::apply_scope_styles`).
pub(in crate::editor) struct Markup {
    /// Every line break a `\n`, so the parse's text is this one, padded
    /// with a final `\n` when it lacks one.
    text: String,
    /// `None` without a grammar for the text: the popup renders plain, and
    /// no formatter is asked for. Lent out while a formatter runs
    /// ([`Self::begin_format`]).
    syntax: Option<MarkupSyntax>,
    format: Format,
    /// The providers of the format the formatter returned, empty until it
    /// returns one.
    providers: ProviderSet,
    content: PopupContent,
}

/// Whether a [`Markup`]'s formatter has run.
enum Format {
    /// A formatter for the text's grammar may still apply.
    Pending,
    Done,
}

impl Markup {
    /// `text` with its line breaks normalized, highlighted through the
    /// grammar named `lang` when there is one, which `loads` loads,
    /// unformatted.
    pub(in crate::editor) fn new(
        text: String,
        lang: Option<&str>,
        loads: &mut GrammarLoads<'_>,
        theme: &mut Theme,
    ) -> Self {
        let text = normalize_line_endings_owned(text);
        let syntax = lang.and_then(|lang| build_markup_syntax(loads, lang, &text));
        Self::parsed(text, syntax, theme, loads.scopes())
    }

    /// `text` with its line breaks normalized, plain.
    #[cfg(test)]
    pub(in crate::editor) fn plain(
        text: &str,
        theme: &mut Theme,
        registry: &ScopeRegistry,
    ) -> Self {
        Self::parsed(
            normalize_line_endings_owned(text.to_owned()),
            None,
            theme,
            registry,
        )
    }

    /// `text`, normalized, highlighted through `syntax`, parsed from it.
    fn parsed(
        text: String,
        syntax: Option<MarkupSyntax>,
        theme: &mut Theme,
        registry: &ScopeRegistry,
    ) -> Self {
        let format = match syntax {
            Some(_) => Format::Pending,
            None => Format::Done,
        };
        let providers = ProviderSet::new();
        let content = popup_content(&text, syntax.as_ref(), &providers, theme, registry);
        Self {
            text,
            syntax,
            format,
            providers,
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
            Format::Done => None,
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
        self.syntax = Some(syntax);
        self.format = Format::Done;
        if let Some(format) = format {
            self.providers = format.providers;
            self.rebuild(theme, registry);
        }
    }

    /// Builds the content again against `theme`, keeping the format.
    pub(in crate::editor) fn rebuild(&mut self, theme: &mut Theme, registry: &ScopeRegistry) {
        self.content = popup_content(
            &self.text,
            self.syntax.as_ref(),
            &self.providers,
            theme,
            registry,
        );
    }
}

/// A `ui.popup` box's content for `text`: highlighted through `syntax` and
/// drawn with a format's `providers` when there is a parse, as a pane draws
/// a line, plain otherwise. Its styled runs bake `theme`'s colors in.
///
/// Bakes `theme` first: the grammar interned its scopes when `syntax` was
/// built, and the format when it was checked, possibly since the last bake,
/// and resolving an unbaked scope is a bug (`Theme::resolve`).
fn popup_content(
    text: &str,
    syntax: Option<&MarkupSyntax>,
    providers: &ProviderSet,
    theme: &mut Theme,
    registry: &ScopeRegistry,
) -> PopupContent {
    let Some(syntax) = syntax else {
        return PopupContent::plain(text);
    };
    theme.bake_if_stale(registry);
    let base = theme.resolve_by_name(Scope(ui_scopes::POPUP));
    PopupContent::styled(styled_lines(
        syntax.text.rope(),
        &syntax.syntax,
        providers,
        base,
        theme,
    ))
}

/// What a `format-text/<lang>` command returned for a text, checked against
/// that text's parse as the decoration setters check theirs, scopes
/// interned, and held by the providers a pane draws its own decorations
/// with.
pub(in crate::editor) struct TextFormat {
    /// The format's conceals, extra highlights and line backgrounds.
    providers: ProviderSet,
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
        let highlights = scoped_highlight_entries(text, spec.highlights, registry)
            .map_err(|e| format!("'highlights {e}"))?;
        let line_backgrounds = line_background_entries(registry, spec.line_backgrounds)
            .map_err(|e| format!("'line-backgrounds {e}"))?;
        let mut providers = ProviderSet::new();
        let handles = build_text_providers(&mut providers);
        handles.set_conceals(conceals_by_line(text, &conceals));
        let whole = ExclusiveRange::new(CharOffset::default(), text.end());
        handles.set_extra(extra_highlight_spans(
            text,
            &highlights,
            std::iter::once(whole),
        ));
        handles.set_line_backgrounds(first_per_line(
            line_backgrounds
                .iter()
                .map(|entry| (entry.line, entry.scope)),
        ));
        Ok(Self { providers })
    }
}

/// Synchronously-parsed highlight state for a popup's read-only text.
pub(in crate::editor) struct MarkupSyntax {
    pub(in crate::editor) syntax: hume_treesitter::syntax::Syntax,
    /// Same content the syntax was parsed from, wrapped as a rope-backed
    /// `BufferText`: the text the popup is drawn from, and the one a
    /// formatter is handed and checked against.
    pub(in crate::editor) text: BufferText,
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
