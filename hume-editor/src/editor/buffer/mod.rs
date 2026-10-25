use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use super::search::{SearchMatches, SearchPattern};
use crate::editor::edit_session::EditGroup;
use crate::editor::position_stores::PositionStores;
use crate::editor::settings::BufferOverrides;
use hume_editing::changeset::{ChangeSet, changesets_from_line_diff};
use hume_editing::edit::{Edited, TextChange};
use hume_editing::history::{History, RevisionId};
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::{BufferText, LineEnding};
use hume_editing::transaction::Transaction;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_platform::path::AbsolutePath;

mod disk;
mod dump;
// Sibling buffer submodules (`file_open::enter_buffer`,
// `Buffer::disk_state`'s field type) reach `disk::{DiskCheckTrigger,
// DiskState}` via `super::disk::` instead, staying inside the `buffer`
// module tree. `DiskCheckTrigger` and `ReloadError` are re-exported
// unconditionally because `commands::typed_buffer`, outside the tree, needs
// them: `typed_checktime` names `DiskCheckTrigger::Explicit` as its own
// trigger, and `typed_edit` tells a reload's two failures apart. `DiskState`
// stays `#[cfg(test)]`-only since only test code (a different module tree
// entirely, passing a trigger to `check_buffer_disk_state` directly or
// injecting a `DiskState` value to test the state machine without depending
// on filesystem mtime precision) needs it outside `buffer` itself.
pub(in crate::editor) use buffer_file::{BufferFile, FileTarget};
#[cfg(test)]
pub(in crate::editor) use disk::DiskState;
pub(in crate::editor) use disk::{DiskCheckTrigger, ReloadError};
#[cfg(test)]
pub(in crate::editor) use dump::dump_path_for;
pub(crate) use dump::{print_worker_panics, report_dumps};
pub(in crate::editor) use file_open::ReplaceSource;
pub(in crate::editor) use history::{OnUnsaved, Unsaved};
mod buffer_file;
mod file_open;
mod history;
pub(in crate::editor) mod lifecycle;
pub(in crate::editor) mod offsets;
pub(in crate::editor) mod store;
use hume_treesitter::layers::SyntaxLayers;
use hume_treesitter::registry::LanguageId;
use hume_treesitter::syntax::Syntax;

/// A read-only view the editor itself opens. Its name is reserved:
/// [`ViewLabel::named`] refuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum EditorView {
    Messages,
    PluginStatus,
    LspStatus,
}

/// Generates `EditorView::name`'s match and `EditorView::ALL` from one
/// list, so a variant missing from `ALL` is also missing from `name`'s
/// match and fails to compile.
macro_rules! editor_views {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        impl EditorView {
            const ALL: &[Self] = &[$(Self::$variant),+];

            fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }
        }
    };
}

editor_views! {
    Messages => "messages",
    PluginStatus => "plugin-status",
    LspStatus => "lsp-status",
}

/// The label of a read-only view buffer, `[name]`. A plugin's view can
/// never take the label of one of the editor's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::editor) struct ViewLabel(String);

impl ViewLabel {
    pub(in crate::editor) fn editor(view: EditorView) -> Self {
        Self::bracketed(view.name())
    }

    /// Errors on an empty `name` or an [`EditorView`]'s name.
    pub(in crate::editor) fn named(name: &str) -> Result<Self, String> {
        if name.is_empty() {
            return Err("the name must not be empty".into());
        }
        if EditorView::ALL.iter().any(|v| v.name() == name) {
            return Err(format!("[{name}] is one of the editor's own views"));
        }
        Ok(Self::bracketed(name))
    }

    fn bracketed(name: &str) -> Self {
        Self(format!("[{name}]"))
    }

    pub(in crate::editor) fn as_str(&self) -> &str {
        &self.0
    }
}

// ── Buffer ────────────────────────────────────────────────────────────────────

/// What one composed undo/redo walk hands back. See
/// [`Buffer::apply_transactions`]'s doc for each part's contract. Named
/// once here so `undo_n`/`redo_n`, `doc_ops::apply_doc_history_walk`, and
/// `commands::edit::history_step` all spell the same shape.
pub(in crate::editor) type HistoryWalkResult = Option<(SelectionSet, ChangeSet, usize)>;

/// The part of a buffer's undo state `on-undo-history-changed` reports: the
/// history's `change_seq` and the revision `buffer-undo-tree` marks `saved?`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct HistoryMark {
    seq: u64,
    saved: Option<RevisionId>,
}

impl HistoryMark {
    fn of(history: &History, saved: Option<RevisionId>) -> Self {
        Self {
            seq: history.change_seq(),
            saved,
        }
    }
}

/// What a buffer's text belongs to.
enum Origin {
    /// Nothing: `*scratch*`.
    Scratch,
    File(BufferFile),
    /// A read-only view of text the editor or a plugin built, such as
    /// `[messages]`, named by its label in the statusline and `:ls`.
    View(ViewLabel),
}

/// Content-only document: text, undo history, search state, and per-buffer overrides.
///
/// `Buffer` is the SSOT for everything intrinsic to an open file and shared
/// across all panes viewing it. It does **not** own:
/// - selections (per-(pane, buffer), live on `PaneBufferState`)
/// - viewport / scroll (per-pane, live on engine `Pane`)
/// - per-pane search cursor (live on `PaneBufferState`)
/// - edit groups / insert sessions (per-(pane, buffer), live on `PaneBufferState`)
///
/// ## Edit API
///
/// All text mutations funnel through [`Self::install`], which carries every
/// position stored against the text through the change: each mutator takes
/// the [`PositionStores`] for that reason, so text cannot change without
/// them. The three edit entry points (`apply_edit`, `apply_edit_grouped`,
/// `apply_edit_regrouped`) each return the acting pane's post-edit
/// `SelectionSet` + the `ChangeSet` it applied, and handle undo bookkeeping
/// internally. While a [`TextHold`] on the buffer is alive, `install`
/// panics: every gate that would change the text refuses first, through
/// [`Self::text_refusal`].
pub(crate) struct Buffer {
    text: BufferText,
    history: History,
    /// The revision at which the buffer was last saved (or first opened).
    /// `None` means the saved state was overwritten by an `undo-levels`
    /// promotion and no longer exists anywhere in the tree: the buffer is
    /// dirty until the next save.
    saved_revision: Option<RevisionId>,
    /// The line ending at the last save (or first open). A line-ending change
    /// records no revision, so unsaved-ness compares this against the text's.
    saved_line_ending: LineEnding,
    /// The file or view the text belongs to, if any.
    origin: Origin,
    /// Active search pattern shared by all panes viewing this buffer.
    /// `None` when no search is active. A present `SearchPattern` is always
    /// fully-valid. Invalid regexes leave this as `None`.
    pub(in crate::editor) search_pattern: Option<SearchPattern>,
    /// Cached match list for `search_pattern`. Invalidated by revision change
    /// or pattern change; rebuilt lazily by `update_buffer_matches`.
    pub(in crate::editor) search_matches: SearchMatches,
    /// How many times `update_buffer_matches` has scanned this buffer for
    /// matches. Clearing the cache leaves it as it is.
    #[cfg(test)]
    pub(in crate::editor) search_scans: usize,
    /// Per-buffer setting overrides. `None` fields inherit from
    /// [`crate::editor::settings::EditorSettings`].
    pub(in crate::editor) overrides: BufferOverrides,
    /// Detected or explicitly set language identity (e.g. `rust`, `json`).
    /// `None` for unrecognised filetypes and scratch buffers.
    pub(crate) language: Option<LanguageId>,
    /// `true` when `language` was written by `:set buffer language=` or Steel's
    /// `set-buffer-option!`'s `"language"`, rather than by detection. `:reload-config`'s
    /// reset reads this (before clearing it) to restore the user's own
    /// assertion across the reload instead of letting re-detection silently
    /// pick something else. See `clear_languages_all`.
    pub(in crate::editor) language_explicit: bool,
    /// The text generation most recently announced as an `on-text-changed`
    /// event. `Buffer` cannot reach the event queue (it holds no
    /// `EditorState`), so the hook is raised by diffing this against the
    /// text's generation at a drain observation point (see
    /// `BufferStore::take_text_changed`) rather than at `install` itself.
    pub(in crate::editor) announced_generation: u64,
    /// The history state most recently announced as an
    /// `on-undo-history-changed` event, diffed the same way
    /// `announced_generation` is (see `BufferStore::take_history_changed`).
    pub(in crate::editor) announced_history: HistoryMark,
    /// Per-buffer tree-sitter syntax attachment: grammar identity, committed
    /// parse layers, generation bookkeeping, and in-flight state, all in one
    /// place. `None` when no grammar is attached or the buffer exceeds
    /// `syntax-highlight-max-bytes`.
    pub(in crate::editor) syntax: Option<Syntax>,
    /// When `true`, all forward text mutations are blocked at the `doc_ops`
    /// layer. Entering Insert mode is also refused. Read-only is orthogonal to
    /// language/syntax: a read-only buffer may still be highlighted.
    pub(in crate::editor) read_only: bool,
    /// The [`TextHold`] keeping the text as it is, if one is alive.
    hold: Weak<()>,
    /// True from chokepoint open (`lifecycle::open_buffer_and_notify`, or
    /// `lifecycle::close_buffer_and_notify`'s own `queue_open_announcement`
    /// call for the fresh scratch buffer a last-buffer close allocates)
    /// until `Editor::detect_pending_languages` fires this buffer's
    /// `OnBufferOpen`. Read by `lifecycle::close_buffer_and_notify`: a still-
    /// pending buffer (opened and closed before that drain ran, e.g. within
    /// one Steel eval) announced no open, so it must announce no close
    /// either. The startup buffer, built inline by `Editor::open` (which
    /// can't call the chokepoint, since it's what bootstraps the very
    /// `EngineView`/pane-state maps the chokepoint needs), is the one buffer
    /// that defaults to `false`: its close always announces.
    pub(in crate::editor) open_hook_pending: bool,
    /// The buffer's disk state as of the last check, set by
    /// `Editor::check_buffer_disk_state`, cleared to `InSync` by a reload or
    /// a successful write. Always `InSync` for scratch/synthetic buffers,
    /// which the check skips.
    pub(in crate::editor) disk_state: disk::DiskState,
    /// The crash dump found next to this buffer's file when it was opened
    /// (`from_file_or_new`, not a reload), until the user answers the
    /// restore prompt for it. Kept as a path, not re-derived from `path`: a
    /// save-as changes `path` but not which dump the prompt is about.
    pub(in crate::editor) pending_dump: Option<PathBuf>,
}

/// While alive, nothing may change its buffer's text: the work that owns
/// it will land on the text as it is. Minted only by [`Buffer::hold_text`];
/// dropping it releases the buffer.
#[must_use]
#[derive(Debug)]
pub(in crate::editor) struct TextHold {
    _alive: Rc<()>,
}

/// Why a buffer's text may not change now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum TextRefusal {
    ReadOnly,
    /// A [`TextHold`] is alive.
    Busy,
}

impl TextRefusal {
    /// What refusing a change to `buffer`'s text reports. A busy buffer is
    /// named: the change can come from a script, a timer or a language
    /// server, for a buffer not in view.
    pub(in crate::editor) fn message(self, buffer: &Buffer) -> String {
        match self {
            Self::ReadOnly => "Buffer is read-only".to_string(),
            Self::Busy => format!("{} is busy: a pipe is running", buffer.display_name()),
        }
    }
}

/// How a new text relates to the one it replaces, for [`Buffer::install`].
enum Change<'a> {
    /// The new text is the old one with this change applied.
    Edit(&'a ChangeSet),
    /// The new text replaces the old one wholesale.
    Replace,
}

impl Buffer {
    /// Display name used for buffers that have no backing file.
    pub(in crate::editor) const SCRATCH_BUFFER_NAME: &'static str = "*scratch*";

    /// The syntax layers parsed from the buffer's current text; `None` when
    /// it has no syntax or the tree describes an older text.
    pub(in crate::editor) fn current_layers(&self) -> Option<&SyntaxLayers> {
        self.syntax
            .as_ref()
            .and_then(|syn| syn.current_layers(self.text().generation()))
    }

    /// Create a new buffer holding `text`. A pane showing it starts with a
    /// cursor on its first cluster.
    pub(crate) fn at_start(text: BufferText) -> Self {
        let history = History::new();
        let saved_revision = Some(history.current_id());
        let saved_line_ending = text.line_ending();
        let announced_generation = text.generation();
        let announced_history = HistoryMark::of(&history, saved_revision);
        Self {
            text,
            history,
            saved_revision,
            saved_line_ending,
            origin: Origin::Scratch,
            search_pattern: None,
            search_matches: SearchMatches::default(),
            #[cfg(test)]
            search_scans: 0,
            overrides: BufferOverrides::default(),
            language: None,
            language_explicit: false,
            announced_generation,
            announced_history,
            syntax: None,
            read_only: false,
            hold: Weak::new(),
            open_hook_pending: false,
            disk_state: disk::DiskState::InSync,
            pending_dump: None,
        }
    }

    /// Load the file the user named `typed` from disk, returning a
    /// ready-to-use `Buffer` whose file is the one the read resolved.
    /// `search_pattern` and `search_matches` are left at their defaults (no
    /// active search).
    pub(in crate::editor::buffer) fn from_file(typed: &AbsolutePath) -> io::Result<Self> {
        let (text, file) = BufferFile::read(typed.clone())?;
        let mut buf = Self::at_start(text);
        buf.origin = Origin::File(file);
        Ok(buf)
    }

    /// Empty buffer bound to a file that doesn't exist on disk yet: `:e` on
    /// a missing file, matching Vim's `:e newfile` semantics: `:w` creates it
    /// (see `is_new_file`). The file is set so the buffer participates in
    /// `find_by_path` dedup and displays its intended name.
    pub(in crate::editor::buffer) fn new_file(file: BufferFile) -> Self {
        let mut buf = Self::at_start(BufferText::empty());
        buf.origin = Origin::File(file);
        buf
    }

    /// `true` for a buffer bound to a file not on disk yet, opened via
    /// [`Self::new_file`] and not yet written.
    pub(in crate::editor) fn is_new_file(&self) -> bool {
        self.file().is_some_and(|file| file.file_meta().is_none())
    }

    /// Load the file `target` names, or open an empty [`Self::new_file`]
    /// buffer bound to it if it doesn't exist yet (Vim's `:e newfile`
    /// semantics: `:w` creates it). A missing file's resolved path is the
    /// target's, the form `find_by_path` dedup looks it up by.
    ///
    /// The single decision point for "is a missing path openable", shared by
    /// every open chokepoint, so they can't diverge on tolerance. A path with
    /// no file name (`/`) errors (see [`BufferFile::new_file`]).
    pub(in crate::editor) fn from_file_or_new(target: FileTarget) -> io::Result<Self> {
        let mut buf = match Self::from_file(target.typed()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Self::new_file(target.into_new_file()?)
            }
            other => other?,
        };
        buf.pending_dump = buf
            .path()
            .map(AbsolutePath::as_path)
            .map(dump::dump_path_for)
            .filter(|dump| std::fs::symlink_metadata(dump).is_ok());
        Ok(buf)
    }

    /// Empty scratch buffer (single structural `\n`, no path, default overrides).
    ///
    /// Used when closing the last buffer to keep the "always ≥1 buffer open"
    /// invariant without leaving the editor in an invalid state.
    pub(in crate::editor) fn scratch() -> Self {
        Self::at_start(BufferText::empty())
    }

    /// Create a read-only view buffer from in-memory content, for
    /// [`lifecycle::show_read_only_view`]. The buffer has no backing file,
    /// no language detection, and blocks all user edits.
    pub(in crate::editor) fn read_only_view(text: BufferText, label: ViewLabel) -> Self {
        let mut buf = Self::at_start(text);
        buf.read_only = true;
        buf.origin = Origin::View(label);
        buf
    }

    /// Replace the content of a read-only view buffer with new text.
    ///
    /// Resets history to a clean root and clears search state so the refreshed
    /// buffer is non-dirty and has no stale match data. This is a system
    /// refresh: it intentionally bypasses the `read_only` guard in `doc_ops`.
    pub(in crate::editor) fn set_view_content(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        content: &str,
    ) {
        let text = self.text.replaced_with(content);
        self.history.reset();
        self.saved_revision = Some(self.history.current_id());
        self.saved_line_ending = text.line_ending();
        self.search_pattern = None;
        self.search_matches = SearchMatches::default();
        self.install(id, stores, text, Change::Replace, None);
    }

    /// `true` when the buffer blocks user edits.
    pub(crate) fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// `true` while a [`TextHold`] on this buffer is alive.
    pub(in crate::editor) fn is_busy(&self) -> bool {
        self.hold.strong_count() > 0
    }

    /// Why the text may not change now, `None` when it may.
    pub(in crate::editor) fn text_refusal(&self) -> Option<TextRefusal> {
        if self.read_only {
            Some(TextRefusal::ReadOnly)
        } else if self.is_busy() {
            Some(TextRefusal::Busy)
        } else {
            None
        }
    }

    /// Hold the text as it is until the returned hold drops.
    ///
    /// # Panics
    /// Panics when the text may not change now: a hold is for work that
    /// will change it.
    pub(in crate::editor) fn hold_text(&mut self) -> TextHold {
        assert_eq!(
            self.text_refusal(),
            None,
            "hold_text: only a writable, unheld buffer can be held"
        );
        let alive = Rc::new(());
        self.hold = Rc::downgrade(&alive);
        TextHold { _alive: alive }
    }

    /// `true` for in-memory view buffers (e.g. `[messages]`, `[plugin-status]`).
    pub(in crate::editor) fn is_synthetic(&self) -> bool {
        matches!(self.origin, Origin::View(_))
    }

    /// Replace the buffer text and carry every stored position with it:
    /// through the edit's `ChangeSet`, or, for a replacement no change
    /// describes, by resetting them to the buffer's initial state. All text-mutating paths go through here,
    /// each with a text of a later generation, which is how
    /// `reparse_stale_buffers` and the `on-text-changed` announcer see that
    /// the text moved. The one exception is an undo walk that nets to no
    /// change: the same text under the version its selections were recorded
    /// for, at the same generation, carried through an identity change so
    /// every store is retagged while nothing that orders texts sees a change.
    fn install(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        text: BufferText,
        change: Change<'_>,
        acting: Option<PaneId>,
    ) {
        assert!(!self.is_busy(), "install: a held buffer's text changed");
        let relabel = text.generation() == self.text.generation();
        debug_assert!(
            text.generation() > self.text.generation()
                || (relabel
                    && matches!(change, Change::Edit(changes) if changes.is_identity())
                    && text == self.text),
            "install: a replacement text must be a later generation of the buffer's text, \
             or the same text relabeled through an identity change"
        );
        let before = std::mem::replace(&mut self.text, text);
        match change {
            Change::Edit(changes) => {
                let change = TextChange::new(&before, &self.text, changes);
                if !relabel && let Some(syn) = self.syntax.as_mut() {
                    syn.record_edit(&change);
                }
                stores.carry(id, &change, acting);
            }
            Change::Replace => stores.reset(id, || crate::editor::pane_state::fresh_from_buf(self)),
        }
    }

    /// Bind the buffer to `file`. A view buffer is never bound to one: a
    /// write of it is an export (see `WriteKind::classify`).
    pub(in crate::editor) fn set_file(&mut self, file: BufferFile) {
        debug_assert!(!self.is_synthetic(), "a view buffer is bound to a file");
        self.origin = Origin::File(file);
    }

    /// Unbinds the buffer from its file or label and keeps the rest of its
    /// state, for a test that needs a pathless buffer in a state only a file
    /// buffer reaches.
    #[cfg(test)]
    pub(in crate::editor) fn forget_file(&mut self) {
        self.origin = Origin::Scratch;
    }

    /// The backing file, or `None` for scratch/synthetic buffers.
    pub(in crate::editor) fn file(&self) -> Option<&BufferFile> {
        match &self.origin {
            Origin::File(file) => Some(file),
            Origin::Scratch | Origin::View(_) => None,
        }
    }

    /// The label of a view buffer, or `None` for any other.
    pub(in crate::editor) fn label(&self) -> Option<&ViewLabel> {
        match &self.origin {
            Origin::View(label) => Some(label),
            Origin::Scratch | Origin::File(_) => None,
        }
    }

    /// Canonical backing-file path, or `None` for scratch buffers.
    pub(in crate::editor) fn path(&self) -> Option<&AbsolutePath> {
        self.file().map(BufferFile::resolved)
    }

    /// The fully display-ready path string, or `None` for scratch/synthetic
    /// buffers. Print verbatim, no further transforms needed.
    pub(crate) fn display_path(&self) -> Option<&str> {
        self.file().map(BufferFile::display)
    }

    /// First line of buffer content, capped at 64 bytes, stopping at the
    /// line's `\n`.
    /// Returns `None` when the first line is empty. Used for shebang detection.
    /// Iterates codepoints, not grapheme clusters: safe because shebang lines are ASCII-only.
    pub(in crate::editor) fn first_line(&self) -> Option<String> {
        const CAP: usize = 64;
        let mut out = String::with_capacity(CAP);
        for ch in self.text.rope().chars() {
            if ch == '\n' {
                break;
            }
            if out.len() + ch.len_utf8() > CAP {
                break;
            }
            out.push(ch);
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// The selections a pane starts with when it first views this buffer: a
    /// cursor on the first cluster.
    pub(in crate::editor) fn initial_sels(&self) -> SelectionSet {
        EditState::at_text_start(self.text.clone()).into_selections()
    }

    /// The name shown in the UI: label for view buffers, basename for named
    /// buffers, `*scratch*` for unnamed ones.
    pub(crate) fn display_name(&self) -> String {
        match &self.origin {
            Origin::Scratch => Self::SCRATCH_BUFFER_NAME.to_owned(),
            Origin::File(file) => file.file_name().to_string_lossy().into_owned(),
            Origin::View(label) => label.as_str().to_owned(),
        }
    }

    /// Replace `self.text` with `new_text`, recording the swap as a single
    /// revision in the existing history so `u` reverts to the previous text.
    ///
    /// Unlike [`set_view_content`](Self::set_view_content), which resets
    /// history, this treats the replacement as an ordinary edit: `u` shows the
    /// previous text with the full undo tree intact beneath, and `Ctrl-r`
    /// re-applies it. It leaves `saved_revision` and `disk_state` alone, so
    /// the replacement reads as dirty unless it was a no-op.
    ///
    /// The `ChangeSet` pair is line-diff-derived ([`changesets_from_line_diff`])
    /// so the inverse carries only the changed lines, not a full-buffer
    /// delete-all + insert-all. `install` carries every stored position
    /// through the diff. The history revision records `focused`'s selections
    /// for the buffer before and after.
    ///
    /// Returns whether the content changed (`install` ran, the version moved),
    /// `false` when `new_text` has the same content. The line ending is
    /// adopted from `new_text` either way.
    pub(in crate::editor::buffer) fn replace_text_recorded(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        new_text: BufferText,
        focused: PaneId,
    ) -> bool {
        // Build the CS pair from immutable borrows of both texts, before
        // `install` mutates `self.text`. The helper takes `&BufferText` on both
        // sides; `new_text` is still owned by us here so the borrow is fine.
        let (forward, inverse) = changesets_from_line_diff(&self.text, &new_text);

        // Identical text: `self.text` already equals `new_text`, so skip
        // `install` entirely rather than change the version (and fire
        // `on-text-changed` plus a spurious tree-sitter reparse) for a no-op.
        // Nothing is recorded, as there is nothing to undo to.
        if forward.is_identity() {
            self.adopt_line_ending(new_text.line_ending());
            return false;
        }

        // Applying the diff keeps the buffer's lineage, so the replacement is
        // the next generation of the same document rather than a new one.
        // `install` does NOT reset history (`set_view_content` is the only
        // writer that resets history).
        let replaced = forward
            .apply(&self.text)
            .expect("a line diff of the current text applies to it");
        debug_assert_eq!(
            replaced, new_text,
            "replace: the line diff must reproduce the new text"
        );
        let recorded =
            |stores: &PositionStores<'_>| stores.panes.seeded(focused, id).selections().clone();
        let pre_sels = recorded(stores);
        self.install(
            id,
            stores,
            replaced.with_line_ending(new_text.line_ending()),
            Change::Edit(&forward),
            None,
        );
        let post_sels = recorded(stores);
        self.record_revision(forward, inverse, pre_sels, post_sels);
        true
    }

    /// `true` if the buffer has unsaved changes.
    ///
    /// Comparing revision IDs means undoing back to the save point correctly
    /// reports a clean buffer, which a simple `dirty: bool` flag cannot do.
    /// `saved_revision == None` (saved state evicted by promotion) always
    /// reads dirty.
    ///
    /// Tracks revision identity, not byte equality with the saved text: a
    /// history walk that lands on a *different* revision whose text happens
    /// to be byte-identical to the saved one (e.g. undoing an insert and its
    /// own later delete, past the save point) still reads dirty. Making this
    /// content-truthful would mean hashing the buffer on every dirty query
    /// (the statusline makes one every frame) for a case that self-corrects
    /// on the next real edit or save.
    pub(in crate::editor::buffer) fn revision_dirty(&self) -> bool {
        self.saved_revision != Some(self.history.current_id())
    }

    /// Record the current revision as the saved state.
    ///
    /// Call this immediately after a successful file write.
    pub(in crate::editor) fn mark_saved(&mut self) {
        self.saved_revision = Some(self.history.current_id());
        self.saved_line_ending = self.text.line_ending();
        self.disk_state = disk::DiskState::InSync;
    }

    /// Set the line ending a save writes. Not an edit: no revision is
    /// recorded, so undo does not revert it. Refused while the text may not
    /// change ([`Self::text_refusal`]).
    pub(in crate::editor) fn set_line_ending(
        &mut self,
        line_ending: LineEnding,
    ) -> Result<(), String> {
        if let Some(refusal) = self.text_refusal() {
            return Err(refusal.message(self));
        }
        self.adopt_line_ending(line_ending);
        Ok(())
    }

    /// Take the line ending of text read from disk, which a read-only buffer
    /// accepts too.
    fn adopt_line_ending(&mut self, line_ending: LineEnding) {
        assert!(
            !self.is_busy(),
            "adopt_line_ending: a held buffer's text changed"
        );
        self.text = self.text.clone().with_line_ending(line_ending);
    }

    /// `true` if the line ending differs from the one last saved.
    pub(in crate::editor) fn line_ending_changed(&self) -> bool {
        self.text.line_ending() != self.saved_line_ending
    }

    /// `true` if the last disk-state check found the backing file changed or
    /// vanished and the user has not yet acted on it (reloaded or written).
    /// Test-only: `:w`'s write guard reads the file fresh instead of trusting
    /// this. This remains for tests that assert
    /// on the reported/warned state itself, not on write behavior.
    #[cfg(test)]
    pub(in crate::editor) fn is_disk_stale(&self) -> bool {
        !matches!(self.disk_state, disk::DiskState::InSync)
    }

    /// Set the `undo-levels` cap on this buffer's history. `0` means unlimited.
    pub(in crate::editor) fn set_undo_levels(&mut self, levels: usize) {
        self.history.set_undo_levels(levels);
    }

    /// Record a revision, remapping `saved_revision` if `undo-levels`
    /// trimming just promoted it into the new root, and invalidating
    /// `saved_revision` if trimming instead overwrote the root's state out
    /// from under it.
    ///
    /// A revision ID that gets merely evicted (not promoted) needs no
    /// handling: `RevisionId`s are never reused, so `is_dirty()`'s equality
    /// check against a stale `saved_revision` correctly stays `true`
    /// forever. Promotion is the one case that needs explicit handling,
    /// since the promoted node's state is still reachable: it's now what
    /// the root represents. But promotion also *overwrites* the root's
    /// previous state, so a `saved_revision` that pointed at `History::ROOT`
    /// (the buffer was opened, never saved since) no longer names the saved
    /// state at all. It must become `None`, not silently keep pointing at
    /// ROOT's new (different) content.
    fn record_revision(
        &mut self,
        forward: ChangeSet,
        inverse: ChangeSet,
        pre_sels: SelectionSet,
        post_sels: SelectionSet,
    ) {
        if let Some(promoted) = self.history.record(forward, inverse, pre_sels, post_sels) {
            if self.saved_revision == Some(promoted) {
                self.saved_revision = Some(History::ROOT);
            } else if self.saved_revision == Some(History::ROOT) {
                self.saved_revision = None;
            }
        }
    }

    /// `cmd`'s edit of `state`, which must be an edit of this buffer's text.
    ///
    /// # Panics
    /// Panics if `cmd` returns an edit of another text: its changeset and
    /// selections would be installed against the wrong one.
    fn run_edit(&self, state: EditState, cmd: impl FnOnce(EditState) -> Edited) -> Edited {
        let edited = cmd(state);
        assert_eq!(
            edited.base(),
            self.text.version(),
            "an edit of another text was applied to this buffer"
        );
        edited
    }

    /// Apply an edit and record it in the undo history.
    ///
    /// Takes `sels` (the acting pane's current selections) by value and returns
    /// the post-edit selections + the forward `ChangeSet` (for the syntax and
    /// LSP streams); `install` carries every stored position through it.
    /// `acting` is `None` for an edit no pane owns: `sels` is then only what
    /// the revision records, and every pane's stored selections are carried.
    pub(crate) fn apply_edit(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: Option<PaneId>,
        sels: SelectionSet,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (SelectionSet, ChangeSet) {
        let pre_sels = sels.clone();
        let pre = EditState::bind(&self.text, sels);
        let (post, cs) = self.run_edit(pre, cmd).into_parts();

        // An identity `cs` moved no bytes: recording it would litter the undo
        // tree with a no-op revision, and a new text generation would fire
        // `on-text-changed` for a mutation that never happened.
        if cs.is_identity() {
            return (post.into_selections(), cs);
        }

        // self.text is still pre-edit here, so it's safe to call invert.
        let inverse_cs = cs.invert(&self.text);
        let post_text = post.text().clone();
        let post_sels = post.into_selections();
        self.record_revision(cs.clone(), inverse_cs, pre_sels, post_sels.clone());
        self.install(id, stores, post_text, Change::Edit(&cs), acting);
        (post_sels, cs)
    }

    /// Apply an edit that belongs to the open edit group, without recording
    /// a history revision: the caller composes the returned `ChangeSet` into
    /// the group.
    pub(in crate::editor) fn apply_edit_grouped(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        sels: SelectionSet,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (SelectionSet, ChangeSet) {
        let (post, cs) = self
            .run_edit(EditState::bind(&self.text, sels), cmd)
            .into_parts();
        let new_text = post.text().clone();
        let new_sels = post.into_selections();

        // An identity `cs` moved no bytes, but a new text generation would
        // fire `on-text-changed` for one.
        if cs.is_identity() {
            return (new_sels, cs);
        }
        self.install(id, stores, new_text, Change::Edit(&cs), Some(acting));
        (new_sels, cs)
    }

    /// Re-paste from the paste-session snapshot, replacing the accumulated CS.
    ///
    /// Always starts from `snapshot`, the group's, so every cycle cleanly
    /// discards the previous paste output (including added lines).
    /// `to_snapshot` maps the live text back to `snapshot`'s, `None` while
    /// the group has no edits. Returns the new selections, a propagation CS
    /// mapping the current buffer text to the new text, which `install`
    /// carries every stored position through, and the CS from `snapshot`'s
    /// text, the group's new accumulated edits.
    pub(in crate::editor) fn apply_edit_regrouped(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        snapshot: EditState,
        to_snapshot: Option<ChangeSet>,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (SelectionSet, ChangeSet, ChangeSet) {
        let edited = cmd(snapshot);
        let new_cs = edited.changes().clone();

        // With no edits yet the live text is the snapshot's, so propagation
        // CS == new_cs.
        let propagation_cs = match to_snapshot {
            None => new_cs.clone(),
            Some(to_snapshot) => to_snapshot.compose(new_cs.clone()),
        };

        // An identity `propagation_cs` relabels the live text without
        // changing it, so `install` retags the stores and nothing else sees
        // a change.
        let (post, cs) = edited
            .rebased(&self.text, propagation_cs)
            .expect("the propagation changeset maps the live text")
            .into_parts();
        if post.text().version() != self.text.version() {
            self.install(
                id,
                stores,
                post.text().clone(),
                Change::Edit(&cs),
                Some(acting),
            );
        }
        (post.into_selections(), cs, new_cs)
    }

    /// Open an edit group. Snapshots the current text with `base`, the
    /// selections the group's edits start from, so `commit_edit_group` can
    /// invert the composed CS and record one revision, which undoes to
    /// `undo`, the selections the command that opened the group was made
    /// from.
    ///
    /// Returns a fresh [`EditGroup`] rather than writing through a pointer.
    /// Whether a session may open is the caller's check
    /// (`edit_session::check_can_open`), not this one's.
    pub(in crate::editor) fn begin_edit_group(
        &self,
        base: SelectionSet,
        undo: SelectionSet,
    ) -> EditGroup {
        EditGroup {
            snapshot: EditState::bind(&self.text, base),
            undo_sels: EditState::bind(&self.text, undo).into_selections(),
            cs: None,
        }
    }

    /// Close the current edit group and record it as a single undo step.
    ///
    /// If no edits were applied since `begin_edit_group` (empty group), or the
    /// composed `ChangeSet` cancelled out to the identity transform (e.g. type
    /// a char, then backspace it), no revision is recorded. Takes `group` by
    /// value: the caller has already closed the session that held it.
    pub(in crate::editor) fn commit_edit_group(
        &mut self,
        group: EditGroup,
        post_sels: SelectionSet,
    ) {
        // An identity `cs` moved no bytes: recording it would put a no-op
        // revision on the undo stack, and undoing that revision would
        // still consume a step and move `current`. `undo_n`/`redo_n`
        // don't inspect the transaction they're walking, only whether a
        // parent/child link exists, so `u` would consume a step, change
        // no text, and print no exhaustion message either. See
        // `apply_transactions`'s doc for why its own identity guard is a
        // different one, protecting a different case.
        let (Some(inverse_cs), Some(cs)) = (group.inverse(), group.cs) else {
            return;
        };
        let pre_sels = group.undo_sels;
        let post_sels = EditState::bind(&self.text, post_sels).into_selections();
        self.record_revision(cs, inverse_cs, pre_sels, post_sels);
    }

    /// Apply an ordered Transaction list (from `History::undo_n`/`redo_n`/
    /// `goto_revision`) as one composed transform: fold them together with
    /// `Transaction::compose_all` (sound because each Transaction
    /// in the list maps the state the previous one produced, see
    /// `History::goto_revision`'s doc) and apply the result once. Returns a
    /// [`HistoryWalkResult`]: the restored selections, the net ChangeSet,
    /// and how many steps `txns` held: short of the caller's requested
    /// count when the walk hit the root/leaf, so the caller can tell
    /// exhaustion apart from a full walk. `None` when `txns` is empty
    /// (nothing to do, already at the target).
    ///
    /// The landed text carries the version the landing selections were
    /// recorded for. A walk that undoes an insert and its own later delete
    /// nets to no text change: the text is relabeled at the same generation,
    /// so nothing that orders texts sees a mutation that never happened, and
    /// `install` runs only when the version moved. A different guard from
    /// `apply_edit`/`commit_edit_group`'s, not the same one: those two skip
    /// recording a revision at all, so a no-op edit never enters history;
    /// this walk's revision move already happened in
    /// `History::undo_n`/`redo_n`/`goto_revision` before this is even called.
    fn apply_transactions(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        txns: Vec<Transaction>,
    ) -> HistoryWalkResult {
        let steps = txns.len();
        let txn = Transaction::compose_all(txns)?;
        let landed = txn
            .apply(&self.text)
            .expect("composed history transaction failed: history is corrupt");
        let new_text = landed.text().clone();
        let new_sels = landed.into_selections();
        let cs = txn.into_changes();
        if new_text.version() != self.text.version() {
            self.install(id, stores, new_text, Change::Edit(&cs), Some(acting));
        }
        Some((new_sels, cs, steps))
    }

    /// The current buffer contents.
    pub(crate) fn text(&self) -> &BufferText {
        &self.text
    }

    /// The bytes a save writes: the text with the buffer's own line endings.
    pub(crate) fn serialized(&self) -> String {
        match self.text.line_ending() {
            LineEnding::CrLf => self.text.to_string().replace('\n', "\r\n"),
            LineEnding::Lf => self.text.to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
