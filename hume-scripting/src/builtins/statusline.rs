//! Statusline configuration builtin: `configure-statusline!`.
//!
//! The statusline is configured declaratively — the user or a plugin passes
//! three lists of element names (left, center, right) and the builtin forwards
//! them to the editor via [`EditorHost::configure_statusline`].  The editor
//! parses the element names and writes them into the settings; the renderer
//! picks them up the next frame.
//!
//! ## Steel API
//!
//! ```scheme
//! (configure-statusline!
//!   '("Position" "FileName" "DirtyIndicator")  ; left section
//!   '()                                         ; center section (empty)
//!   '("MacroRecording" "SearchMatches" "Separator" "Mode"))  ; right section
//! ```

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::SteelVal;

use crate::SteelCtx;

type SteelResult = Result<SteelVal, SteelErr>;

/// Extract a Steel list of strings into a `Vec<String>`.
///
/// Accepts a `ListV` of strings.  Raises a type error if the value is not a
/// list, and a type mismatch error if any element is not a string.
fn extract_string_list(val: &SteelVal, section: &str) -> Result<Vec<String>, SteelErr> {
    match val {
        SteelVal::ListV(lst) => lst
            .iter()
            .map(|v| match v {
                SteelVal::StringV(s) => Ok(s.to_string()),
                _ => Err(SteelErr::new(
                    ErrorKind::TypeMismatch,
                    format!(
                        "configure-statusline!: {section} section expects a list of \
                                 strings, got {:?}",
                        v
                    ),
                )),
            })
            .collect(),
        _ => Err(SteelErr::new(
            ErrorKind::TypeMismatch,
            format!(
                "configure-statusline!: {section} section must be a list, got {:?}",
                val
            ),
        )),
    }
}

/// `(configure-statusline! left center right)` — configure the three sections
/// of the statusline.
///
/// Each argument is a Steel list of element-name strings.  Pass `'()` for an
/// empty section.  The new config takes effect immediately — the next rendered
/// frame picks it up automatically.
///
/// Valid during `init.scm` or any plugin load.
pub(crate) fn configure_statusline(
    ctx: &mut SteelCtx,
    left: SteelVal,
    center: SteelVal,
    right: SteelVal,
) -> SteelResult {
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
        steel::stop!(Generic =>
            "configure-statusline!: only valid during init.scm or plugin load, not from a Steel command body");
    }
    let left = extract_string_list(&left, "left")?;
    let center = extract_string_list(&center, "center")?;
    let right = extract_string_list(&right, "right")?;
    ctx.host
        .configure_statusline(left, center, right)
        .map_err(|e| SteelErr::new(ErrorKind::Generic, e))?;
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use steel::rvals::IntoSteelVal as _;
    use crate::test_support::SteelCtxTestHarness;

    fn empty_list() -> SteelVal {
        Vec::<SteelVal>::new().into_steelval().unwrap()
    }

    fn string_list(items: &[&str]) -> SteelVal {
        items
            .iter()
            .map(|s| SteelVal::StringV((*s).into()))
            .collect::<Vec<_>>()
            .into_steelval()
            .unwrap()
    }

    // ── Init-only guard ───────────────────────────────────────────────────────

    /// `configure-statusline!` is blocked in plain command mode.
    ///
    /// Fail oracle: remove the guard → statusline layout could be mutated from
    /// a command body at arbitrary times.
    #[test]
    fn configure_statusline_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = configure_statusline(&mut ctx, empty_list(), empty_list(), empty_list());
        assert!(result.is_err(), "configure-statusline! must error in command mode");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("init"), "error must mention 'init'; got: {msg}");
    }

    // ── Type validation of section args ───────────────────────────────────────

    /// `configure-statusline!` rejects a non-list `left` argument.
    ///
    /// Fail oracle: remove the list check → a boolean would be accepted and the
    /// iterator would produce garbage.
    #[test]
    fn configure_statusline_non_list_left_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = configure_statusline(
            &mut ctx,
            SteelVal::BoolV(false), // not a list
            empty_list(),
            empty_list(),
        );
        assert!(result.is_err(), "configure-statusline! must reject non-list left");
    }

    /// `configure-statusline!` rejects a non-list `center` argument.
    #[test]
    fn configure_statusline_non_list_center_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = configure_statusline(
            &mut ctx,
            empty_list(),
            SteelVal::IntV(42),
            empty_list(),
        );
        assert!(result.is_err(), "configure-statusline! must reject non-list center");
    }

    /// `configure-statusline!` rejects a non-list `right` argument.
    #[test]
    fn configure_statusline_non_list_right_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = configure_statusline(
            &mut ctx,
            empty_list(),
            empty_list(),
            SteelVal::SymbolV("bad".into()),
        );
        assert!(result.is_err(), "configure-statusline! must reject non-list right");
    }

    /// `configure-statusline!` rejects a list that contains a non-string element.
    ///
    /// Fail oracle: remove the element type check → integer elements would be
    /// passed as-is to the host and likely panic or produce garbage element names.
    #[test]
    fn configure_statusline_non_string_list_item_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let bad_list: SteelVal = vec![SteelVal::IntV(1)].into_steelval().unwrap();
        let result = configure_statusline(&mut ctx, bad_list, empty_list(), empty_list());
        assert!(result.is_err(), "configure-statusline! must reject non-string list items");
    }

    // ── Guard passes, host called ─────────────────────────────────────────────

    /// In init mode with valid args, `configure-statusline!` reaches the host.
    /// NullHost returns Err, proving the guard was passed.
    #[test]
    fn configure_statusline_init_mode_calls_host() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let left = string_list(&["FileName"]);
        let result = configure_statusline(&mut ctx, left, empty_list(), empty_list());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            !msg.contains("only valid during"),
            "must reach the host, not the guard; got: {msg}"
        );
    }
}
