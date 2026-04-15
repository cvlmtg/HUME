//! `(set-option! key value)` builtin.
//!
//! Applies a global setting mutation from an `init.scm` or plugin script.
//! Records the prior value in the ledger so the change can be reversed when
//! the plugin is unloaded.

use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::settings::{apply_setting, serialize_setting, BufferOverrides, SettingScope};
use crate::scripting::ledger::Owner;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(set-option! key value)`
///
/// Sets the global setting `key` to `value`. The value may be a Steel string,
/// boolean, or integer — it is converted to a string and forwarded to
/// [`apply_setting`].
///
/// Only `Global` scope is supported from scripts. Use `:set buffer …` from the
/// command line to override a setting for the active buffer.
///
/// Ledger behaviour:
/// - When called from a plugin body (attribution stack is non-empty), records
///   the prior value so it can be restored on plugin unload.
/// - When called from top-level `init.scm` (attribution = `User`), no ledger
///   entry is written — `:reload-config` resets everything from a clean slate.
pub(crate) fn set_option(args: &[SteelVal]) -> SteelResult {
    if args.len() != 2 {
        steel::stop!(ArityMismatch => "set-option! expects 2 args (key value), got {}", args.len());
    }

    let key = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "set-option!: first arg (key) must be a string, got {:?}", args[0]),
    };

    // Accept string, bool, or integer for `value` and convert to the string
    // representation that `apply_setting` expects.
    let value = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        SteelVal::BoolV(b)   => b.to_string(),
        SteelVal::IntV(n)    => n.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "set-option!: second arg (value) must be a string, bool, or integer, got {:?}", args[1]),
    };

    super::with_ctx("set-option!", |ctx| {
        // Capture prior state for the ledger before we overwrite it.
        let prior_value = serialize_setting(&ctx.settings, &key).unwrap_or_default();
        // prior_owner is who owned the setting *before* this mutation —
        // derived from the ledger (last-writer-wins), not the current plugin.
        let prior_owner = ctx.ledger_stack.owner_of(&key);
        let current_owner = ctx.plugin_stack.current_owner();

        let mut dummy_overrides = BufferOverrides::default();
        apply_setting(
            SettingScope::Global,
            &key,
            &value,
            &mut ctx.settings,
            &mut dummy_overrides,
        )
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;

        // Only record ledger entries for plugin-attributed mutations.
        // User-level mutations (top-level init.scm) need no ledger entry
        // because `:reload-config` rebuilds everything from scratch.
        if let Owner::Plugin(ref plugin_id) = current_owner {
            ctx.ledger_stack.record(plugin_id, key, prior_owner, prior_value);
        }

        Ok(SteelVal::Void)
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arity_error_on_wrong_arg_count() {
        let err = set_option(&[SteelVal::StringV("tab-width".into())]).unwrap_err();
        assert!(err.to_string().contains("expects 2 args"), "got: {err}");
    }

    #[test]
    fn type_error_on_non_string_key() {
        let err = set_option(&[SteelVal::IntV(42), SteelVal::IntV(4)]).unwrap_err();
        assert!(err.to_string().contains("key"), "got: {err}");
    }

    #[test]
    fn type_error_on_list_value() {
        use steel::rvals::IntoSteelVal;
        let list = Vec::<SteelVal>::new().into_steelval().unwrap();
        let err = set_option(&[SteelVal::StringV("tab-width".into()), list]).unwrap_err();
        assert!(err.to_string().contains("value"), "got: {err}");
    }
}
