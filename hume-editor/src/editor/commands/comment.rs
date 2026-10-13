//! `toggle-comment`: each unit takes the comment tokens of the language at
//! its text, the innermost syntax layer that has any.

use hume_editing::comment::CommentTokens;
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::edit::{
    CommentPlanError, MultiLineComments, apply_comment_plan, plan_comment_toggle,
};
use hume_rope::cluster::ClusterRange;
use hume_treesitter::registry::LanguageRegistry;

use super::{CommandPane, apply_pane_edit, ensure_writable, pane_view};
use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::error::CommandError;
use crate::editor::registry::Count;
use crate::editor::syntax::{SyntaxUse, ensure_syntax_current};

/// Comment or uncomment the lines of each selection (`G G`).
pub(in crate::editor) fn cmd_toggle_comment(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ensure_writable(state, view, t)?;
    let bid = t.bid(view);
    ensure_syntax_current(state, bid, SyntaxUse::InjectedLanguages);
    let buf = state.buffers.get(bid);
    let multi_line = if buf.overrides.prefer_line_comments(&state.settings) {
        MultiLineComments::Line
    } else {
        MultiLineComments::Block
    };
    let tab_width = buf.overrides.tab_width(&state.settings);
    let plan = plan_comment_toggle(pane_view(state, view, t), multi_line, tab_width, |span| {
        comment_tokens_at(buf, &state.config.languages, span)
    })
    .map_err(|e| match e {
        CommentPlanError::Tokens(e) => e,
        CommentPlanError::BlockEndInside(end) => CommandError::transient(format!(
            "Cannot comment: the text already holds the block comment end '{end}'"
        )),
    })?;
    apply_pane_edit(state, view, t, move |edit| apply_comment_plan(edit, plan))
}

/// The tokens of the innermost layer holding `span` whose language has
/// any. Without a parse of the current text, the buffer's own language.
fn comment_tokens_at<'a>(
    buf: &Buffer,
    languages: &'a LanguageRegistry,
    span: ClusterRange,
) -> Result<CommentTokens<'a>, CommandError> {
    let text = buf.text();
    let layered = buf
        .syntax
        .as_ref()
        .and_then(|syn| syn.current_layers(text.generation()))
        .and_then(|layers| {
            let chars = span.chars();
            let bytes = text.char_to_byte(chars.start)..text.char_to_byte(chars.end);
            layers
                .languages_containing(bytes)
                .into_iter()
                .find_map(|id| languages.comment_tokens_of(id))
        });
    if let Some(tokens) = layered {
        return Ok(tokens);
    }
    let Some(id) = buf.language else {
        return Err(CommandError::transient(
            "No comment tokens: buffer has no language",
        ));
    };
    languages.comment_tokens_of(id).ok_or_else(|| {
        CommandError::transient(format!(
            "No comment tokens for language '{}'",
            languages.name_of(id)
        ))
    })
}
