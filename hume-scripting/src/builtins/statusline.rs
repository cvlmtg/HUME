//! Statusline configuration builtin: `configure-statusline!`.
//!
//! Takes three lists of element names (left, center, right) and forwards them
//! to [`crate::host::SettingsHost::configure_statusline`]; the editor parses
//! the names and the renderer picks them up the next frame.
//!
//! Valid element names: `Cwd`, `Diagnostics`, `DirtyIndicator`, `FilePath`,
//! `FileName`, `KittyProtocol`, `Language`, `LineEnding`, `MacroRecording`,
//! `MiniBuf`, `Mode`, `Position`, `ReadOnly`, `SearchMatches`, `Separator`,
//! plus `steel:<name>` for text pushed via `(set-statusline-text! <name> bid
//! text)`, placed and pushed in either order.

use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::SteelResult;
use super::args::list_to_strings;
use super::errors::generic_err;

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
mod tests;
