//! What a recorded change holds: the command that began it, its count and
//! selection recipe, the answers its interactive inputs got, and the
//! transcript of its Insert-session input.

use std::borrow::Cow;

use termina::event::KeyEvent;

use super::{BindingId, SelectionStep};
use crate::editor::keymap::CanonicalKey;

/// The user's choice at one interactive input: a character read, a picker,
/// a prompt or a menu. Its place is taken when the input opens, so answers
/// stay in the order their inputs opened, which is the order a re-run opens
/// them in; it is filled when the input resolves. Handed back to the same
/// input when `.` re-runs the command or binding that opened it, which then
/// never waits for the user. What the command does with the
/// choice is decided again at the new cursor.
#[derive(Debug, Clone)]
pub(crate) enum Answer {
    /// A character: a native operand (`r`/`f`/`t`/`surround-add`) or an
    /// `on-next-char!` answer. `None` when the read ended without one.
    Char(Option<char>),
    /// A picker pick or dismissal.
    Pick(Pick),
    /// A prompt's confirmed text, `None` when it was cancelled.
    Prompt(Option<String>),
    /// A menu's chosen item, `None` when it was dismissed.
    Menu(Option<Choice>),
    /// An input that opened but never resolved: one a reload or a cascade
    /// cap dropped, or a menu `close-menu!` closed. Its callback never ran,
    /// so on replay the input opens nothing and runs nothing.
    Unanswered,
}

/// A picker's answer. `action` is the `#:actions` key that closed the
/// picker, `None` for `on-select`; `picked` is `None` for a dismissal (the
/// callback got `#f`).
#[derive(Debug, Clone)]
pub(crate) struct Pick {
    pub(in crate::editor) action: Option<CanonicalKey>,
    pub(in crate::editor) picked: Option<Picked>,
}

/// How `.` hands a picked item back to the re-run picker, decided when the
/// pick is made.
#[derive(Debug, Clone)]
pub(crate) enum Picked {
    /// By its text, when every item came from the list `picker!` opened
    /// with: the re-run builds that list again, so the payload is the one
    /// built for the new spot.
    Choice(Choice),
    /// By the payload the callback got, when items arrived after the picker
    /// opened (`#:pending`, `picker-push!`, a spawned source, a
    /// `live-picker!`): the re-run picker holds none of them yet when `.`
    /// answers it.
    Payload(hume_scripting::Rooted),
    /// A `Payload` a reload forgot: it belonged to the Steel engine the
    /// reload replaced. `.` refuses it.
    PayloadForgotten,
}

/// A picked menu or picker item, named by its text: the `nth` (from 0, in
/// list order) of the items shown as `label`. Replayed against the list the
/// re-run builds, so the pick follows the item's text rather than its
/// position, and of several items with the same text it is the same one.
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    pub(crate) label: String,
    pub(crate) nth: usize,
}

impl Choice {
    /// The choice naming `items[index]`, each item shown as `text` gives it.
    pub(in crate::editor) fn of<T>(items: &[T], index: usize, text: impl Fn(&T) -> &str) -> Self {
        let label = text(&items[index]);
        Self {
            label: label.to_owned(),
            nth: items[..index]
                .iter()
                .filter(|item| text(item) == label)
                .count(),
        }
    }

    /// The index of the item this choice names in `items`, `None` when
    /// fewer than `nth + 1` items are shown as `label`.
    pub(in crate::editor) fn find<T>(
        &self,
        items: &[T],
        text: impl Fn(&T) -> &str,
    ) -> Option<usize> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| text(item) == self.label)
            .nth(self.nth)
            .map(|(i, _)| i)
    }

    /// The refusal `what` raises during `.` when the re-run list has no item
    /// this choice names.
    pub(in crate::editor) fn missing(&self, what: &str) -> String {
        let label = &self.label;
        match self.nth {
            0 => format!("{what}: `.` found no item \"{label}\" to pick"),
            nth => format!(
                "{what}: `.` found no item \"{label}\" number {} among those with that text",
                nth + 1
            ),
        }
    }
}

/// One Insert-session input a change consumed, in arrival order.
///
/// `.` replays a change by re-running its command and then re-feeding each
/// transcript entry into the funnel live input goes through, never by
/// inferring, after a dispatch, what it turned out to do. The one exception
/// is a completion accept ([`Input::Result`]), recorded as its own net
/// cursor edit, since it hands no choice to a callback that could take it
/// again.
#[derive(Debug, Clone)]
pub(crate) enum Input {
    /// A key with no Insert keymap binding: one run of its *default*
    /// behaviour (`commands::insert_default_key`), never a keymap walk.
    Key(KeyEvent),
    /// A paste
    /// ([`EditorState::record_paste`](crate::editor::EditorState::record_paste)),
    /// replayed as one bulk insert rather than synthetic per-char keys: a
    /// synthesized `Enter` would auto-indent, which a real paste never does.
    Paste(Vec<String>),
    /// A key bound in the Insert keymap, re-run on replay through
    /// [`Editor::dispatch`](crate::editor::Editor::dispatch) as the live key
    /// was, so it decides again at the new cursor, its own `call!`s, register
    /// prefix, and `#:extend` included. Its interactive inputs are answered
    /// from `answers`. A binding whose command is not repeatable has none: an
    /// input it opens ends the change and drops this entry.
    Binding {
        id: BindingId,
        name: Cow<'static, str>,
        answers: Vec<Answer>,
    },
    /// The net edit a completion accept produced at the cursor: a completion
    /// popup's Enter, or an Insert-key binding that called
    /// `completion-accept!` (in place of that binding's own entry). Replayed
    /// by applying the edit directly.
    Result(CursorReplacement),
}

/// The net text a completion accept wrote at the cursor: `back` chars behind
/// and `forward` ahead of the head, replaced by `text`. The primary cursor's
/// own counts, applied uniformly at every cursor on replay. Anything
/// document-absolute (a completion's `additionalTextEdits`) is excluded: it
/// has no meaning at a different cursor.
#[derive(Debug, Clone)]
pub(crate) struct CursorReplacement {
    pub(in crate::editor) back: usize,
    pub(in crate::editor) forward: usize,
    pub(in crate::editor) text: String,
}

/// A recorded editing action that can be replayed by `.`.
///
/// Stores the recipe to re-execute a command rather than the raw changeset:
/// changesets are position-dependent and can't be replayed at a different
/// cursor.
#[derive(Debug, Clone)]
pub(crate) struct RepeatableAction {
    /// The command name that began this change (e.g. `"delete"`, `"change"`).
    /// `Cow::Borrowed` for built-ins (zero allocation); `Cow::Owned` for a
    /// dynamically-registered (Steel) command.
    pub command: Cow<'static, str>,
    /// The count prefix the change ran with. Overridden when `.` itself is given a
    /// count.
    pub count: usize,
    /// Selection-building recipe to replay BEFORE the command.
    ///
    /// `[]` (the command acted on a pre-existing selection), or a sequence of
    /// `Establishes`/`Composes` steps; see `SelectionTracking`. Taken from
    /// `EditorState::selection_recipe` when the change starts.
    pub selection_recipe: Vec<SelectionStep>,
    /// The answers the command's own interactive inputs got, in the order
    /// their inputs opened. See [`Answer`].
    pub answers: Vec<Answer>,
    /// Every Insert-session input the change consumed, in arrival order.
    /// See [`Input`].
    pub transcript: Vec<Input>,
}
