//! Theme-file search-path resolution. Zero knowledge of TOML or themes:
//! pure filesystem/name safety.

use std::path::PathBuf;

use rustc_hash::FxHashSet;

use crate::theme::error::ThemeError;

/// A theme name safe to use as one filesystem path segment: non-empty, no
/// `.`/`..`, no path separator, no NUL, and no `:` or `"`.
///
/// The `:` rejection matters on Windows specifically: a name like `c:evil`
/// makes `PathBuf::push` treat it as a drive-relative root, replacing the
/// search directory entirely instead of joining onto it. NUL and the empty
/// string are rejected here rather than left to the filesystem so both come
/// back as "no such theme": an empty name would otherwise probe for a hidden
/// `.toml` in every search dir, and a NUL surfaces from `read_to_string` as
/// `ErrorKind::InvalidInput`, which the search loop doesn't skip on and would
/// report as an I/O failure.
///
/// Accepts exactly the same set as `hume_platform::path::is_safe_segment` and
/// `core:stdlib`'s `stdlib/safe-path-segment?` (`runtime/plugins/core/stdlib/plugin.scm`).
/// Kept as its own copy because `hume-engine` deliberately depends on no
/// platform layer — taking one for a six-line predicate would pull `termina`
/// and `nix` into the renderer and everything downstream of it.
pub(super) fn is_safe_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c != '/' && c != '\\' && c != '"' && c != '\0' && c != ':')
}

/// Finds and reads the first `<name>.toml` in `search_paths` not already in
/// `visited`, inserting its canonical path into `visited` before returning
/// its source alongside that same canonical path (the file's identity for
/// error attribution).
///
/// A candidate whose canonical path is already in `visited` is a cycle
/// through *that specific file* — but not necessarily through `name`: a
/// config-dir theme shadowing a bundled theme of the same name legitimately
/// `inherits`s the bundled (lower-priority, distinct-file) copy, so a match
/// on the first, higher-priority candidate must not end the search. Skip and
/// keep scanning (matching Helix's own `Loader::path`); only report `Cycle`
/// once every candidate has been exhausted this way.
///
/// Reads each candidate first (matching on `NotFound` to skip to the next
/// search dir), then canonicalizes the path for the visited-check. Canonicalize
/// failure after a successful read uses the unresolved path as the cycle key —
/// safe because a deleted-after-read file cannot form a cycle.
///
/// A candidate that exists but can't be read for some other reason (a
/// directory left in its place, a permissions error) is likewise skipped
/// rather than aborting the whole search: search order is a priority list,
/// and one broken higher-priority candidate must not shadow a working
/// lower-priority one — most concretely the bundled copy a config-dir theme
/// of the same name would otherwise make unreachable. The first such error is
/// remembered and only reported if nothing later in the list works either, so
/// a real problem still surfaces instead of silently becoming `NotFound`.
pub(super) fn find_theme_file(
    name: &str,
    search_paths: &[PathBuf],
    visited: &mut FxHashSet<PathBuf>,
) -> Result<(String, PathBuf), ThemeError> {
    if !is_safe_theme_name(name) {
        return Err(ThemeError::NotFound {
            name: name.to_owned(),
        });
    }
    let filename = format!("{name}.toml");
    let mut cycle_found = false;
    let mut io_error = None;
    for dir in search_paths {
        let candidate = dir.join(&filename);
        match std::fs::read_to_string(&candidate) {
            Ok(source) => {
                // Canonicalize after read — residual race only affects cycle-key
                // accuracy, not file content. Not a security prefix check.
                let canonical =
                    std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
                if visited.insert(canonical.clone()) {
                    return Ok((source, canonical));
                }
                cycle_found = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                io_error.get_or_insert(ThemeError::Io {
                    name: name.to_owned(),
                    path: candidate,
                    error: e,
                });
            }
        }
    }
    if cycle_found {
        return Err(ThemeError::Cycle {
            name: name.to_owned(),
        });
    }
    if let Some(e) = io_error {
        return Err(e);
    }
    Err(ThemeError::NotFound {
        name: name.to_owned(),
    })
}
