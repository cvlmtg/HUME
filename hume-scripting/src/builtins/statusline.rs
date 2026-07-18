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
//!   '("Position" "FileName" "FilePath" "DirtyIndicator")  ; left section
//!   '()                                                    ; center section (empty)
//!   '("MacroRecording" "SearchMatches" "Separator" "Mode"))  ; right section
//! ```
//!
//! Valid element names: `Cwd`, `DirtyIndicator`, `FilePath`, `FileName`,
//! `KittyProtocol`, `Language`, `LineEnding`, `MacroRecording`, `MiniBuf`,
//! `Mode`, `Position`, `ReadOnly`, `SearchMatches`, `Selections`, `Separator`.
//!
//! `FilePath` shows the full path to the focused file with the home prefix
//! collapsed to `~`.  When the terminal row is too narrow the path is
//! progressively shortened: leading directory components are abbreviated to
//! their first character, and the filename is truncated with `…` as a last
//! resort.  It renders as empty for scratch and synthetic buffers.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::args::list_to_strings;
use super::errors::generic_err;

type SteelResult = Result<SteelVal, SteelErr>;

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
    let left = list_to_strings(left, "configure-statusline! left")?;
    let center = list_to_strings(center, "configure-statusline! center")?;
    let right = list_to_strings(right, "configure-statusline! right")?;
    ctx.host
        .settings()
        .configure_statusline(left, center, right)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use steel::rvals::IntoSteelVal as _;

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
        let result = super::super::errors::require_config(&h.ctx(), "configure-statusline!");
        assert!(
            result.is_err(),
            "configure-statusline! must error in command mode"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("init"),
            "error must mention 'init'; got: {msg}"
        );
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
        assert!(
            result.is_err(),
            "configure-statusline! must reject non-list left"
        );
    }

    /// `configure-statusline!` rejects a non-list `center` argument.
    #[test]
    fn configure_statusline_non_list_center_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = configure_statusline(&mut ctx, empty_list(), SteelVal::IntV(42), empty_list());
        assert!(
            result.is_err(),
            "configure-statusline! must reject non-list center"
        );
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
        assert!(
            result.is_err(),
            "configure-statusline! must reject non-list right"
        );
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
        assert!(
            result.is_err(),
            "configure-statusline! must reject non-string list items"
        );
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
