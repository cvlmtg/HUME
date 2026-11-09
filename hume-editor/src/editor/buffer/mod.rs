use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use super::search::{SearchMatches, SearchPattern};
use crate::editor::clock::Clock;
use crate::editor::edit_session::EditGroup;
use crate::editor::position_stores::PositionStores;
use crate::editor::settings::BufferOverrides;
use hume_core::{BufferId, BufferLanguageTie, Claim, ConfigOrigin, LaterWins, PaneHandle, PaneId};
use hume_editing::change_log::ChangeLog;
use hume_editing::changeset::{ChangeSet, changeset_from_line_diff};
use hume_editing::edit::{Edited, TextChange};
use hume_editing::history::{History, RevisionId};
use hume_editing::hunk::ChangeHunk;
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::{BufferText, LineEnding};
use hume_editing::transaction::Transaction;
use hume_engine::providers::SyntaxSource;
use hume_platform::path::AbsolutePath;
use hume_rope::cluster::ClusterRange;
use md5::{Digest, Md5};

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
pub(in crate::editor) use history::{SavedSelections, Unsaved};
mod buffer_file;
mod file_open;
mod history;
pub(in crate::editor) mod lifecycle;
pub(in crate::editor) mod offsets;
pub(in crate::editor) mod store;
use store::ScratchNumber;
mod undo_file;
use hume_treesitter::layers::SyntaxLayers;
use hume_treesitter::registry::{LanguageId, LanguageRegistry};
use hume_treesitter::syntax::Syntax;

hume_core::named_enum! {
    /// A read-only view the editor itself opens. Its name is reserved:
    /// [`ViewLabel::named`] refuses it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::editor) enum EditorView {
        Messages => "messages",
        PluginStatus => "plugin-status",
        LspStatus => "lsp-status",
    }
}

/// The label of a read-only view buffer, `[name]`. A plugin's view can
/// never take the label of one of the editor's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::editor) struct ViewLabel(ViewOwner);

#[derive(Clone, Debug, PartialEq, Eq)]
enum ViewOwner {
    Editor(EditorView),
    Plugin(String),
}

impl ViewLabel {
    pub(in crate::editor) fn editor(view: EditorView) -> Self {
        Self(ViewOwner::Editor(view))
    }

    /// Errors on an empty `name` or an [`EditorView`]'s name.
    pub(in crate::editor) fn named(name: &str) -> Result<Self, String> {
        if name.is_empty() {
            return Err("the name must not be empty".into());
        }
        if EditorView::from_name(name).is_some() {
            return Err(format!("[{name}] is one of the editor's own views"));
        }
        Ok(Self(ViewOwner::Plugin(name.to_owned())))
    }

    fn name(&self) -> &str {
        match &self.0 {
            ViewOwner::Editor(view) => view.name(),
            ViewOwner::Plugin(name) => name,
        }
    }
}

impl std::fmt::Display for ViewLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]", self.name())
    }
}

// ── Buffer ────────────────────────────────────────────────────────────────────

/// What one composed undo/redo walk hands back. See
/// [`Buffer::apply_transactions`]'s doc for each part's contract. Named
/// once here so `undo_n`/`redo_n`, `doc_ops::apply_doc_history_walk`, and
/// `commands::edit::history_step` all spell the same shape.
pub(in crate::editor) type HistoryWalkResult = Option<(SelectionSet, ChangeSet, usize)>;

/// The selections an applied edit left, tagged with whether its changeset
/// moved any text.
pub(crate) enum Applied {
    Unchanged(SelectionSet),
    Changed(SelectionSet),
}

impl Applied {
    pub(in crate::editor) fn of(sels: SelectionSet, cs: &ChangeSet) -> Self {
        if cs.is_identity() {
            Self::Unchanged(sels)
        } else {
            Self::Changed(sels)
        }
    }

    #[cfg(test)]
    pub(crate) fn into_selections(self) -> SelectionSet {
        match self {
            Self::Unchanged(sels) | Self::Changed(sels) => sels,
        }
    }
}

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
    /// Nothing: a scratch buffer, named by its number.
    Scratch(ScratchNumber),
    File(BufferFile),
    /// A read-only view of text the editor or a plugin built, such as
    /// `[messages]`, named by its label in the statusline and `:ls`.
    View(ViewLabel),
}

/// What a new buffer is made from. [`store::BufferStore::open`] builds the
/// [`Buffer`] from it, and nothing else builds one.
pub(in crate::editor) enum NewBuffer {
    /// A scratch buffer holding this text.
    Scratch(BufferText),
    /// A buffer bound to `file`, offering `pending_dump` (see
    /// [`Buffer::pending_dump`]).
    File {
        text: BufferText,
        file: BufferFile,
        pending_dump: Option<PathBuf>,
    },
    /// A read-only view of text the editor or a plugin built. It has no
    /// language detection and refuses user edits.
    View { text: BufferText, label: ViewLabel },
}

impl NewBuffer {
    /// The file `target` names, read from disk, or an empty buffer bound to
    /// it if it doesn't exist yet (Vim's `:e newfile` semantics: `:w` creates
    /// it). A missing file's resolved path is the target's, the form
    /// `find_by_path` dedup looks it up by.
    ///
    /// The open for which a missing file is one to create;
    /// [`Self::existing_file`] is the open with nothing to create. A path
    /// with no file name (`/`) errors (see [`BufferFile::new_file`]).
    pub(in crate::editor) fn file_or_new(target: FileTarget) -> io::Result<Self> {
        let (text, file) = match BufferFile::read(target.typed().clone()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                (BufferText::empty(), target.into_new_file()?)
            }
            other => other?,
        };
        Ok(Self::file(text, file))
    }

    /// The file `target` names, read from disk. A missing file is a
    /// `NotFound` error, for an open that has nothing to create.
    pub(in crate::editor) fn existing_file(target: FileTarget) -> io::Result<Self> {
        let (text, file) = BufferFile::read(target.typed().clone())?;
        Ok(Self::file(text, file))
    }

    fn file(text: BufferText, file: BufferFile) -> Self {
        let pending_dump = Some(dump::dump_path_for(file.resolved().as_path()))
            .filter(|dump| std::fs::symlink_metadata(dump).is_ok());
        Self::File {
            text,
            file,
            pending_dump,
        }
    }
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
    /// The origin of the `:set buffer language=` or `set-buffer-option!`
    /// `"language"` write that set `language`, empty when detection did. A
    /// later write from a lower origin leaves it, and detection never
    /// replaces it. `:reload-config`'s reset reads this (before clearing it)
    /// to restore the assertion, with its origin, across the reload instead
    /// of letting re-detection pick something else. See `clear_languages_all`.
    pub(in crate::editor) language_by: Claim<BufferLanguageTie>,
    /// The origin of the `line-ending` write that set the text's line
    /// ending, empty when the text it came with decided it. A line-ending
    /// write from one origin replaces the earlier one.
    line_ending_by: Claim<LaterWins>,
    /// The text generation most recently announced as an `on-text-changed`
    /// event. `Buffer` cannot reach the event queue (it holds no
    /// `EditorState`), so the hook is raised by diffing this against the
    /// text's generation at a drain observation point (see
    /// `BufferStore::take_text_changed`) rather than at `install` itself.
    pub(in crate::editor) announced_generation: u64,
    /// The recent text changes, read through [`Self::changes_since`].
    change_log: ChangeLog,
    /// The history state most recently announced as an
    /// `on-undo-history-changed` event, diffed the same way
    /// `announced_generation` is (see `BufferStore::take_history_changed`).
    pub(in crate::editor) announced_history: HistoryMark,
    /// Per-buffer tree-sitter syntax attachment: grammar identity, committed
    /// parse layers, generation bookkeeping, and in-flight state, all in one
    /// place. `None` when no grammar is attached or the buffer exceeds
    /// `syntax-highlight-max-bytes`.
    pub(in crate::editor) syntax: Option<Syntax>,
    /// The [`TextHold`] keeping the text as it is, if one is alive.
    hold: Weak<()>,
    /// Set once `Editor::detect_pending_languages` queues this buffer's
    /// `OnBufferOpen`. Read by `lifecycle::close_buffer_and_notify`: a buffer
    /// closed before then (within the Steel eval that opened it, or by a
    /// plugin during startup) announced no open, so it announces no close
    /// either.
    pub(in crate::editor) open_announced: bool,
    /// The buffer as dispatched events have shown it to plugins.
    pub(in crate::editor) plugin_view: PluginView,
    /// The buffer's disk state as of the last check, set by
    /// `Editor::check_buffer_disk_state`, cleared to `InSync` by a reload or
    /// a successful write. Always `InSync` for scratch/synthetic buffers,
    /// which the check skips.
    pub(in crate::editor) disk_state: disk::DiskState,
    /// The crash dump found next to this buffer's file when it was opened
    /// (`NewBuffer::file_or_new`, not a reload), until the user answers the
    /// restore prompt for it. Kept as a path, not re-derived from `path`: a
    /// save-as changes `path` but not which dump the prompt is about.
    pub(in crate::editor) pending_dump: Option<PathBuf>,
    /// The editor's clock, which stamps this buffer's revisions.
    clock: Clock,
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
    /// What refusing a change to `buffer`'s text reports. The buffer is
    /// named: the change can come from a script, a timer or a language
    /// server, for a buffer not in view.
    pub(in crate::editor) fn message(self, buffer: &Buffer) -> String {
        match self {
            Self::ReadOnly => format!("{} is read-only", buffer.display_name()),
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
    /// The syntax layers parsed from the buffer's current text; `None` when
    /// it has no syntax or the tree describes an older text.
    pub(in crate::editor) fn current_layers(&self) -> Option<&SyntaxLayers> {
        self.syntax
            .as_ref()
            .and_then(|syn| syn.current_layers(self.text().generation()))
    }

    /// The buffer's syntax as the engine reads it, `None` without a grammar.
    pub(in crate::editor) fn syntax_source(&self) -> Option<&dyn SyntaxSource> {
        self.syntax.as_ref().map(|syn| syn as &dyn SyntaxSource)
    }

    /// Names the committed syntax layers, `None` without syntax: it changes
    /// whenever a parse lands, so a view settled before it can be settled
    /// again.
    pub(in crate::editor) fn layers_identity(&self) -> Option<u64> {
        self.syntax.as_ref().map(Syntax::committed_gen)
    }

    /// A buffer holding `text`, for [`store::BufferStore::open`], which
    /// builds `origin` from the [`NewBuffer`] it was handed and hands it the
    /// editor's `clock`. A pane showing it starts with a cursor on its first
    /// cluster.
    fn new(text: BufferText, origin: Origin, pending_dump: Option<PathBuf>, clock: Clock) -> Self {
        let history = History::new(clock.wall_now());
        let saved_revision = Some(history.current_id());
        let saved_line_ending = text.line_ending();
        let announced_generation = text.generation();
        let announced_history = HistoryMark::of(&history, saved_revision);
        Self {
            text,
            history,
            saved_revision,
            saved_line_ending,
            origin,
            search_pattern: None,
            search_matches: SearchMatches::default(),
            #[cfg(test)]
            search_scans: 0,
            overrides: BufferOverrides::default(),
            language: None,
            language_by: Claim::default(),
            line_ending_by: Claim::default(),
            announced_generation,
            change_log: ChangeLog::default(),
            announced_history,
            syntax: None,
            hold: Weak::new(),
            open_announced: false,
            plugin_view: PluginView::default(),
            disk_state: disk::DiskState::InSync,
            pending_dump,
            clock,
        }
    }

    /// `true` for a buffer bound to a file not on disk yet, opened by
    /// [`NewBuffer::file_or_new`] on a missing path and not yet written.
    pub(in crate::editor) fn is_new_file(&self) -> bool {
        self.file().is_some_and(|file| file.file_meta().is_none())
    }

    /// Replace the content of a read-only view buffer with new text.
    ///
    /// Resets history to a clean root and clears search state so the refreshed
    /// buffer is non-dirty and has no stale match data. This is a system
    /// refresh: it bypasses the read-only guard in `doc_ops`.
    pub(in crate::editor) fn set_view_content(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        content: &str,
    ) {
        let text = self.text.replaced_with(content);
        self.history.reset(self.clock.wall_now());
        self.saved_revision = Some(self.history.current_id());
        self.saved_line_ending = text.line_ending();
        self.search_pattern = None;
        self.search_matches = SearchMatches::default();
        self.install(PaneHandle::buffer_only(id), stores, text, Change::Replace);
    }

    /// `true` for a view buffer (`[messages]`, `[plugin-status]`, a plugin's
    /// view), which blocks user edits: `doc_ops` refuses every text change
    /// and Insert mode is refused. It may still be highlighted.
    pub(crate) fn is_read_only(&self) -> bool {
        matches!(self.origin, Origin::View(_))
    }

    /// `true` while a [`TextHold`] on this buffer is alive.
    pub(in crate::editor) fn is_busy(&self) -> bool {
        self.hold.strong_count() > 0
    }

    /// Why the text may not change now, `None` when it may.
    pub(in crate::editor) fn text_refusal(&self) -> Option<TextRefusal> {
        if self.is_read_only() {
            Some(TextRefusal::ReadOnly)
        } else if self.is_busy() {
            Some(TextRefusal::Busy)
        } else {
            None
        }
    }

    /// `Err` with [`Self::text_refusal`]'s message when the text may not
    /// change now.
    pub(in crate::editor) fn check_text_change(&self) -> Result<(), String> {
        self.text_refusal()
            .map_or(Ok(()), |refusal| Err(refusal.message(self)))
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
        target: PaneHandle,
        stores: &mut PositionStores<'_>,
        text: BufferText,
        change: Change<'_>,
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
                if !relabel {
                    self.change_log.record(&change);
                    if let Some(syn) = self.syntax.as_mut() {
                        syn.record_edit(&change);
                    }
                }
                stores.carry(target, &change);
            }
            Change::Replace => {
                self.change_log.record_replace(&before);
                stores.reset(target.buffer(), || {
                    crate::editor::pane_state::fresh_from_buf(self)
                });
            }
        }
    }

    /// The hunks separating the text at `generation` (old side) from the
    /// live text (new side). `None` when the buffer cannot say: a view
    /// refresh replaced the text since then, the change log keeps no step
    /// that far back, or the changed lines are too large to report.
    pub(in crate::editor) fn changes_since(&self, generation: u64) -> Option<Vec<ChangeHunk>> {
        self.change_log.since(generation, &self.text)
    }

    /// Bind the buffer to `file`. A view buffer is never bound to one: a
    /// write of it is an export (see `WriteKind::classify`).
    pub(in crate::editor::buffer) fn set_file(&mut self, file: BufferFile) {
        debug_assert!(!self.is_read_only(), "a view buffer is bound to a file");
        self.origin = Origin::File(file);
    }

    /// The backing file, or `None` for scratch/synthetic buffers.
    pub(in crate::editor) fn file(&self) -> Option<&BufferFile> {
        match &self.origin {
            Origin::File(file) => Some(file),
            Origin::Scratch(_) | Origin::View(_) => None,
        }
    }

    /// The label of a view buffer, or `None` for any other.
    pub(in crate::editor) fn label(&self) -> Option<&ViewLabel> {
        match &self.origin {
            Origin::View(label) => Some(label),
            Origin::Scratch(_) | Origin::File(_) => None,
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

    /// The selections a pane starts with when it first views this buffer: a
    /// cursor on the first cluster.
    pub(in crate::editor) fn initial_sels(&self) -> SelectionSet {
        SelectionSet::at_text_start(&self.text)
    }

    /// [`Self::display_path`], or [`Self::display_name`] for a buffer with no
    /// file.
    pub(crate) fn display_path_or_name(&self) -> Cow<'_, str> {
        self.display_path()
            .map_or_else(|| Cow::Owned(self.display_name()), Cow::Borrowed)
    }

    /// The name shown in the UI: label for view buffers, basename for named
    /// buffers, `*scratch*` or `*scratch-N*` for scratch ones.
    pub(crate) fn display_name(&self) -> String {
        match &self.origin {
            Origin::Scratch(number) => number.to_string(),
            Origin::File(file) => file.file_name().to_string_lossy().into_owned(),
            Origin::View(label) => label.to_string(),
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
    /// The `ChangeSet` is line-diff-derived ([`changeset_from_line_diff`]), so
    /// it and the inverse the history derives from it carry only the changed
    /// lines, not a full-buffer delete-all + insert-all. `install` carries
    /// every stored position through the diff. The history revision records
    /// `focused`'s selections for the buffer before and after, for undo and
    /// redo. No pane made the replacement: every pane keeps the selections it
    /// built since its last edit, and selection undo offers none of the
    /// revision's.
    ///
    /// Returns whether the content changed (`install` ran, the version moved),
    /// `false` when `new_text` has the same content. The line ending is
    /// adopted from `new_text` either way, which then decides it: any origin
    /// may set it again.
    pub(in crate::editor::buffer) fn replace_text_recorded(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        new_text: BufferText,
        focused: PaneId,
    ) -> bool {
        self.line_ending_by = Claim::default();
        let forward = changeset_from_line_diff(&self.text, &new_text);
        if forward.is_identity() {
            self.adopt_line_ending(new_text.line_ending());
            return false;
        }
        // Applying the diff keeps the buffer's lineage, so the replacement is
        // the next generation of the same document rather than a new one.
        let replaced = forward
            .apply(&self.text)
            .expect("a line diff of the current text applies to it");
        debug_assert_eq!(
            replaced, new_text,
            "replace_text_recorded: the line diff must reproduce the new text"
        );
        let recorded =
            |stores: &PositionStores<'_>| stores.panes.seeded(focused, id).selections().clone();
        let pre_sels = recorded(stores);
        let pre_text = self.text.clone();
        let target = PaneHandle::buffer_only(id);
        self.install(
            target,
            stores,
            replaced.with_line_ending(new_text.line_ending()),
            Change::Edit(&forward),
        );
        let post_sels = recorded(stores);
        self.record_revision(target.pane(), forward, &pre_text, pre_sels, post_sels);
        true
    }

    /// Install `text`, `change` applied to a read-only view's text, carrying
    /// every stored position through `change` instead of resetting them as
    /// [`set_view_content`](Self::set_view_content) does. Records no
    /// revision: a view's history stays at its root, so it never reads
    /// modified. An identity change installs nothing, so the version stays.
    pub(in crate::editor) fn follow_view_change(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        text: BufferText,
        change: &ChangeSet,
    ) {
        debug_assert!(
            self.is_read_only(),
            "follow_view_change: an edit to a writable buffer belongs in its history"
        );
        if !change.is_identity() {
            self.install(
                PaneHandle::buffer_only(id),
                stores,
                text,
                Change::Edit(change),
            );
        }
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
    pub(in crate::editor::buffer) fn mark_saved(&mut self) {
        self.saved_revision = Some(self.history.current_id());
        self.saved_line_ending = self.text.line_ending();
        self.disk_state = disk::DiskState::InSync;
    }

    /// Set the line ending a save writes, from `by`. Not an edit: no
    /// revision is recorded, so undo does not revert it. Refused while the
    /// text may not change ([`Self::check_text_change`]), and `Ok(false)`,
    /// changing nothing, when a higher origin set it.
    pub(in crate::editor) fn set_line_ending(
        &mut self,
        line_ending: LineEnding,
        by: ConfigOrigin,
    ) -> Result<bool, String> {
        self.check_text_change()?;
        if !self.line_ending_by.claim(by).is_applied() {
            return Ok(false);
        }
        self.adopt_line_ending(line_ending);
        Ok(true)
    }

    /// Take the line ending of text read from disk, or one
    /// [`Self::set_line_ending`] admitted.
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
        owner: Option<PaneId>,
        forward: ChangeSet,
        pre_text: &BufferText,
        pre_sels: SelectionSet,
        post_sels: SelectionSet,
    ) {
        if let Some(promoted) = self.history.record(
            owner,
            forward,
            pre_text,
            pre_sels,
            post_sels,
            self.clock.wall_now(),
        ) {
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
    /// the post-edit selections; `install` carries every stored position, the
    /// syntax tree and the change log through the edit's `ChangeSet`.
    /// For a `target` with no pane, an edit no pane owns, `sels` is only what
    /// the revision records, and every pane's stored selections are carried.
    pub(crate) fn apply_edit(
        &mut self,
        target: PaneHandle,
        stores: &mut PositionStores<'_>,
        sels: SelectionSet,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> Applied {
        let pre_sels = sels.clone();
        let pre = EditState::bind(self.text.clone(), sels);
        let (post, cs) = self.run_edit(pre, cmd).into_parts();

        // An identity `cs` moved no bytes: recording it would litter the undo
        // tree with a no-op revision, and a new text generation would fire
        // `on-text-changed` for a mutation that never happened.
        if cs.is_identity() {
            return Applied::Unchanged(post.into_selections());
        }

        let pre_text = self.text.clone();
        let (post_text, post_sels) = post.into_parts();
        self.install(target, stores, post_text, Change::Edit(&cs));
        self.record_revision(target.pane(), cs, &pre_text, pre_sels, post_sels.clone());
        Applied::Changed(post_sels)
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
            .run_edit(EditState::bind(self.text.clone(), sels), cmd)
            .into_parts();
        let (new_text, new_sels) = post.into_parts();

        // An identity `cs` moved no bytes, but a new text generation would
        // fire `on-text-changed` for one.
        if cs.is_identity() {
            return (new_sels, cs);
        }
        self.install(
            PaneHandle::with_pane(id, acting),
            stores,
            new_text,
            Change::Edit(&cs),
        );
        (new_sels, cs)
    }

    /// Re-paste from the paste-session snapshot, replacing the accumulated CS.
    ///
    /// Always starts from `snapshot`, the group's, so every cycle cleanly
    /// discards the previous paste output (including added lines).
    /// `to_snapshot` maps the live text back to `snapshot`'s, `None` while
    /// the group has no edits. The edit reaches the live text through a
    /// propagation CS, which `install` carries every stored position through.
    /// Returns the new selections and the CS from `snapshot`'s text, the
    /// group's new accumulated edits.
    pub(in crate::editor) fn apply_edit_regrouped(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        snapshot: EditState,
        to_snapshot: Option<ChangeSet>,
        cmd: impl FnOnce(EditState) -> Edited,
    ) -> (Applied, ChangeSet) {
        let (result, new_cs) = cmd(snapshot).into_parts();

        // With no edits the live text has the snapshot's content, so new_cs
        // is the propagation CS, and the propagation CS is the group's.
        let (propagation_cs, group_cs) = match to_snapshot {
            None => (new_cs, None),
            Some(to_snapshot) => (to_snapshot.compose(new_cs.clone()), Some(new_cs)),
        };

        // An identity `propagation_cs` relabels the live text without
        // changing it, so `install` retags the stores and nothing else sees
        // a change.
        let (post, cs) = Edited::rebase(result, &self.text, propagation_cs)
            .expect("the propagation changeset maps the live text")
            .into_parts();
        let (post_text, post_sels) = post.into_parts();
        if post_text.version() != self.text.version() {
            self.install(
                PaneHandle::with_pane(id, acting),
                stores,
                post_text,
                Change::Edit(&cs),
            );
        }
        let applied = Applied::of(post_sels, &cs);
        (applied, group_cs.unwrap_or(cs))
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
            snapshot: EditState::bind(self.text.clone(), base),
            undo_sels: EditState::bind(self.text.clone(), undo).into_selections(),
            cs: None,
        }
    }

    /// Close the current edit group and record it as a single undo step.
    ///
    /// If no edits were applied since `begin_edit_group` (empty group), or the
    /// composed `ChangeSet` cancelled out to the identity transform (e.g. type
    /// a char, then backspace it), no revision is recorded. Takes `group` by
    /// value: the caller has already closed the session that held it.
    /// `owner` is the pane the session edited through.
    pub(in crate::editor) fn commit_edit_group(
        &mut self,
        group: EditGroup,
        post_sels: SelectionSet,
        owner: PaneId,
    ) {
        // An identity `cs` moved no bytes: recording it would put a no-op
        // revision on the undo stack, and undoing that revision would
        // still consume a step and move `current`. `undo_n`/`redo_n`
        // don't inspect the transaction they're walking, only whether a
        // parent/child link exists, so `u` would consume a step, change
        // no text, and print no exhaustion message either. See
        // `apply_transactions`'s doc for why its own identity guard is a
        // different one, protecting a different case.
        if group.edits().is_none() {
            return;
        }
        let cs = group.cs.expect("the group holds edits");
        let post_sels = EditState::bind(self.text.clone(), post_sels).into_selections();
        self.record_revision(
            Some(owner),
            cs,
            group.snapshot.text(),
            group.undo_sels,
            post_sels,
        );
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
        let (landed, cs) = txn
            .apply(&self.text)
            .expect("composed history transaction failed: history is corrupt");
        let (new_text, new_sels) = landed.into_parts();
        if new_text.version() != self.text.version() {
            self.install(
                PaneHandle::with_pane(id, acting),
                stores,
                new_text,
                Change::Edit(&cs),
            );
        }
        Some((new_sels, cs, steps))
    }

    /// The current buffer contents.
    pub(crate) fn text(&self) -> &BufferText {
        &self.text
    }

    /// The MD5 of the current text, which tells whether a file saved
    /// alongside it (undo history, a session) still describes it. Nothing
    /// relies on it resisting a deliberate collision.
    pub(in crate::editor) fn text_md5(&self) -> [u8; 16] {
        let mut hasher = Md5::new();
        for chunk in self.text.rope().chunks() {
            hasher.update(chunk.as_bytes());
        }
        hasher.finalize().into()
    }

    /// The text beside its syntax tree, which reads it while it changes.
    /// `None` without a language.
    pub(in crate::editor) fn text_and_syntax_mut(&mut self) -> Option<(&BufferText, &mut Syntax)> {
        let syntax = self.syntax.as_mut()?;
        Some((&self.text, syntax))
    }

    /// Store `matches` of `pattern` as found in the current text.
    pub(in crate::editor) fn set_search_matches(
        &mut self,
        pattern: String,
        matches: Vec<ClusterRange>,
    ) {
        self.search_matches.set(pattern, matches, &self.text);
    }

    /// The bytes a save writes: the text with the buffer's own line endings.
    pub(crate) fn serialized(&self) -> String {
        match self.text.line_ending() {
            LineEnding::CrLf => self.text.to_string().replace('\n', "\r\n"),
            LineEnding::Lf => self.text.to_string(),
        }
    }

    /// The registered name of the buffer's language, `None` for none.
    pub(crate) fn language_name<'a>(&self, languages: &'a LanguageRegistry) -> Option<&'a str> {
        self.language.map(|id| languages.name_of(id))
    }

    /// Whether code of `visibility` acts on this buffer, judged by the
    /// language it has now.
    pub(in crate::editor) fn visible_to(
        &self,
        languages: &LanguageRegistry,
        visibility: &hume_scripting::BufferVisibility,
    ) -> bool {
        visibility.sees(self.language_name(languages))
    }
}

/// What the plugins have been told of a buffer: whether its open was
/// dispatched, and the language its last dispatched language set named.
/// Hooks reach a plugin by this language, not the buffer's current one, so
/// a buffer's close reaches the same plugins its open or language set
/// did, even when it closes before a queued language set is dispatched.
#[derive(Debug, Default)]
pub(in crate::editor) struct PluginView {
    pub(in crate::editor) opened: bool,
    pub(in crate::editor) language: Option<Rc<str>>,
}

#[cfg(test)]
pub(crate) mod doc_helper;
#[cfg(test)]
mod tests;
