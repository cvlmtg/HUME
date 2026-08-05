use super::*;
use crate::test_support::SteelCtxTestHarness;
use hume_engine::pipeline::BufferId;
use steel::rvals::IntoSteelVal as _;

fn default_bid() -> BidArg {
    BidArg(BufferId::default())
}

/// `set-option!` is registered `open` (`builtins/mod.rs`) — no eval-mode
/// gate at all, since `set_option` (this file) has no gate check of its own
/// and the write it forwards to already goes through the validating
/// chokepoint (`editor::settings_ops::apply_global`) regardless of caller.
/// Reaches the host from ordinary command-mode context, same as
/// `set-buffer-option!`/`get-option`.
///
/// Fail oracle: change `set-option!`'s table entry back to `config` →
/// this call would fail with a gate error instead of reaching (and
/// erroring on) `NullHost`.
#[test]
fn set_option_reaches_host_from_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx(); // EvalMode::Command
    let result = set_option(&mut ctx, "tab-width".into(), SteelVal::IntV(4));
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during") && !msg.contains("command body"),
        "must reach the host, not a gate; got: {msg}"
    );
}

/// `set-option!` rejects value types that are not string, bool, or integer.
///
/// Fail oracle: remove the type check → a list or void would be silently
/// stringified via `{:?}` and applied as a setting value.
#[test]
fn set_option_invalid_value_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    // Pass a list — not a valid value type.
    let list: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_option(&mut ctx, "tab-width".into(), list);
    assert!(
        result.is_err(),
        "set-option! must reject non-string/bool/int value"
    );
}

/// In init mode with valid args, `set-option!` reaches the host (NullHost → Err,
/// proving the guard was passed and the host was called).
///
/// Fail oracle: make the guard unconditionally reject → the host is never called
/// → the error message would contain "init" instead of "NullHost".
#[test]
fn set_option_init_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = set_option(&mut ctx, "tab-width".into(), SteelVal::IntV(4));
    // NullHost.set_global_option returns Err — the error must NOT be the guard error.
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during"),
        "must reach the host, not the guard; got: {msg}"
    );
}

/// `set-option!` accepts all three valid value types without type-error.
#[test]
fn set_option_accepts_string_bool_int_values() {
    // We only need to reach value-string conversion without type error.
    // NullHost will reject the host call, but the type conversion is the
    // interesting path here.
    let mut h = SteelCtxTestHarness::new();

    // All three types must pass the type-check (host error is fine).
    for val in [
        SteelVal::StringV("4".into()),
        SteelVal::BoolV(true),
        SteelVal::IntV(4),
    ] {
        let mut ctx = h.ctx_init();
        let result = set_option(&mut ctx, "tab-width".into(), val);
        // NullHost returns Err, but it must NOT be a TypeMismatch error.
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("must be a string, bool, or integer"),
                "valid value type must not produce a type-mismatch error"
            );
        }
    }
}

/// `get-option` (registered as `%get-option`, wrapped by BOOTSTRAP's
/// `(get-option [bid] key)`) is registered `open` — readable
/// during init eval too, unlike the old `cmd`-gated version, since a stale
/// or default buffer id degrades gracefully to the global default rather
/// than erroring (see `EditorHostImpl::get_option`'s `try_get`-based
/// fallback). `#f` for `bid` selects that fallback explicitly.
///
/// Fail oracle: change `%get-option`'s table entry back to `cmd` → this
/// call would fail with a gate error during init instead of reaching (and
/// erroring on) `NullHost`.
#[test]
fn get_option_reaches_host_during_init_eval() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = get_option(&mut ctx, "tab-width".into(), SteelVal::BoolV(false));
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("not available during init"),
        "must reach the host, not a gate; got: {msg}"
    );
}

/// In command mode, `get-option` reaches the host (`NullHost` → Err,
/// proving the host was called) with an explicit bid decoded from a real
/// `SteelVal`, not just the `#f` default.
#[test]
fn get_option_command_mode_calls_host_with_explicit_bid() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let bid = super::super::ids::SteelBufferId::new(default_bid().0).into_steel_val();
    let result = get_option(&mut ctx, "tab-width".into(), bid);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("not available during init"),
        "must reach the host, not a gate; got: {msg}"
    );
}

// ── set-buffer-option! ──────────────────────────────────────────────────────

/// `set-buffer-option!` is blocked in init mode (`cmd` kind) — gated at
/// registration time, tested via the gate primitive directly.
///
/// Fail oracle: change the table entry from `cmd` to `open` → callable from
/// `init.scm`, where there is no meaningful buffer to target.
#[test]
fn set_buffer_option_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "set-buffer-option!");
    assert!(
        result.is_err(),
        "set-buffer-option! must error in init mode"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init"),
        "error must mention 'init'; got: {msg}"
    );
}

/// `set-buffer-option!` rejects value types that are not string, bool, or
/// integer, before ever consulting the host.
///
/// Fail oracle: remove the type check → a list would be silently
/// stringified via `{:?}` and applied as a setting value.
#[test]
fn set_buffer_option_invalid_value_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let list: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_buffer_option(&mut ctx, default_bid(), "tab-width".into(), list);
    assert!(
        result.is_err(),
        "set-buffer-option! must reject non-string/bool/int value"
    );
}

/// `set-buffer-option!` rejects `"language"` outright — that lives on the
/// buffer's language identity, not its settings — before checking the bid,
/// so the error names `set-buffer-language!` rather than the (also true,
/// but less useful) "invalid buffer id" from `NullHost`.
///
/// Fail oracle: drop the special case → the error becomes "invalid buffer
/// id" here, or "unknown setting 'language'" against a real host.
#[test]
fn set_buffer_option_language_key_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = set_buffer_option(
        &mut ctx,
        default_bid(),
        "language".into(),
        SteelVal::StringV("rust".into()),
    );
    assert!(result.is_err(), "'language' must be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("set-buffer-language!"),
        "error must point at set-buffer-language!; got: {msg}"
    );
}

/// `set-buffer-option!` forwards any `bid`, valid or not, to the host
/// unconditionally — the load-bearing validation is
/// `EditorHostImpl::set_buffer_option`'s own `try_get` guard, which prevents
/// a panic on a stale id (see `host_set_buffer_option_invalid_bid_errors` in
/// `hume-editor/src/editor/tests/settings_effects.rs`). Whatever the host
/// returns for an unrecognized bid is forwarded verbatim.
///
/// Fail oracle: reintroduce a `buffer_exists` check in the builtin body →
/// this call would fail with the builtin's own "invalid buffer id" instead
/// of reaching (and erroring on) `NullHost`'s unconditional
/// "set_buffer_option not available".
#[test]
fn set_buffer_option_forwards_any_bid_to_host_unvalidated() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = set_buffer_option(
        &mut ctx,
        default_bid(),
        "tab-width".into(),
        SteelVal::IntV(2),
    );
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("set_buffer_option not available"),
        "must reach NullHost, not a builtin-level bid check; got: {msg}"
    );
}

/// `set-buffer-option!` accepts all three valid value types without a
/// type-mismatch error (the host may still reject the call for other
/// reasons — `NullHost` always does).
#[test]
fn set_buffer_option_accepts_string_bool_int_values() {
    let mut h = SteelCtxTestHarness::new();
    for val in [
        SteelVal::StringV("2".into()),
        SteelVal::BoolV(true),
        SteelVal::IntV(2),
    ] {
        let mut ctx = h.ctx();
        let result = set_buffer_option(&mut ctx, default_bid(), "tab-width".into(), val);
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("must be a string, bool, or integer"),
                "valid value type must not produce a type-mismatch error"
            );
        }
    }
}

/// A [`crate::null_host::NullHost`] wrapper that reports every buffer id as
/// existing and records `set_buffer_option` calls, so a test can prove the
/// builtin's guards were all passed and the exact `(key, value, bid)` the
/// host received — without a real editor.
#[derive(Default)]
struct RecordingBufferOptionHost {
    inner: crate::null_host::NullHost,
    pub(crate) calls: Vec<(String, String, BufferId)>,
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
}

impl crate::host::BufferHost for RecordingBufferOptionHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        Vec::new()
    }
    fn pane_ids(&self) -> Vec<hume_engine::pipeline::PaneId> {
        Vec::new()
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        true
    }
    fn buffer_path(&self, id: BufferId) -> Option<std::path::PathBuf> {
        self.inner.buffer_path(id)
    }
    fn buffer_display_path(&self, id: BufferId) -> Option<String> {
        self.inner.buffer_display_path(id)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.inner.buffer_display_name(id)
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.inner.buffer_is_dirty(id)
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        self.inner.buffer_stored_language(id)
    }
    fn open_buffer(&mut self, path: &std::path::Path) -> Result<BufferId, String> {
        self.inner.open_buffer(path)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        self.inner.close_buffer(id)
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        self.inner.switch_to_buffer(current, target)
    }
    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        self.inner.buffer_generation(id)
    }
    fn viewport_range(&self, id: BufferId) -> Option<(usize, usize)> {
        self.inner.viewport_range(id)
    }
}

impl crate::host::SettingsHost for RecordingBufferOptionHost {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        crate::host::SettingsHost::set_global_option(&mut self.inner, key, value)
    }
    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String> {
        self.calls.push((key.to_string(), value.to_string(), bid));
        Ok(())
    }
    fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        crate::host::SettingsHost::get_option(&self.inner, key, bid)
    }
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        crate::host::SettingsHost::configure_statusline(&mut self.inner, left, center, right)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        crate::host::SettingsHost::steel_command_budget_ms(&self.inner)
    }
}

/// With valid args and a bid the host recognizes, `set-buffer-option!`
/// reaches the host and forwards exactly the coerced `(key, value, bid)`.
///
/// Fail oracle: any guard rejecting unconditionally, or the builtin
/// forwarding `(current-buffer)` instead of the explicit `bid`, would leave
/// `calls` empty or wrong.
#[test]
fn set_buffer_option_reaches_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = RecordingBufferOptionHost::default();
    let target = BufferId::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let result = set_buffer_option(
        &mut ctx,
        BidArg(target),
        "tab-width".into(),
        SteelVal::IntV(8),
    );
    assert!(result.is_ok(), "expected Ok, got {result:?}");
    assert_eq!(
        host.calls,
        vec![("tab-width".to_string(), "8".to_string(), target)]
    );
}
