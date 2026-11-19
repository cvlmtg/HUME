//! [`SessionHost`]: the editor as a Steel session's builtins see it. A
//! session may be read-only from its start (a drawer's row renderer, run
//! between `settle` and the frame it renders into) or from a frame up (an
//! `on-paste` handler and whatever it calls): its code may then read the
//! editor but not change it.

use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use hume_core::{
    BufferId, BufferOptionWrite, HostToken, OnUnsaved, OptionInput, OptionValue, PaneHandle,
};
use hume_editing::hunk::ChangeHunk;
use hume_editing::text::BufferText;
use hume_platform::path::AbsolutePath;
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;
use steel::rerrs::SteelErr;

use crate::attribution::{AttributionStack, EntryId, Owner};
use crate::builtins::errors::generic_err;
use crate::command_procs::CommandProc;
use crate::host::{
    AsyncProcessHost, BufferHost, Captures, CommandHolder, CommandHost, CompletionHost, CursorHost,
    DecorationHost, DiffHost, EditHost, EditorHost, EventHost, EventShape, LanguageHost, LspHost,
    RegisterHost, SelectionInfo, SettingsHost, ShownHandle, StoredLanguage, SyntaxHost, TimerHost,
    UiHost, UndoNode,
};
use crate::types::{GrammarReg, SteelCmdDef, SteelTypedCmdDef};

/// Why a session cannot change the editor from its start to its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadOnlySession {
    /// It renders rows or a text's format.
    Rendering,
    /// It reads a bundled language's configuration.
    LanguageConfig,
}

/// Why the editor cannot be changed, and until when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadOnly {
    /// The whole session.
    Session(ReadOnlySession),
    /// An `on-paste` handler's frame: until the attribution frame at
    /// `depth` is left.
    PasteHandler { depth: usize },
}

impl ReadOnly {
    fn during(self) -> &'static str {
        match self {
            Self::Session(ReadOnlySession::Rendering) => "while rendering",
            Self::Session(ReadOnlySession::LanguageConfig) => {
                "while a language's configuration loads"
            }
            Self::PasteHandler { .. } => "while an on-paste handler runs",
        }
    }
}

/// A session's read-only span, and the first write refused in it.
struct Scope {
    read_only: ReadOnly,
    refused: Cell<Option<Refused>>,
}

impl Scope {
    fn new(read_only: ReadOnly) -> Self {
        Self {
            read_only,
            refused: Cell::new(None),
        }
    }
}

/// The host a session's builtins reach through `SteelCtx::host`: the
/// wrapped host itself while no read-only scope is open, and a read-only
/// view of it while one is. Every [`EditorHost`] method takes `&mut self`,
/// so calls go through [`DerefMut`]; shared access is the read-only view.
///
/// It does not implement [`EditorHost`] itself, so a method call never
/// stops at the wrapper and always reaches whichever host is current.
///
/// It also holds the session's attribution frames, which only its own
/// methods change, so a frame that opened a read-only scope cannot be left
/// without ending it.
pub(crate) struct SessionHost<'h> {
    view: ReadOnlyHost<'h>,
    /// The frames of attributed code running: plugin bodies and `call!`ed
    /// commands. See `SteelCtx::owner`.
    attribution: &'h mut AttributionStack,
}

impl<'h> SessionHost<'h> {
    pub(crate) fn new(host: &'h mut dyn EditorHost, attribution: &'h mut AttributionStack) -> Self {
        Self {
            view: ReadOnlyHost {
                inner: RefCell::new(host),
                scope: None,
            },
            attribution,
        }
    }

    /// A session read-only from its start to its end, for `why`.
    pub(crate) fn read_only(
        host: &'h mut dyn EditorHost,
        attribution: &'h mut AttributionStack,
        why: ReadOnlySession,
    ) -> Self {
        let mut session = Self::new(host, attribution);
        session.view.scope = Some(Scope::new(ReadOnly::Session(why)));
        session
    }

    pub(crate) fn attribution(&self) -> &AttributionStack {
        self.attribution
    }

    /// Enters a `call!` frame run as `owner`, returning the depth
    /// [`Self::leave_frames_above`] restores.
    pub(crate) fn enter_call(&mut self, owner: Rc<Owner>) -> usize {
        self.attribution.enter_call(owner)
    }

    /// [`Self::enter_call`] for an `on-paste` handler: the session is
    /// read-only until the frame is left. An enclosing scope already open
    /// stays.
    pub(crate) fn enter_paste_handler_call(&mut self, owner: Rc<Owner>) -> usize {
        let depth = self.attribution.enter_call(owner);
        if self.view.scope.is_none() {
            self.view.scope = Some(Scope::new(ReadOnly::PasteHandler { depth }));
        }
        depth
    }

    pub(crate) fn push_body(&mut self, id: EntryId) {
        self.attribution.push_body(id);
    }

    /// Leaves the innermost frame when it is a plugin body; see
    /// [`AttributionStack::pop_body`]. A body never opens a read-only
    /// scope, so none ends here.
    pub(crate) fn pop_body(&mut self) -> Option<EntryId> {
        self.attribution.pop_body()
    }

    /// Leaves every frame above `depth`, ending the read-only scope one of
    /// them opened, and returns the plugin bodies among them innermost
    /// first.
    pub(crate) fn leave_frames_above(&mut self, depth: usize) -> Vec<EntryId> {
        if let Some(Scope {
            read_only: ReadOnly::PasteHandler { depth: at },
            ..
        }) = self.view.scope
            && at >= depth
        {
            self.view.scope = None;
        }
        self.attribution.drain_above(depth)
    }

    /// Why the session cannot change the editor now, if it cannot.
    fn read_only_reason(&self) -> Option<ReadOnly> {
        self.view.scope.as_ref().map(|scope| scope.read_only)
    }

    /// `Err`, recorded like a refused host write, while the session cannot
    /// change the editor: for a write that does not go through the host.
    pub(crate) fn check_write(&self) -> Result<(), String> {
        match &self.view.scope {
            Some(_) => Err(self.view.refuse()),
            None => Ok(()),
        }
    }

    /// The first write refused in the open scope, as an error message, so
    /// code that caught the error still fails.
    pub(crate) fn take_refusal(&self) -> Option<String> {
        let scope = self.view.scope.as_ref()?;
        scope
            .refused
            .take()
            .map(|what| what.message(scope.read_only))
    }

    /// The optional capability `cap` reaches, or the error for its absence:
    /// it is not available while the session cannot change the editor (one
    /// whose every call is a write records the refusal as it is withheld),
    /// and otherwise not supported by this host.
    pub(crate) fn require<'s, T: ?Sized + 's>(
        &'s mut self,
        cap: fn(&'s mut (dyn EditorHost + 'h)) -> Option<&'s mut T>,
    ) -> Result<&'s mut T, SteelErr> {
        let during = self.read_only_reason().map(ReadOnly::during);
        cap(&mut **self).ok_or_else(|| match during {
            Some(during) => generic_err(format!("not available {during}")),
            None => generic_err("not supported by this host"),
        })
    }
}

impl<'h> Deref for SessionHost<'h> {
    type Target = dyn EditorHost + 'h;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl<'h> DerefMut for SessionHost<'h> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        if self.view.scope.is_some() {
            &mut self.view
        } else {
            &mut **self.view.inner.get_mut()
        }
    }
}

/// Passes every read through to the wrapped host and refuses every write.
/// Offers each optional capability the wrapped host has except three: the
/// timers and processes, whose every call is a write, and the terminal,
/// which only code that may change the editor can claim.
///
/// A refused write returns `Err` where its signature allows. Either way it is
/// recorded, for [`SessionHost::take_refusal`]. Most reads take `&self`
/// while the wrapped host's accessors take `&mut self`, hence the
/// `RefCell`; reads never nest.
struct ReadOnlyHost<'h> {
    inner: RefCell<&'h mut dyn EditorHost>,
    scope: Option<Scope>,
}

/// What a refusal says cannot happen. It names no write: a builtin that
/// raises it puts its own name in front.
#[derive(Clone, Copy)]
enum Refused {
    EditorChange,
    /// Reached through an internal builtin, whose name tells a plugin
    /// author nothing.
    PluginLoad,
}

impl Refused {
    fn message(self, read_only: ReadOnly) -> String {
        let subject = match self {
            Self::EditorChange => "the editor cannot be changed",
            Self::PluginLoad => "a plugin cannot load",
        };
        format!("{subject} {}", read_only.during())
    }
}

impl ReadOnlyHost<'_> {
    fn refuse(&self) -> String {
        self.refuse_as(Refused::EditorChange)
    }

    fn refuse_as(&self, what: Refused) -> String {
        let scope = self
            .scope
            .as_ref()
            .expect("the read-only view is reached only while a scope is open");
        if scope.refused.get().is_none() {
            scope.refused.set(Some(what));
        }
        what.message(scope.read_only)
    }
}

/// The capability an accessor offered, which it offers only while the
/// wrapped host has it.
fn offered<T: ?Sized>(cap: Option<&mut T>) -> &mut T {
    cap.expect("a read-only capability is offered only while the wrapped host has it")
}

impl EditorHost for ReadOnlyHost<'_> {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        self
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        self
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        self
    }
    fn events(&mut self) -> &mut dyn EventHost {
        self
    }
    fn ui(&mut self) -> Option<&mut dyn UiHost> {
        self.inner.get_mut().ui().is_some().then_some(self)
    }
    fn edits(&mut self) -> Option<&mut dyn EditHost> {
        self.inner.get_mut().edits().is_some().then_some(self)
    }
    fn completions(&mut self) -> Option<&mut dyn CompletionHost> {
        self.inner.get_mut().completions().is_some().then_some(self)
    }
    fn decorations(&mut self) -> Option<&mut dyn DecorationHost> {
        self.inner.get_mut().decorations().is_some().then_some(self)
    }
    fn lsp(&mut self) -> Option<&mut dyn LspHost> {
        self.inner.get_mut().lsp().is_some().then_some(self)
    }
    fn registers(&mut self) -> Option<&mut dyn RegisterHost> {
        self.inner.get_mut().registers().is_some().then_some(self)
    }
    fn diff(&mut self) -> Option<&mut dyn DiffHost> {
        self.inner.get_mut().diff().is_some().then_some(self)
    }
    fn syntax(&mut self) -> Option<&mut dyn SyntaxHost> {
        self.inner.get_mut().syntax().is_some().then_some(self)
    }
    fn timers(&mut self) -> Option<&mut dyn TimerHost> {
        self.refuse();
        None
    }
    fn async_process(&mut self) -> Option<&mut dyn AsyncProcessHost> {
        self.refuse();
        None
    }
    // Code that may not change the editor is expected to run after every
    // plugin it depends on has activated.
    fn permit_plugin_activation(&mut self, _plugin: &EntryId) -> Result<(), String> {
        Err(self.refuse_as(Refused::PluginLoad))
    }
}

impl CursorHost for ReadOnlyHost<'_> {
    fn buffer_cursor_line(&self, pane: ShownHandle) -> Result<ContentLine, String> {
        self.inner.borrow_mut().cursor().buffer_cursor_line(pane)
    }
    fn buffer_selections(&self, pane: ShownHandle) -> Result<Vec<SelectionInfo>, String> {
        self.inner.borrow_mut().cursor().buffer_selections(pane)
    }
    fn symbol_under_cursor(&self, pane: ShownHandle) -> Result<String, String> {
        self.inner.borrow_mut().cursor().symbol_under_cursor(pane)
    }
    fn selections_linewise(&self, pane: ShownHandle) -> Result<bool, String> {
        self.inner.borrow_mut().cursor().selections_linewise(pane)
    }
    fn selections_charwise(&self, pane: ShownHandle) -> Result<bool, String> {
        self.inner.borrow_mut().cursor().selections_charwise(pane)
    }
}

impl CommandHost for ReadOnlyHost<'_> {
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.borrow_mut().commands().command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        _name: &str,
        _pane: PaneHandle,
        _call: crate::host::NativeCall,
    ) -> Result<bool, String> {
        Err(self.refuse())
    }
    fn open_dispatch_group(&mut self) {}
    fn close_dispatch_group(&mut self) {}
    fn register_command(&mut self, _def: SteelCmdDef) -> Result<(), String> {
        Err(self.refuse())
    }
    fn register_typed_command(&mut self, _def: SteelTypedCmdDef) -> Result<(), String> {
        Err(self.refuse())
    }
    fn defined_command(&self, name: &str) -> Option<CommandProc> {
        self.inner.borrow_mut().commands().defined_command(name)
    }
    fn unregister_commands_registered_in(&mut self, _activation: &EntryId) {
        self.refuse();
    }
    fn command_holder(&self, name: &str) -> Option<CommandHolder> {
        self.inner.borrow_mut().commands().command_holder(name)
    }
    fn register_lazy_commands(
        &mut self,
        _plugin: &EntryId,
        _commands: &[String],
        _typed_commands: &[String],
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn lazy_command_owner(&self, name: &str) -> Option<EntryId> {
        self.inner.borrow_mut().commands().lazy_command_owner(name)
    }
    fn lazy_mappable_command_owner(&self, name: &str) -> Option<EntryId> {
        self.inner
            .borrow_mut()
            .commands()
            .lazy_mappable_command_owner(name)
    }
    fn unregister_lazy_stubs_of(&mut self, _plugin: &EntryId) {
        self.refuse();
    }
}

impl LanguageHost for ReadOnlyHost<'_> {
    fn attach_grammar(&mut self, _reg: &GrammarReg) -> Result<bool, String> {
        Err(self.refuse())
    }
    fn check_grammar(&self, _name: &str, _source: &hume_core::GrammarSource) -> Result<(), String> {
        Err(self.refuse())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.inner.borrow_mut().language().has_grammar(language)
    }
    fn parsed_text_captures(
        &mut self,
        parsed: HostToken,
        language: &str,
        query: &str,
    ) -> Result<Captures, String> {
        self.inner
            .borrow_mut()
            .language()
            .parsed_text_captures(parsed, language, query)
    }
}

impl SettingsHost for ReadOnlyHost<'_> {
    fn set_global_option(
        &mut self,
        _key: &str,
        _value: OptionInput,
        _origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn set_buffer_option(
        &mut self,
        _key: &str,
        _value: OptionInput,
        _bid: BufferId,
        _origin: hume_core::ConfigOrigin,
    ) -> Result<BufferOptionWrite, String> {
        Err(self.refuse())
    }
    fn set_scope_styles(
        &mut self,
        _styles: Vec<hume_core::ScopeStyleInput>,
        _origin: hume_core::ConfigOrigin,
    ) -> Result<(), String> {
        Err(self.refuse())
    }
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        self.inner.borrow_mut().settings().get_global_option(key)
    }
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        self.inner
            .borrow_mut()
            .settings()
            .get_buffer_option(key, bid)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        self.inner.borrow_mut().settings().steel_command_budget_ms()
    }
}

impl BufferHost for ReadOnlyHost<'_> {
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.inner.borrow_mut().buffers().buffer_ids()
    }
    fn panes(&self) -> Vec<PaneHandle> {
        self.inner.borrow_mut().buffers().panes()
    }
    fn focused_pane(&self) -> PaneHandle {
        self.inner.borrow_mut().buffers().focused_pane()
    }
    fn buffer_panes(&self, id: BufferId) -> Vec<PaneHandle> {
        self.inner.borrow_mut().buffers().buffer_panes(id)
    }
    fn pane_live(&self, pane: PaneHandle) -> bool {
        self.inner.borrow_mut().buffers().pane_live(pane)
    }
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.inner.borrow_mut().buffers().buffer_exists(id)
    }
    fn buffer_path(&self, id: BufferId) -> Option<AbsolutePath> {
        self.inner.borrow_mut().buffers().buffer_path(id)
    }
    fn buffer_display_path(
        &self,
        id: BufferId,
        relative_to: Option<&AbsolutePath>,
    ) -> Option<String> {
        self.inner
            .borrow_mut()
            .buffers()
            .buffer_display_path(id, relative_to)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.inner.borrow_mut().buffers().buffer_display_name(id)
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.inner.borrow_mut().buffers().buffer_is_dirty(id)
    }
    fn buffer_is_read_only(&self, id: BufferId) -> Option<bool> {
        self.inner.borrow_mut().buffers().buffer_is_read_only(id)
    }
    fn buffer_stored_language(&self, id: BufferId) -> StoredLanguage {
        self.inner.borrow_mut().buffers().buffer_stored_language(id)
    }
    fn buffer_visible_to(&self, id: BufferId, visibility: &crate::BufferVisibility) -> bool {
        self.inner
            .borrow_mut()
            .buffers()
            .buffer_visible_to(id, visibility)
    }
    fn cwd(&self) -> AbsolutePath {
        self.inner.borrow_mut().buffers().cwd()
    }
    fn search_path(&self) -> Option<std::ffi::OsString> {
        self.inner.borrow_mut().buffers().search_path()
    }
    fn set_cwd(&mut self, _path: &AbsolutePath) -> Result<AbsolutePath, String> {
        Err(self.refuse())
    }
    fn open_buffer(&mut self, _path: &AbsolutePath) -> Result<BufferId, String> {
        Err(self.refuse())
    }
    fn close_buffer(&mut self, _id: BufferId, _on_unsaved: OnUnsaved) -> Result<(), String> {
        Err(self.refuse())
    }
    fn switch_to_buffer(
        &mut self,
        _pane: ShownHandle,
        _target: BufferId,
    ) -> Result<PaneHandle, String> {
        Err(self.refuse())
    }
    fn show_read_only_buffer(
        &mut self,
        _pane: ShownHandle,
        _name: &str,
        _text: &str,
    ) -> Result<PaneHandle, String> {
        Err(self.refuse())
    }
    fn live_text(&self, id: BufferId) -> Option<BufferText> {
        self.inner.borrow_mut().buffers().live_text(id)
    }
    fn word_search_pattern(&self, id: BufferId, word: &str) -> Option<String> {
        self.inner
            .borrow_mut()
            .buffers()
            .word_search_pattern(id, word)
    }
    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<UndoNode>> {
        self.inner.borrow_mut().buffers().buffer_undo_tree(id)
    }
    fn viewport_range(&mut self, pane: ShownHandle) -> Result<ExclusiveRange<ContentLine>, String> {
        self.inner.get_mut().buffers().viewport_range(pane)
    }
    fn buffer_viewport_ranges(&mut self, id: BufferId) -> crate::host::ViewportLines {
        self.inner.get_mut().buffers().buffer_viewport_ranges(id)
    }
}

impl EventHost for ReadOnlyHost<'_> {
    fn known_event_names(&self) -> &'static [&'static str] {
        self.inner.borrow_mut().events().known_event_names()
    }
    fn event_shape(&self, name: &str) -> Option<EventShape> {
        self.inner.borrow_mut().events().event_shape(name)
    }
    fn finish_hold(&mut self, _hold: HostToken) {
        self.refuse();
    }
}

impl SyntaxHost for ReadOnlyHost<'_> {
    fn syntax_captures(
        &mut self,
        bid: BufferId,
        language: &str,
        query: &str,
        ranges: &[ExclusiveRange<ContentLine>],
    ) -> Result<Captures, String> {
        offered(self.inner.get_mut().syntax()).syntax_captures(bid, language, query, ranges)
    }
}

impl DiffHost for ReadOnlyHost<'_> {
    fn revision_diff(&self, bid: BufferId, revision: usize) -> Result<Vec<ChangeHunk>, String> {
        offered(self.inner.borrow_mut().diff()).revision_diff(bid, revision)
    }
    fn changes_since(
        &self,
        bid: BufferId,
        generation: u64,
    ) -> Result<Option<Vec<ChangeHunk>>, String> {
        offered(self.inner.borrow_mut().diff()).changes_since(bid, generation)
    }
}

mod optional;
#[cfg(test)]
mod tests;
