use super::*;
use crate::host::StoredLanguage;
use crate::test_support::{SteelCtxTestHarness, default_bid};
use hume_core::{BufferId, ConfigOrigin, OnUnsaved, OptionInput};
use steel::rvals::IntoSteelVal as _;

/// `set-option!` is registered `open` (`builtins/mod.rs`): no eval-mode
/// gate at all, since `set_option` (this file) has no gate check of its own
/// and the write it forwards to already goes through the validating
/// chokepoint (`editor::settings::ops::apply_global`) regardless of caller.
/// Reaches the host from ordinary command-mode context, same as
/// `set-buffer-option!`/`get-option`.
#[test]
fn set_option_reaches_host_from_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx(); // EvalMode::Command
    let result = set_option(&mut ctx, "indent-width".into(), SteelVal::IntV(4));
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during") && !msg.contains("command body"),
        "must reach the host, not a gate; got: {msg}"
    );
}

/// In init mode with valid args, `set-option!` reaches the host (NullHost →
/// Err, proving the guard was passed).
#[test]
fn set_option_init_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = set_option(&mut ctx, "indent-width".into(), SteelVal::IntV(4));
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during"),
        "must reach the host, not the guard; got: {msg}"
    );
}

/// `get-option` is registered `open`, so it is readable during init eval too,
/// mirroring `set-option!`. `get-buffer-option` is `cmd`-gated instead,
/// mirroring `set-buffer-option!` (see that test below).
#[test]
fn get_option_reaches_host_during_init_eval() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = get_option(&mut ctx, "indent-width".into());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("not available during init"),
        "must reach the host, not a gate; got: {msg}"
    );
}

/// In command mode, `get-buffer-option` reaches the host (`NullHost` → Err,
/// proving the host was called) with an explicit bid decoded from a real
/// `SteelVal`.
#[test]
fn get_buffer_option_command_mode_calls_host_with_explicit_bid() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = get_buffer_option(&mut ctx, default_bid(), "indent-width".into());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("not available during init"),
        "must reach the host, not a gate; got: {msg}"
    );
}

/// What `set-buffer-option!` hands the host for `value`.
fn forwarded(value: SteelVal) -> OptionInput {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    set_buffer_option(&mut ctx, default_bid(), "indent-width".into(), value)
        .expect("the recording host accepts every write");
    drop(ctx);
    let [(key, input, bid)] = <[_; 1]>::try_from(host.calls).expect("one write");
    assert_eq!((key.as_str(), bid), ("indent-width", default_bid()));
    input
}

/// A boolean, an integer and a string reach the host as values of their own
/// type, whatever the option takes: the editor checks the type.
#[test]
fn a_one_value_reaches_the_host_in_its_own_type() {
    assert_eq!(
        forwarded(SteelVal::BoolV(true)),
        OptionInput::Value(OptionValue::Bool(true))
    );
    assert_eq!(
        forwarded(SteelVal::IntV(8)),
        OptionInput::Value(OptionValue::Int(8))
    );
    assert_eq!(
        forwarded(SteelVal::StringV("8".into())),
        OptionInput::Value(OptionValue::Str("8".to_string()))
    );
}

/// A list of strings or symbols reaches the host as its words. A symbol in a
/// list is a word, as a symbol on its own is the text.
#[test]
fn a_list_value_is_forwarded_as_words() {
    let forms: [fn(&str) -> SteelVal; 2] = [
        |word| SteelVal::StringV(word.into()),
        |word| SteelVal::SymbolV(word.into()),
    ];
    for form in forms {
        let list = vec![form("/opt/my sh"), form("-c")]
            .into_steelval()
            .unwrap();
        assert_eq!(
            forwarded(list),
            OptionInput::Value(OptionValue::List(vec![
                "/opt/my sh".to_string(),
                "-c".to_string()
            ]))
        );
    }
}

/// A list whose items are not all strings reaches the host as an unreadable
/// list, so the editor can refuse it as a list for an option that takes
/// something else before saying what is wrong inside it.
#[test]
fn a_list_with_a_non_string_item_is_forwarded_as_unreadable() {
    let list = vec![SteelVal::StringV("sh".into()), SteelVal::IntV(1)]
        .into_steelval()
        .unwrap();
    match forwarded(list) {
        OptionInput::Unreadable {
            ty: OptionType::List,
            error,
        } => assert!(error.contains("expected a string"), "got: {error}"),
        other => panic!("expected an unreadable list, got {other:?}"),
    }
}

#[test]
fn a_hash_missing_a_section_is_forwarded_as_unreadable() {
    let hash = symbol_hash([("left", string_list(Vec::new()))]);
    assert!(matches!(
        forwarded(hash),
        OptionInput::Unreadable {
            ty: OptionType::Sections,
            ..
        }
    ));
}

/// A named value is a string, the spelling `get-buffer-option` reads back:
/// a symbol, like void, is no option value, and reaches the host named.
#[test]
fn a_value_of_no_option_type_is_forwarded_named() {
    assert_eq!(
        forwarded(SteelVal::SymbolV("soft".into())),
        OptionInput::Foreign("the symbol 'soft".to_string())
    );
    assert!(matches!(forwarded(SteelVal::Void), OptionInput::Foreign(_)));
}

#[test]
fn set_buffer_option_line_ending_reaches_the_host_as_a_buffer_write() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let target = BufferId::default();
    let mut ctx = h.ctx_with_host(&mut host);
    set_buffer_option(
        &mut ctx,
        hume_core::PaneHandle::buffer_only(target).buffer(),
        "line-ending".into(),
        SteelVal::StringV("crlf".into()),
    )
    .expect("the host accepts the write");
    assert_eq!(
        host.calls,
        vec![(
            "line-ending".to_string(),
            OptionInput::Value(OptionValue::Str("crlf".to_string())),
            target
        )]
    );
}

/// `set-buffer-option!`'s `"language"` queues each write as an effect
/// instead of writing through the host, a repeat of the same value
/// included: the editor ranks every write and records the origin of each.
#[test]
fn set_buffer_option_language_queues_every_write_with_its_origin() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    for _ in 0..2 {
        set_buffer_option(
            &mut ctx,
            default_bid(),
            "language".into(),
            SteelVal::StringV("rust".into()),
        )
        .expect("a string language is accepted");
    }
    drop(ctx);
    let writes: Vec<(Option<&str>, hume_core::ConfigOrigin)> = h
        .state
        .effects
        .iter()
        .filter_map(|q| match &q.effect {
            crate::types::Effect::SetBufferLanguage {
                language, origin, ..
            } => Some((language.as_deref(), *origin)),
            _ => None,
        })
        .collect();
    assert_eq!(
        writes,
        vec![
            (Some("rust"), hume_core::ConfigOrigin::User),
            (Some("rust"), hume_core::ConfigOrigin::User)
        ]
    );
}

#[test]
fn set_buffer_option_language_rejects_a_non_string() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let msg = set_buffer_option(
        &mut ctx,
        default_bid(),
        "language".into(),
        SteelVal::BoolV(false),
    )
    .unwrap_err()
    .to_string();
    assert!(
        msg.contains("invalid value for 'language': expected a string"),
        "got: {msg}"
    );
}

// `set-buffer-option!`'s `bid` is validated live before the builtin body
// (this function) ever runs (`args::LiveBuffer`'s `BuiltinArg::resolve`, in
// the `builtins!`-registered closure), covered once, centrally, through a
// real `ScriptingHost` by `builtins::tests::
// pane_builtins_raise_on_a_closed_buffer_through_real_registration`.
// A direct call to this function (as every other test in this file makes)
// can't reach that check at all, since it bypasses the registration layer.

/// A [`crate::null_host::NullHost`] wrapper that reports every buffer id as
/// existing, accepts every option write, and records each
/// `set_buffer_option` call, so a test can prove the builtin's guards were
/// all passed and the exact `(key, value, bid)` the host received, without a
/// real editor.
#[derive(Default)]
struct RecordingBufferOptionHost {
    inner: crate::null_host::NullHost,
    pub(crate) calls: Vec<(String, OptionInput, BufferId)>,
    /// The origin each global or buffer write arrived with, in order.
    pub(crate) origins: Vec<ConfigOrigin>,
}

impl crate::host::EditorHost for RecordingBufferOptionHost {
    fn cursor(&mut self) -> &mut dyn crate::host::CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn crate::host::CommandHost {
        &mut self.inner
    }
    fn language(&mut self) -> &mut dyn crate::host::LanguageHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn crate::host::SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn crate::host::BufferHost {
        self
    }
    fn events(&mut self) -> &mut dyn crate::host::EventHost {
        &mut self.inner
    }
    fn permit_plugin_activation(
        &mut self,
        plugin: &crate::attribution::EntryId,
    ) -> Result<(), String> {
        self.inner.permit_plugin_activation(plugin)
    }
}

impl crate::host::BufferHost for RecordingBufferOptionHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        Vec::new()
    }
    fn panes(&self) -> Vec<hume_core::PaneHandle> {
        Vec::new()
    }
    fn focused_pane(&self) -> hume_core::PaneHandle {
        self.inner.focused_pane()
    }
    fn buffer_panes(&self, id: BufferId) -> Vec<hume_core::PaneHandle> {
        self.inner.buffer_panes(id)
    }
    fn pane_live(&self, pane: hume_core::PaneHandle) -> bool {
        self.inner.pane_live(pane)
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        true
    }
    fn buffer_path(&self, id: BufferId) -> Option<hume_platform::path::AbsolutePath> {
        self.inner.buffer_path(id)
    }
    fn buffer_display_path(
        &self,
        id: BufferId,
        relative_to: Option<&hume_platform::path::AbsolutePath>,
    ) -> Option<String> {
        self.inner.buffer_display_path(id, relative_to)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.inner.buffer_display_name(id)
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.inner.buffer_is_dirty(id)
    }
    fn buffer_stored_language(&self, id: BufferId) -> StoredLanguage {
        self.inner.buffer_stored_language(id)
    }
    fn buffer_visible_to(&self, id: BufferId, visibility: &crate::BufferVisibility) -> bool {
        self.inner.buffer_visible_to(id, visibility)
    }
    fn cwd(&self) -> hume_platform::path::AbsolutePath {
        self.inner.cwd()
    }
    fn search_path(&self) -> Option<std::ffi::OsString> {
        self.inner.search_path()
    }
    fn set_cwd(
        &mut self,
        path: &hume_platform::path::AbsolutePath,
    ) -> Result<hume_platform::path::AbsolutePath, String> {
        self.inner.set_cwd(path)
    }
    fn open_buffer(
        &mut self,
        path: &hume_platform::path::AbsolutePath,
    ) -> Result<BufferId, String> {
        self.inner.open_buffer(path)
    }
    fn close_buffer(&mut self, id: BufferId, on_unsaved: OnUnsaved) -> Result<(), String> {
        self.inner.close_buffer(id, on_unsaved)
    }
    fn switch_to_buffer(
        &mut self,
        pane: crate::host::ShownHandle,
        target: BufferId,
    ) -> Result<hume_core::PaneHandle, String> {
        self.inner.switch_to_buffer(pane, target)
    }
    fn show_read_only_buffer(
        &mut self,
        pane: crate::host::ShownHandle,
        name: &str,
        text: &str,
    ) -> Result<hume_core::PaneHandle, String> {
        self.inner.show_read_only_buffer(pane, name, text)
    }
    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        self.inner.buffer_generation(id)
    }
    fn buffer_search(
        &self,
        id: BufferId,
        pattern: &str,
    ) -> Option<Result<Option<std::ops::Range<usize>>, String>> {
        self.inner.buffer_search(id, pattern)
    }
    fn word_search_pattern(&self, id: BufferId, word: &str) -> Option<String> {
        self.inner.word_search_pattern(id, word)
    }
    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<crate::host::UndoNode>> {
        self.inner.buffer_undo_tree(id)
    }
    fn buffer_text(
        &self,
        id: BufferId,
        range: Option<(usize, usize)>,
    ) -> Option<Result<String, String>> {
        self.inner.buffer_text(id, range)
    }
    fn buffer_line_count(&self, id: BufferId) -> Option<usize> {
        self.inner.buffer_line_count(id)
    }
    fn buffer_char_count(&self, id: BufferId) -> Option<usize> {
        self.inner.buffer_char_count(id)
    }
    fn buffer_lines(
        &self,
        id: BufferId,
        range: hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
    ) -> Option<Vec<String>> {
        self.inner.buffer_lines(id, range)
    }
    fn line_to_char_offset(
        &self,
        id: BufferId,
        line: hume_rope::line::ContentLine,
    ) -> Option<usize> {
        self.inner.line_to_char_offset(id, line)
    }
    fn viewport_range(
        &mut self,
        pane: crate::host::ShownHandle,
    ) -> Result<hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>, String> {
        self.inner.viewport_range(pane)
    }
}

impl crate::host::SettingsHost for RecordingBufferOptionHost {
    fn set_global_option(
        &mut self,
        _key: &str,
        _value: OptionInput,
        origin: ConfigOrigin,
    ) -> Result<(), String> {
        self.origins.push(origin);
        Ok(())
    }
    fn set_buffer_option(
        &mut self,
        key: &str,
        value: OptionInput,
        bid: BufferId,
        origin: ConfigOrigin,
    ) -> Result<(), String> {
        self.calls.push((key.to_string(), value, bid));
        self.origins.push(origin);
        Ok(())
    }
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        crate::host::SettingsHost::get_global_option(&self.inner, key)
    }
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        crate::host::SettingsHost::get_buffer_option(&self.inner, key, bid)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        crate::host::SettingsHost::steel_command_budget_ms(&self.inner)
    }
}

#[test]
fn a_list_option_value_reads_back_as_a_list_of_strings() {
    assert_eq!(
        option_value_to_steel(OptionValue::List(vec!["sh".to_string(), "-c".to_string()])),
        vec![
            SteelVal::StringV("sh".into()),
            SteelVal::StringV("-c".into())
        ]
        .into_steelval()
        .unwrap()
    );
}

#[test]
fn a_named_option_value_reads_back_as_a_string() {
    assert_eq!(
        option_value_to_steel(OptionValue::Str("soft".to_string())),
        SteelVal::StringV("soft".into())
    );
}

/// An option write reaches the host as the calling code's origin: a command
/// the user runs is the user's, a plugin body is the plugin's.
#[test]
fn option_writes_carry_the_callers_origin() {
    use crate::attribution::{EntryId, PluginId};

    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let pane = hume_core::PaneHandle::buffer_only(BufferId::default());
    let write = |ctx: &mut SteelCtx| {
        set_option(ctx, "indent-width".into(), SteelVal::IntV(2)).expect("global write");
        set_buffer_option(ctx, pane.buffer(), "indent-width".into(), SteelVal::IntV(2))
            .expect("buffer write");
    };
    write(&mut h.ctx_with_host(&mut host));
    h.state
        .attribution
        .push_body(EntryId::main(PluginId::parse("core:test-plugin").unwrap()));
    write(&mut h.ctx_with_host(&mut host));

    assert_eq!(
        host.origins,
        [
            ConfigOrigin::User,
            ConfigOrigin::User,
            ConfigOrigin::Plugin,
            ConfigOrigin::Plugin
        ]
    );
}
