//! Statusline configuration builtin: `configure-statusline!`.
//!
//! The statusline is configured declaratively — the user or a plugin passes
//! three lists of element names (left, center, right) and the builtin forwards
//! them to the editor via [`crate::host::SettingsHost::configure_statusline`].
//! The editor parses the element names and writes them into the settings;
//! the renderer picks them up the next frame.
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
mod tests;
