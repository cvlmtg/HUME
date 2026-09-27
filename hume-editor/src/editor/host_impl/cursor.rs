//! `EditorHostImpl`'s live cursor/selection reads.

use hume_rope::offset::CharOffset;

use crate::editor::commands::CommandPane;
use crate::editor::commands::effective_word_chars;

use super::EditorHostImpl;
use hume_scripting::PaneHandle;
use hume_scripting::host::CursorHost;

impl<'a> EditorHostImpl<'a> {
    /// `t`'s buffer and selections, as tracked in `t`'s own pane. Shared by
    /// every `CursorHost` method: `t` is already resolved (see
    /// `commands::CommandPane::resolve`), so this never fails; a resolved
    /// `CommandPane`'s own buffer is always seeded (every pane creation or
    /// buffer switch seeds its `PaneBufferState`).
    fn buffer_and_selections(
        &self,
        t: CommandPane,
    ) -> (
        &crate::editor::buffer::Buffer,
        &hume_editing::selection::SelectionSet,
    ) {
        let bid = t.bid(self.view);
        (
            self.buffer(bid).expect("resolved CommandPane's own buffer"),
            t.state(&self.state.panes.state, self.view).selections(),
        )
    }

    /// `true` if every *unambiguous* selection in `t`'s pane satisfies `pred`
    /// (see `hume_editing::selection::linewise_classification`); a selection
    /// collapsed on an empty line carries no vote either way and is skipped.
    /// A set where every selection is ambiguous votes `pred(false)`: it reads as
    /// charwise, the default a bare collapsed cursor already gets. Deriving that
    /// from `pred` rather than taking it separately is what keeps the "exactly
    /// one of these, or neither (mixed)" contract callers rely on true by
    /// construction, since `Iterator::all` alone would agree `true` with both
    /// polarities over an empty sequence.
    fn all_unambiguous_selections(&self, t: CommandPane, pred: impl Fn(bool) -> bool) -> bool {
        let (buf, sels) = self.buffer_and_selections(t);
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
    }
}

impl<'a> CursorHost for EditorHostImpl<'a> {
    fn buffer_cursor_line(&self, pane: PaneHandle) -> Result<usize, String> {
        let t = self.command_pane(pane)?;
        let (_, sels) = self.buffer_and_selections(t);
        let bid = t.bid(self.view);
        Ok(self
            .offset_to_line(bid, sels.primary().head().index())
            .expect("resolved CommandPane's own head offset is always in range"))
    }

    fn buffer_selections(&self, pane: PaneHandle) -> Result<Vec<(usize, usize, bool)>, String> {
        let t = self.command_pane(pane)?;
        let (_, sels) = self.buffer_and_selections(t);
        let primary_index = sels.primary_index();
        Ok(sels
            .iter_sorted()
            .enumerate()
            .map(|(i, sel)| (sel.anchor().index(), sel.head().index(), i == primary_index))
            .collect())
    }

    fn offset_to_line(&self, bid: hume_engine::pipeline::BufferId, idx: usize) -> Option<usize> {
        let text = self.buffer(bid)?.text();
        // `CharOffset::checked` accepts `idx == len_chars()` (only `>` rejects):
        // the one Steel line-index builtin that admits the buffer's own
        // trailing phantom line, so this goes through the ropey domain rather
        // than `char_to_line`'s content-only contract.
        let offset = CharOffset::checked(text.rope(), idx)?;
        Some(text.ropey_char_to_line(offset).index() + 1)
    }

    fn symbol_under_cursor(&self, pane: PaneHandle) -> Result<String, String> {
        let t = self.command_pane(pane)?;
        let (buf, sels) = self.buffer_and_selections(t);
        let text = buf.text();
        let head = sels.primary().head();
        let Some(ch) = text.char_at(head) else {
            return Ok(String::new());
        };
        let chars = effective_word_chars(buf, &self.state.settings);
        if chars.classify(ch) != hume_editing::word::CharClass::Word {
            return Ok(String::new());
        }
        let Some(range) = hume_ops::text_object::inner_word_impl(
            text,
            head,
            hume_editing::word::is_word_boundary,
            chars,
        ) else {
            return Ok(String::new());
        };
        Ok(text.slice(range.to_exclusive()).to_string())
    }

    fn selections_linewise(&self, pane: PaneHandle) -> Result<bool, String> {
        let t = self.command_pane(pane)?;
        Ok(self.all_unambiguous_selections(t, |linewise| linewise))
    }

    fn selections_charwise(&self, pane: PaneHandle) -> Result<bool, String> {
        let t = self.command_pane(pane)?;
        Ok(self.all_unambiguous_selections(t, |linewise| !linewise))
    }
}
