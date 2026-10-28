use super::*;
use crate::host::{OptionValue, StoredLanguage};
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

/// `set-option!` rejects a value that is no option's type.
#[test]
fn set_option_invalid_value_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let result = set_option(&mut ctx, "indent-width".into(), SteelVal::Void);
    let msg = result.expect_err("void is not a setting value").to_string();
    assert!(
        msg.contains("'indent-width' takes an integer"),
        "got: {msg}"
    );
}

/// A list of strings or symbols is forwarded as its words, so the editor
/// can refuse it for a setting that is not a word list. A symbol in a list
/// is a word, as a symbol on its own is the text.
#[test]
fn a_list_value_is_forwarded_as_words() {
    let forms: [fn(&str) -> SteelVal; 2] = [
        |word| SteelVal::StringV(word.into()),
        |word| SteelVal::SymbolV(word.into()),
    ];
    for form in forms {
        let mut h = SteelCtxTestHarness::new();
        let mut host = RecordingBufferOptionHost::default();
        let target = BufferId::default();
        let mut ctx = h.ctx_with_host(&mut host);
        let list = vec![form("/opt/my sh"), form("-c")]
            .into_steelval()
            .unwrap();
        set_buffer_option(
            &mut ctx,
            hume_core::PaneHandle::buffer_only(target).buffer(),
            "shell".into(),
            list,
        )
        .expect("a list of words must be accepted");
        assert_eq!(
            host.calls,
            vec![(
                "shell".to_string(),
                OptionInput::Words(vec!["/opt/my sh".to_string(), "-c".to_string()]),
                target
            )]
        );
    }
}

#[test]
fn a_list_with_a_non_string_item_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let list = vec![SteelVal::StringV("sh".into()), SteelVal::IntV(1)]
        .into_steelval()
        .unwrap();
    let msg = set_option(&mut ctx, "shell".into(), list)
        .expect_err("a non-string item must be refused")
        .to_string();
    assert!(msg.contains("expected a string"), "got: {msg}");
}

/// In init mode with valid args, `set-option!` reaches the host (NullHost → Err,
/// proving the guard was passed and the host was asked for the option).
#[test]
fn set_option_init_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = set_option(&mut ctx, "indent-width".into(), SteelVal::IntV(4));
    // NullHost.get_global_option returns Err; the error must NOT be the guard error.
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during"),
        "must reach the host, not the guard; got: {msg}"
    );
}

/// `set-option!` and `set-buffer-option!` take an option's value in the
/// type `get-option` reads it back in, and refuse the other one-value types.
#[test]
fn an_option_takes_only_the_type_it_reads_back() {
    let one_values = [
        SteelVal::BoolV(false),
        SteelVal::IntV(3),
        SteelVal::StringV("3".into()),
    ];
    // One option per type `get-option` reads back, a value of that type,
    // and the error a value of every other one-value type raises.
    let typed_options = [
        ("mouse", SteelVal::BoolV(true), "'mouse' takes a boolean"),
        (
            "indent-width",
            SteelVal::IntV(4),
            "'indent-width' takes an integer",
        ),
        (
            "word-chars",
            SteelVal::StringV("-".into()),
            "'word-chars' takes a string",
        ),
    ];
    let mut h = SteelCtxTestHarness::new();
    for (key, own, refusal) in typed_options {
        for value in &one_values {
            let mut host = RecordingBufferOptionHost::default();
            let mut ctx = h.ctx_with_host(&mut host);
            let global = set_option(&mut ctx, key.into(), value.clone());
            let buffer = set_buffer_option(&mut ctx, default_bid(), key.into(), value.clone());
            if std::mem::discriminant(value) == std::mem::discriminant(&own) {
                global.expect("its own type is accepted");
                buffer.expect("its own type is accepted");
            } else {
                for result in [global, buffer] {
                    let msg = result.expect_err("another type is refused").to_string();
                    assert!(msg.contains(refusal), "{key} {value:?}: {msg}");
                }
            }
        }
    }
}

/// A word-list option takes a list only, not the text `:set` would parse.
#[test]
fn a_word_list_option_refuses_text() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let msg = set_option(&mut ctx, "shell".into(), SteelVal::StringV("sh -c".into()))
        .expect_err("text is refused")
        .to_string();
    assert!(
        msg.contains("'shell' takes a list of strings"),
        "got: {msg}"
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

// ── set-buffer-option! ──────────────────────────────────────────────────────

/// `set-buffer-option!` rejects a value that is no option's type.
#[test]
fn set_buffer_option_invalid_value_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let result = set_buffer_option(
        &mut ctx,
        default_bid(),
        "indent-width".into(),
        SteelVal::Void,
    );
    let msg = result.expect_err("void is not a setting value").to_string();
    assert!(
        msg.contains("'indent-width' takes an integer"),
        "got: {msg}"
    );
}

/// `set-buffer-option!`'s `"language"` queues each write as an effect
/// instead of writing through the host, a repeat of the same value
/// included: the editor ranks every write and records the origin of each.
#[test]
fn set_buffer_option_language_queues_every_write_with_its_origin() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
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
    let mut ctx = h.ctx();
    let msg = set_buffer_option(
        &mut ctx,
        default_bid(),
        "language".into(),
        SteelVal::BoolV(false),
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("'language' takes a string"), "got: {msg}");
}

// `set-buffer-option!`'s `bid` is validated live before the builtin body
// (this function) ever runs (`args::LiveBuffer`'s `BuiltinArg::resolve`, in
// the `builtins!`-registered closure), covered once, centrally, through a
// real `ScriptingHost` by `builtins::tests::
// pane_builtins_raise_on_a_closed_buffer_through_real_registration`.
// A direct call to this function (as every other test in this file makes)
// can't reach that check at all, since it bypasses the registration layer.

/// A [`crate::null_host::NullHost`] wrapper that reports every buffer id as
/// existing, knows every option (`shell` as a word list, `mouse` as a
/// boolean, `word-chars` as text, every other one as an integer), and records `set_buffer_option` and `set_buffer_line_ending`
/// calls, so a test can prove the builtin's guards were all passed and the
/// exact `(key, value, bid)` the host received, without a real editor.
#[derive(Default)]
struct RecordingBufferOptionHost {
    inner: crate::null_host::NullHost,
    pub(crate) calls: Vec<(String, OptionInput, BufferId)>,
    pub(crate) line_endings: Vec<(OptionInput, BufferId)>,
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
        form: crate::host::DisplayPathForm,
    ) -> Option<String> {
        self.inner.buffer_display_path(id, form)
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
    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<crate::host::UndoNode>> {
        self.inner.buffer_undo_tree(id)
    }
    fn buffer_text(&self, id: BufferId) -> Option<String> {
        self.inner.buffer_text(id)
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
    fn set_buffer_line_ending(
        &mut self,
        bid: BufferId,
        value: OptionInput,
        origin: ConfigOrigin,
    ) -> Result<(), String> {
        self.line_endings.push((value, bid));
        self.origins.push(origin);
        Ok(())
    }
    fn get_global_option(&self, key: &str) -> Result<OptionValue, String> {
        Ok(match key {
            "shell" => OptionValue::List(Vec::new()),
            "mouse" => OptionValue::Bool(false),
            "word-chars" => OptionValue::Str(String::new()),
            _ => OptionValue::Int(0),
        })
    }
    fn get_buffer_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        crate::host::SettingsHost::get_buffer_option(&self.inner, key, bid)
    }
    fn buffer_line_ending(&self, bid: BufferId) -> Result<String, String> {
        crate::host::SettingsHost::buffer_line_ending(&self.inner, bid)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        crate::host::SettingsHost::steel_command_budget_ms(&self.inner)
    }
}

/// With valid args and a bid the host recognizes, `set-buffer-option!`
/// reaches the host and forwards the decoded `(key, value, bid)`.
#[test]
fn set_buffer_option_reaches_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let target = BufferId::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let result = set_buffer_option(
        &mut ctx,
        hume_core::PaneHandle::buffer_only(target).buffer(),
        "indent-width".into(),
        SteelVal::IntV(8),
    );
    assert!(result.is_ok(), "expected Ok, got {result:?}");
    assert_eq!(
        host.calls,
        vec![(
            "indent-width".to_string(),
            OptionInput::Text("8".to_string()),
            target
        )]
    );
}

/// A named value is a string, the spelling `get-buffer-option` reads back,
/// so a symbol is refused before it reaches the host.
#[test]
fn set_buffer_option_refuses_a_symbol_value() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let target = BufferId::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let err = set_buffer_option(
        &mut ctx,
        hume_core::PaneHandle::buffer_only(target).buffer(),
        "indent-style".into(),
        SteelVal::SymbolV("soft".into()),
    )
    .expect_err("a symbol value is refused");
    assert_eq!(err.kind(), steel::rerrs::ErrorKind::TypeMismatch, "{err}");
    assert!(host.calls.is_empty(), "nothing reaches the host");
}

#[test]
fn a_list_option_value_reads_back_as_a_list_of_strings() {
    assert_eq!(
        SteelVal::from(OptionValue::List(vec!["sh".to_string(), "-c".to_string()])),
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
        SteelVal::from(OptionValue::Str("soft".to_string())),
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
    h.attribution
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

/// A value each buffer-intrinsic option accepts on `set-buffer-option!`.
fn intrinsic_value(opt: BufferIntrinsicOption) -> SteelVal {
    match opt {
        BufferIntrinsicOption::Language => SteelVal::StringV("rust".into()),
        BufferIntrinsicOption::LineEnding => SteelVal::StringV("crlf".into()),
    }
}

#[test]
fn set_option_refuses_a_buffer_intrinsic_key() {
    for opt in BufferIntrinsicOption::ALL {
        let mut h = SteelCtxTestHarness::new();
        let mut host = RecordingBufferOptionHost::default();
        let mut ctx = h.ctx_with_host(&mut host);
        let msg = set_option(&mut ctx, opt.name().into(), intrinsic_value(opt))
            .expect_err("a per-buffer option has no global value to set")
            .to_string();
        drop(ctx);
        assert!(
            msg.contains("per-buffer") && msg.contains("set-buffer-option!"),
            "{}: got {msg}",
            opt.name()
        );
        assert!(h.effects.is_empty(), "{}: nothing is queued", opt.name());
        assert!(
            host.origins.is_empty() && host.calls.is_empty() && host.line_endings.is_empty(),
            "{}: nothing reaches the host",
            opt.name()
        );
    }
}

#[test]
fn get_option_refuses_a_buffer_intrinsic_key() {
    for opt in BufferIntrinsicOption::ALL {
        let mut h = SteelCtxTestHarness::new();
        let mut host = RecordingBufferOptionHost::default();
        let mut ctx = h.ctx_with_host(&mut host);
        let msg = get_option(&mut ctx, opt.name().into())
            .expect_err("a per-buffer option has no global value to read")
            .to_string();
        assert!(
            msg.contains("per-buffer") && msg.contains("get-buffer-option"),
            "{}: got {msg}",
            opt.name()
        );
    }
}

#[test]
fn set_buffer_option_line_ending_reaches_the_line_ending_setter() {
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
        host.line_endings,
        vec![(OptionInput::Text("crlf".to_string()), target)]
    );
    assert!(host.calls.is_empty(), "no settings-table write");
}
