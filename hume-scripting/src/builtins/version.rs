//! Running-editor version builtins.
//!
//! | Steel name           | Signature                                          | Notes                                           |
//! |----------------------|----------------------------------------------------|-------------------------------------------------|
//! | `hume-version`       | `() → (hash 'major 'minor 'patch 'commit)`         | `commit`: short sha on a dev build, `#f` on a release |
//! | `hume-version>=?`    | `int int int → bool`                               | numeric comparison only; the commit is never ordered |

use steel::rvals::SteelVal;

use super::SteelResult;
use super::args::{symbol_hash, usize_arg};
use super::errors::generic_err;

/// `(hume-version)`: `(hash 'major 'minor 'patch 'commit)` of the running
/// editor.
pub(crate) fn hume_version() -> SteelResult {
    let commit = match hume_platform::version::COMMIT {
        Some(sha) => SteelVal::StringV(sha.into()),
        None => SteelVal::BoolV(false),
    };
    Ok(symbol_hash([
        (
            "major",
            SteelVal::IntV(hume_platform::version::MAJOR as isize),
        ),
        (
            "minor",
            SteelVal::IntV(hume_platform::version::MINOR as isize),
        ),
        (
            "patch",
            SteelVal::IntV(hume_platform::version::PATCH as isize),
        ),
        ("commit", commit),
    ]))
}

/// `(hume-version>=? major minor patch)`: whether the running editor is at
/// least that version. A dev build counts as the release it is heading toward.
pub(crate) fn hume_version_at_least(
    major: SteelVal,
    minor: SteelVal,
    patch: SteelVal,
) -> SteelResult {
    let component = |val: SteelVal, label: &str| -> Result<u32, steel::rerrs::SteelErr> {
        let n = usize_arg(val, label)?;
        u32::try_from(n).map_err(|_| generic_err(format!("{n} is out of range")))
    };
    Ok(SteelVal::BoolV(hume_platform::version::at_least(
        component(major, "major")?,
        component(minor, "minor")?,
        component(patch, "patch")?,
    )))
}

#[cfg(test)]
mod tests;
