//! Parses a `path[:line[:col]]` argument, the shape most tools emit in
//! `file:line:col` diagnostics: `hume`'s positional file arguments
//! (`hume foo.rs:12`, `hume foo.rs:12:24`) and `:e`'s own argument
//! (`typed_buffer::typed_edit`) both split on it, so it can be pasted
//! straight onto the command line or after `:e`. Also holds
//! [`ConfigSource`], the crate-wide-reachable type `run`/`run_keys` take for
//! where a session's Steel config comes from.

use std::path::{Path, PathBuf};

use hume_rope::column::GraphemeCol;
use hume_rope::line::ContentLine;

/// Error text for a `0` in either position of a `:goto` target or a
/// `path:line[:col]` position (CLI or `:e`). Both contracts are 1-based.
/// Lives here (rather than beside `:goto` itself,
/// `editor/commands/typed_misc.rs`) because this module is reachable
/// crate-wide while `editor`'s internals are not; `typed_goto_line` imports
/// it from here so the two error messages can't drift apart.
pub(crate) const LINE_NUMBERS_START_AT_1: &str = "line numbers start at 1";
/// Error text for a `0` column (a [`PathPosition::grapheme_col`]) in a
/// `path:line:col` position. No `:goto` counterpart to share with (`:goto`
/// only ever takes a line); `pub(crate)` so `:e`'s own tests can assert on it
/// without a second copy of the string.
pub(crate) const GRAPHEME_COL_NUMBERS_START_AT_1: &str = "column numbers start at 1";

/// A startup or `:e` cursor position, in the units the statusline shows:
/// `line` counts buffer lines, `grapheme_col` counts grapheme clusters
/// within that line (see `hume_editing::lines::place_grapheme_column`), not
/// chars, so it agrees with what the user read off a `file:line:col`
/// diagnostic or the statusline itself. Both are 0-based: decoded from the
/// 1-based digits via `ContentLine::from_number`/`GraphemeCol::from_number`
/// at parse time below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathPosition {
    pub line: ContentLine,
    pub grapheme_col: GraphemeCol,
}

/// One `hume` command-line file argument, split into the path to open and
/// the optional trailing position to place the cursor at once it's open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileArg {
    pub path: PathBuf,
    pub pos: Option<PathPosition>,
}

/// Where a session's Steel config comes from: `--config` and `--no-config`
/// collapsed into one value, since they're mutually exclusive at the CLI
/// layer. Independent of `--keys`: any variant is valid in either headless
/// or interactive mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// `<config_dir>/init.scm`.
    Default,
    /// `--config FILE`: validated and pinned to the startup cwd by `resolve`.
    File(PathBuf),
    /// `--no-config`: bundled runtime Scheme only, no user `init.scm`.
    Skip,
}

/// Parses one positional CLI argument into a [`FileArg`].
///
/// A path that names a real file or symlink *as typed* always wins over
/// splitting: a real file named `weird:12` must stay openable from
/// the CLI. Only when the literal path doesn't exist is a trailing
/// `:<line>` or `:<line>:<col>` peeled off (a lone trailing `:` is
/// tolerated: `foo.rs:12:` behaves like `foo.rs:12`). Both numbers are
/// 1-based; `0` in either position is an error naming both the offending
/// argument and which number was rejected: `LINE_NUMBERS_START_AT_1`
/// (matching `:goto`'s own contract) or `GRAPHEME_COL_NUMBERS_START_AT_1`.
///
/// `cwd` resolves a relative `raw` the same way `Editor::open`'s own startup
/// path handling does (`absolute_unresolved`), so the disambiguation probe
/// agrees with where the file will actually be read from.
pub fn parse_file_arg(raw: &Path, cwd: &Path) -> Result<FileArg, String> {
    // A non-UTF-8 path can't hold a parseable `:<digits>` suffix in any
    // sense this parser understands.
    let Some(s) = raw.to_str() else {
        return Ok(FileArg {
            path: raw.to_path_buf(),
            pos: None,
        });
    };

    let (path_str, pos) = split_path_position(s, |candidate| literal_path_on_disk(candidate, cwd))?;
    Ok(FileArg {
        path: PathBuf::from(path_str),
        pos,
    })
}

/// Does `candidate` name a real file or symlink, exactly as typed (modulo
/// `~` expansion and joining against `cwd`)? The disk half of the "literal
/// path wins over splitting" rule.
///
/// Probes the *expanded* form (`~/weird:12` → `$HOME/weird:12`) so a quoted
/// tilde path is disambiguated the same way it will actually be opened.
/// `symlink_metadata`, not `.exists()`: this is a disambiguation probe, not
/// a pre-open gate, so a broken symlink still counts as "the user meant this
/// path", and a later TOCTOU race just falls through to the other reading
/// rather than lying about a check that already passed.
pub(crate) fn literal_path_on_disk(candidate: &str, cwd: &Path) -> bool {
    let expanded = hume_platform::path::expand(candidate);
    let absolute = hume_platform::path::absolute_unresolved(Path::new(expanded.as_ref()), cwd);
    std::fs::symlink_metadata(absolute).is_ok()
}

/// Splits `s` into a path and an optional trailing `:line[:col]` position.
///
/// `literal_exists` is consulted only when `s` has a trailing `:<digits>`
/// suffix to begin with (see [`split_trailing_number`]); if it returns
/// `true`, `s` is returned unsplit rather than peeled apart. `0` in either
/// position is an error naming `s` and which number was rejected.
pub(crate) fn split_path_position(
    s: &str,
    literal_exists: impl Fn(&str) -> bool,
) -> Result<(&str, Option<PathPosition>), String> {
    let trimmed = s.strip_suffix(':').unwrap_or(s);
    let Some((rest, last)) = split_trailing_number(trimmed) else {
        return Ok((s, None));
    };
    if literal_exists(s) {
        return Ok((s, None));
    }
    let (path_str, line, grapheme_col) = match split_trailing_number(rest) {
        Some((rest2, prev)) => (rest2, prev, last),
        None => (rest, last, 1),
    };
    let Some(line) = ContentLine::from_number(line) else {
        return Err(format!("{s}: {LINE_NUMBERS_START_AT_1}"));
    };
    let Some(grapheme_col) = GraphemeCol::from_number(grapheme_col) else {
        return Err(format!("{s}: {GRAPHEME_COL_NUMBERS_START_AT_1}"));
    };
    Ok((path_str, Some(PathPosition { line, grapheme_col })))
}

/// Peels one trailing `:<digits>` group off `s`, returning `(remainder,
/// value)`. `None` when there's no trailing colon, the tail isn't a
/// non-empty run of ASCII digits, the remainder names no file (a bare
/// `:12`, or a directory-only remainder like `/dir/:12`, checked by
/// looking at the last char directly, since `Path::file_name` normalizes
/// away a trailing separator and would read `/dir/` as naming `dir`), or,
/// on Windows, the remainder is a single-letter drive (`C:12`, where the
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
