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

/// Finds and reads the first `<name>.toml` in `search_paths` whose canonical
/// path is not in `visited`, inserts that path into `visited`, and returns the
/// source with it (the file's identity for error attribution).
///
/// A visited candidate is skipped, not an error: a config-dir theme may
/// `inherits` the bundled theme of the same name. `Cycle` is reported only
/// once every candidate is exhausted (as in Helix's `Loader::path`). If
/// canonicalizing fails after a successful read, the unresolved path is the
/// cycle key.
///
/// An unreadable candidate (a directory, a permissions error) is also skipped,
/// so a broken higher-priority file can't shadow a working lower-priority one.
/// The first such error is reported if nothing later works.
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
