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
    if !ctx.is_init {
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
