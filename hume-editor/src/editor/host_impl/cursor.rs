//! `EditorHostImpl`'s live cursor/selection reads.

use hume_engine::pipeline::BufferId;
use hume_rope::offset::CharOffset;

use crate::editor::commands::effective_word_chars;

use super::EditorHostImpl;
use hume_scripting::host::CursorHost;

impl<'a> EditorHostImpl<'a> {
    /// The focused pane's own buffer state — the focused buffer as seen in
    /// the focused pane, never a caller-supplied `bid`. Shared by every
    /// `CursorHost` method that reads "the current cursor/selections" with
    /// no `bid` parameter of its own to resolve against.
    fn focused_pane_buffer_state(&self) -> Option<&crate::editor::pane_state::PaneBufferState> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        self.state.focused_buffer_state(buf_id)
    }

    /// `bid`'s buffer and selections as seen in the pane currently showing
    /// it (see `EditorState::shown_buffer_state`). Shared by every
    /// `CursorHost` method that reads selection geometry for an arbitrary
    /// caller-supplied `bid`, which may name a buffer in a non-focused pane.
    fn buffer_and_selections(
        &self,
        bid: BufferId,
    ) -> Option<(
        &crate::editor::buffer::Buffer,
        &hume_editing::selection::SelectionSet,
    )> {
        Some((
            self.buffer(bid)?,
            &self.state.shown_buffer_state(self.view, bid)?.selections,
        ))
    }

    /// `true` if every *unambiguous* selection in `bid`'s state satisfies `pred`
    /// (see `hume_editing::selection::linewise_classification`) — a selection
    /// collapsed on an empty line carries no vote either way and is skipped.
    /// A set where every selection is ambiguous votes `pred(false)`: it reads as
    /// charwise, the default a bare collapsed cursor already gets. Deriving that
    /// from `pred` rather than taking it separately is what keeps the "exactly
    /// one of these, or neither (mixed)" contract callers rely on true by
    /// construction — `Iterator::all` alone would agree `true` with both
    /// polarities over an empty sequence. `false` if `bid` isn't shown in any
    /// pane. Shared by `selections_linewise` and `selections_charwise`, which
    /// differ only in `pred`'s polarity.
    fn all_unambiguous_selections(&self, bid: BufferId, pred: impl Fn(bool) -> bool) -> bool {
        self.buffer_and_selections(bid).is_some_and(|(buf, sels)| {
            let text = buf.text();
            let mut classified = sels
                .iter_sorted()
                .filter_map(|sel| hume_editing::selection::linewise_classification(text, sel))
                .peekable();
            if classified.peek().is_none() {
                pred(false)
            } else {
                classified.all(pred)
            }
        })
    }
}

impl<'a> CursorHost for EditorHostImpl<'a> {
    fn current_line_number(&self) -> Option<usize> {
        let pbs = self.focused_pane_buffer_state()?;
        self.char_index_to_line(pbs.selections.primary().head().index())
    }

    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>> {
        let pbs = self.focused_pane_buffer_state()?;
        let primary_index = pbs.selections.primary_index();
        Some(
            pbs.selections
                .iter_sorted()
                .enumerate()
                .map(|(i, sel)| (sel.anchor().index(), sel.head().index(), i == primary_index))
                .collect(),
        )
    }

    fn char_index_to_line(&self, idx: usize) -> Option<usize> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        let text = self.buffer(buf_id)?.text();
        // `CharOffset::checked` accepts `idx == len_chars()` (only `>` rejects)
        // — the one Steel line-index builtin that admits the buffer's own
        // trailing phantom line, so this goes through the ropey domain rather
        // than `char_to_line`'s content-only contract.
        let offset = CharOffset::checked(text.rope(), idx)?;
        Some(text.ropey_char_to_line(offset).index() + 1)
    }

    fn symbol_under_cursor(&self, bid: BufferId) -> String {
        let Some((buf, sels)) = self.buffer_and_selections(bid) else {
            return String::new();
        };
        let text = buf.text();
        let head = sels.primary().head();
        let Some(ch) = text.char_at(head) else {
            return String::new();
        };
        let chars = effective_word_chars(buf, &self.state.settings);
        if chars.classify(ch) != hume_editing::word::CharClass::Word {
            return String::new();
        }
        let Some(range) = hume_ops::text_object::inner_word_impl(
            text,
            head,
            hume_editing::word::is_word_boundary,
            chars,
        ) else {
            return String::new();
        };
        text.slice(range.to_exclusive()).to_string()
    }

    fn selections_linewise(&self, bid: BufferId) -> bool {
        self.all_unambiguous_selections(bid, |linewise| linewise)
    }

    fn selections_charwise(&self, bid: BufferId) -> bool {
        self.all_unambiguous_selections(bid, |linewise| !linewise)
    }
}
