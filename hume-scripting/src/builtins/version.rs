//! Running-editor version builtins.
//!
//! | Steel name           | Signature                         | Notes                                           |
//! |----------------------|-----------------------------------|-------------------------------------------------|
//! | `hume-version`       | `() → (major minor patch commit)` | `commit`: short sha on a dev build, `#f` on a release |
//! | `hume-version>=?`    | `int int int → bool`              | numeric comparison only; the commit is never ordered |

use steel::rvals::{IntoSteelVal, SteelVal};

use super::SteelResult;
use super::args::usize_arg;
use super::errors::generic_err;

/// `(hume-version)`: `(major minor patch commit)` of the running editor.
pub(crate) fn hume_version(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "hume-version expects 0 args, got {}", args.len());
    }
    let commit = match hume_platform::version::COMMIT {
        Some(sha) => SteelVal::StringV(sha.into()),
        None => SteelVal::BoolV(false),
    };
    vec![
        SteelVal::IntV(hume_platform::version::MAJOR as isize),
        SteelVal::IntV(hume_platform::version::MINOR as isize),
        SteelVal::IntV(hume_platform::version::PATCH as isize),
        commit,
    ]
    .into_steelval()
    .map_err(generic_err)
}

/// `(hume-version>=? major minor patch)`: whether the running editor is at
/// least that version. A dev build counts as the release it is heading toward.
pub(crate) fn hume_version_at_least(args: &[SteelVal]) -> SteelResult {
    let [major, minor, patch] = args else {
        steel::stop!(ArityMismatch => "hume-version>=? expects 3 args, got {}", args.len());
    };
    let component = |val: &SteelVal| -> Result<u32, steel::rerrs::SteelErr> {
        let n = usize_arg(val.clone(), "hume-version>=?")?;
        u32::try_from(n).map_err(|_| generic_err(format!("hume-version>=?: {n} is out of range")))
    };
    Ok(SteelVal::BoolV(hume_platform::version::at_least(
        component(major)?,
        component(minor)?,
        component(patch)?,
    )))
}

#[cfg(test)]
mod tests;
