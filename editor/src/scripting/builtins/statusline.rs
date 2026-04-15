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

use steel::rvals::SteelVal;
use steel::rerrs::{ErrorKind, SteelErr};

use crate::ui::statusline::{StatusElement, StatusLineConfig};

type SteelResult = Result<SteelVal, SteelErr>;

// ── Parsing helpers ───────────────────────────────────────────────────────────

/// Map a string name to the corresponding [`StatusElement`] variant.
///
/// Names are the PascalCase variant names from [`StatusElement`] — the same
/// strings a user would write in `init.scm`.
fn parse_element_name(s: &str) -> Result<StatusElement, SteelErr> {
    match s {
        "Mode"           => Ok(StatusElement::Mode),
        "Separator"      => Ok(StatusElement::Separator),
        "FileName"       => Ok(StatusElement::FileName),
        "Position"       => Ok(StatusElement::Position),
        "Selections"     => Ok(StatusElement::Selections),
        "KittyProtocol"  => Ok(StatusElement::KittyProtocol),
        "DirtyIndicator" => Ok(StatusElement::DirtyIndicator),
        "SearchMatches"  => Ok(StatusElement::SearchMatches),
        "MiniBuf"        => Ok(StatusElement::MiniBuf),
        "MacroRecording" => Ok(StatusElement::MacroRecording),
        _ => Err(SteelErr::new(ErrorKind::Generic,
            format!("configure-statusline!: unknown element '{s}'; \
                     valid names: Mode Separator FileName Position Selections \
                     KittyProtocol DirtyIndicator SearchMatches MiniBuf MacroRecording"))),
    }
}

/// Parse a Steel list of strings into a `Vec<StatusElement>`.
///
/// Accepts a `ListV` of strings.  Raises a type error if the value is not a
/// list, and a generic error if any element name is unrecognised.
fn parse_element_list(val: &SteelVal, section: &str) -> Result<Vec<StatusElement>, SteelErr> {
    match val {
        SteelVal::ListV(lst) => {
            lst.iter()
                .map(|v| match v {
                    SteelVal::StringV(s) => parse_element_name(&s.to_string()),
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
pub(crate) fn configure_statusline(args: &[SteelVal]) -> SteelResult {
    if args.len() != 3 {
        steel::stop!(
            ArityMismatch =>
            "configure-statusline! expects 3 args (left center right), got {}",
            args.len()
        );
    }
    let left   = parse_element_list(&args[0], "left")?;
    let center = parse_element_list(&args[1], "center")?;
    let right  = parse_element_list(&args[2], "right")?;

    super::with_ctx("configure-statusline!", |ctx| {
        ctx.settings.statusline = StatusLineConfig { left, center, right };
        Ok(SteelVal::Void)
    })
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
            ("Position",       StatusElement::Position),
            ("KittyProtocol",  StatusElement::KittyProtocol),
            ("DirtyIndicator", StatusElement::DirtyIndicator),
            ("SearchMatches",  StatusElement::SearchMatches),
            ("MiniBuf",        StatusElement::MiniBuf),
            ("MacroRecording", StatusElement::MacroRecording),
        ] {
            let got = parse_element_name(name).unwrap();
            assert_eq!(got, expected, "element '{name}' mismatch");
        }
    }

    #[test]
    fn unknown_element_errors() {
        let err = parse_element_name("FooBar").unwrap_err();
        assert!(err.to_string().contains("FooBar"), "got: {err}");
    }

    #[test]
    fn parse_element_list_rejects_non_list() {
        let val = SteelVal::BoolV(false);
        assert!(parse_element_list(&val, "left").is_err());
    }
}
