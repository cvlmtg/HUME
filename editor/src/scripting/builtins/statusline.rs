//! Statusline configuration builtin: `configure-statusline!`.
//!
//! The statusline is configured declaratively — the user or a plugin passes
//! three lists of element names (left, center, right) and the builtin writes
//! them into `EditorSettings::statusline`.  The existing `HumeStatusline`
//! renderer picks them up the next time a frame is drawn with no extra wiring.
//!
//! ## Steel API
//!
//! ```scheme
//! (configure-statusline!
//!   '("Position" "FileName" "DirtyIndicator")  ; left section
//!   '()                                         ; center section (empty)
//!   '("MacroRecording" "SearchMatches" "Separator" "Mode"))  ; right section
//! ```
//!
//! ## Ledger participation
//!
//! When called from a plugin body, the prior statusline config is captured via
//! `serialize_setting` before the new config is written, and a ledger entry is
//! recorded so the config can be restored on plugin unload.  Top-level
//! `init.scm` mutations (attribution = `User`) write no ledger entry — reload
//! rebuilds from scratch.

use steel::rvals::SteelVal;
use steel::rerrs::{ErrorKind, SteelErr};

use crate::scripting::{ledger::Owner, SteelCtx};
use crate::settings::serialize_setting;
use crate::ui::statusline::{StatusElement, StatusLineConfig};

type SteelResult = Result<SteelVal, SteelErr>;

const SETTING_KEY: &str = "statusline";

// ── Parsing helpers ───────────────────────────────────────────────────────────

/// Parse a Steel list of strings into a `Vec<StatusElement>`.
///
/// Accepts a `ListV` of strings.  Raises a type error if the value is not a
/// list, and a generic error if any element name is unrecognised.
fn parse_element_list(val: &SteelVal, section: &str) -> Result<Vec<StatusElement>, SteelErr> {
    match val {
        SteelVal::ListV(lst) => {
            lst.iter()
                .map(|v| match v {
                    SteelVal::StringV(s) => s.parse::<StatusElement>().map_err(|e| {
                        SteelErr::new(ErrorKind::Generic,
                            format!("configure-statusline!: {e}"))
                    }),
                    _ => Err(SteelErr::new(ErrorKind::TypeMismatch,
                        format!("configure-statusline!: {section} section expects a list of \
                                 strings, got {:?}", v))),
                })
                .collect()
        }
        _ => Err(SteelErr::new(ErrorKind::TypeMismatch,
            format!("configure-statusline!: {section} section must be a list, got {:?}", val))),
    }
}

// ── Builtin ───────────────────────────────────────────────────────────────────

/// `(configure-statusline! left center right)` — configure the three sections
/// of the statusline.
///
/// Each argument is a Steel list of element-name strings.  Pass `'()` for an
/// empty section.  The new config takes effect immediately — the next rendered
/// frame picks it up automatically.
///
/// Only valid during `init.scm` or plugin load.  When called from a plugin body,
/// the prior config is serialized and recorded in the ledger so it can be
/// restored via `apply_setting` when the plugin unloads.
pub(crate) fn configure_statusline(ctx: &mut SteelCtx, left: SteelVal, center: SteelVal, right: SteelVal) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic =>
            "configure-statusline!: only valid during init.scm or plugin load, not from a Steel command body");
    }
    let left   = parse_element_list(&left,   "left")?;
    let center = parse_element_list(&center, "center")?;
    let right  = parse_element_list(&right,  "right")?;
    let new_cfg = StatusLineConfig { left, center, right };

    // Capture prior state for the ledger before overwriting — same pattern as set_option.
    let prior_value = serialize_setting(ctx.settings, SETTING_KEY).unwrap_or_default();
    let prior_owner = ctx.ledger_stack.owner_of(SETTING_KEY);
    let current_owner = ctx.plugin_stack.current_owner();

    ctx.settings.statusline = new_cfg;

    // Only record a ledger entry for plugin-attributed mutations; User-level
    // mutations (top-level init.scm) are rebuilt from scratch on :reload-config.
    if let Owner::Plugin(ref plugin_id) = current_owner {
        ctx.ledger_stack.record(plugin_id, SETTING_KEY.to_string(), prior_owner, prior_value, false);
    }
    Ok(SteelVal::Void)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_elements_parse() {
        for (name, expected) in [
            ("Mode",           StatusElement::Mode),
            ("Separator",      StatusElement::Separator),
            ("FileName",       StatusElement::FileName),
            ("Cwd",            StatusElement::Cwd),
            ("Position",       StatusElement::Position),
            ("KittyProtocol",  StatusElement::KittyProtocol),
            ("DirtyIndicator", StatusElement::DirtyIndicator),
            ("LineEnding",     StatusElement::LineEnding),
            ("SearchMatches",  StatusElement::SearchMatches),
            ("MiniBuf",        StatusElement::MiniBuf),
            ("MacroRecording", StatusElement::MacroRecording),
        ] {
            let got = name.parse::<StatusElement>().unwrap();
            assert_eq!(got, expected, "element '{name}' mismatch");
        }
    }

    #[test]
    fn unknown_element_errors() {
        let err = "FooBar".parse::<StatusElement>().unwrap_err();
        assert!(err.contains("FooBar"), "got: {err}");
    }

    #[test]
    fn parse_element_list_rejects_non_list() {
        let val = SteelVal::BoolV(false);
        assert!(parse_element_list(&val, "left").is_err());
    }
}
