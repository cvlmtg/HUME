//! Parses `hume`'s positional file arguments, each optionally suffixed with
//! a `:line[:col]` startup cursor position (`hume foo.rs:12`,
//! `hume foo.rs:12:24`) — the shape most tools emit in `file:line:col`
//! diagnostics, so it can be pasted straight onto the command line.

use std::path::{Path, PathBuf};

/// Error text for a `0` in either position of a `:goto` target or a CLI
/// `path:line[:col]` position — both contracts are 1-based. Lives here
/// (rather than beside `:goto` itself, `editor/commands/typed_misc.rs`)
/// because this module is reachable crate-wide while `editor`'s internals
/// are not; `typed_goto_line` imports it from here so the two error
/// messages can't drift apart.
pub(crate) const LINE_NUMBERS_START_AT_1: &str = "line numbers start at 1";
/// Error text for a `0` column (a `CliPosition::grapheme_col`) in a CLI
/// `path:line:col` position. No `:goto` counterpart to share with — `:goto`
/// only ever takes a line — so this stays private to the CLI parser.
const GRAPHEME_COL_NUMBERS_START_AT_1: &str = "column numbers start at 1";

/// A 1-based startup cursor position, in the units the statusline shows:
/// `line` counts buffer lines, `grapheme_col` counts grapheme clusters
/// within that line (see `hume_editing::lines::place_grapheme_column`) —
/// not chars, so it agrees with what the user read off a `file:line:col`
/// diagnostic or the statusline itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliPosition {
    pub line: usize,
    pub grapheme_col: usize,
}

/// One `hume` command-line file argument, split into the path to open and
/// the optional trailing position to place the cursor at once it's open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileArg {
    pub path: PathBuf,
    pub pos: Option<CliPosition>,
}

/// Parses one positional CLI argument into a [`FileArg`].
///
/// A path that names a real file or symlink *as typed* always wins over
/// splitting — a file genuinely called `weird:12` must stay openable from
/// the CLI. Only when the literal path doesn't exist is a trailing
/// `:<line>` or `:<line>:<col>` peeled off (a lone trailing `:` is
/// tolerated: `foo.rs:12:` behaves like `foo.rs:12`). Both numbers are
/// 1-based; `0` in either position is an error naming both the offending
/// argument and which number was rejected — [`LINE_NUMBERS_START_AT_1`]
/// (matching `:goto`'s own contract) or [`GRAPHEME_COL_NUMBERS_START_AT_1`].
pub fn parse_file_arg(raw: &Path) -> Result<FileArg, String> {
    let literal = || FileArg {
        path: raw.to_path_buf(),
        pos: None,
    };

    // A non-UTF-8 path can't hold a parseable `:<digits>` suffix in any
    // sense this parser understands.
    let Some(s) = raw.to_str() else {
        return Ok(literal());
    };

    // Probes the *expanded* form (`~/weird:12` → `$HOME/weird:12`) so a
    // quoted tilde path is disambiguated the same way it will actually be
    // opened, but returns the untransformed `raw` either way — same
    // "display the typed form" convention `open_extra_file` follows.
    // `symlink_metadata`, not `.exists()`: this is a disambiguation probe,
    // not a pre-open gate, so a broken symlink still counts as "the user
    // meant this path", and a later TOCTOU race just falls through to the
    // other reading rather than lying about a check that already passed.
    let expanded = hume_platform::path::expand(s);
    if std::fs::symlink_metadata(expanded.as_ref()).is_ok() {
        return Ok(literal());
    }

    let trimmed = s.strip_suffix(':').unwrap_or(s);
    let Some((rest, last)) = split_trailing_number(trimmed) else {
        return Ok(literal());
    };
    let (path_str, line, grapheme_col) = match split_trailing_number(rest) {
        Some((rest2, prev)) => (rest2, prev, last),
        None => (rest, last, 1),
    };
    if line == 0 {
        return Err(format!("{}: {LINE_NUMBERS_START_AT_1}", raw.display()));
    }
    if grapheme_col == 0 {
        return Err(format!(
            "{}: {GRAPHEME_COL_NUMBERS_START_AT_1}",
            raw.display()
        ));
    }
    Ok(FileArg {
        path: PathBuf::from(path_str),
        pos: Some(CliPosition { line, grapheme_col }),
    })
}

/// Peels one trailing `:<digits>` group off `s`, returning `(remainder,
/// value)`. `None` when there's no trailing colon, the tail isn't a
/// non-empty run of ASCII digits, the remainder names no file (a bare
/// `:12`, or a directory-only remainder like `/dir/:12` — checked by
/// looking at the last char directly, since `Path::file_name` normalizes
/// away a trailing separator and would read `/dir/` as naming `dir`), or —
/// on Windows — the remainder is a single-letter drive (`C:12`, where the
/// colon is the drive separator, not a position marker; `cfg!` rather than
/// `#[cfg]` so this compiles identically on every platform and only the
/// *check* is conditional).
fn split_trailing_number(s: &str) -> Option<(&str, usize)> {
    let (rest, tail) = s.rsplit_once(':')?;
    if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if rest.is_empty() || rest.ends_with(std::path::is_separator) {
        return None;
    }
    if cfg!(windows) && rest.len() == 1 && rest.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    tail.parse().ok().map(|n| (rest, n))
}

#[cfg(test)]
mod tests;
