//! `(regex pattern)`: a `/` prompt pattern compiled once into a value, and
//! the builtins that search with one: `(string-matches text re)` here and
//! `buffer-search` in `buffers.rs`.
//!
//! A pattern compiles through `hume_ops::search::compile_search_input`,
//! the prompt's own path, so its flags and smart case read as typed at `/`.
//! A script that searches many strings with one pattern pays for the
//! compile once.

use std::sync::Arc;

use hume_ops::search::{compile_search_input, find_all_str_matches};
use regex_cursor::engines::meta::Regex;
use steel::rerrs::SteelErr;
use steel::rvals::{Custom, FromSteelVal, IntoSteelVal as _, SteelVal};

use super::SteelResult;
use super::args::{cons_pair, list_of, string_arg, string_ref_arg};
use super::errors::generic_err;

/// The Steel value `(regex pattern)` returns.
#[derive(Clone)]
struct SteelRegex(Arc<Compiled>);

struct Compiled {
    input: String,
    regex: Regex,
}

impl Custom for SteelRegex {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<regex {:?}>", self.0.input)))
    }
}

/// A `regex` value as a builtin argument. Steel's own decode of a custom
/// type would name the Rust type in its error.
#[derive(Clone)]
pub(crate) struct ArgRegex(Arc<Compiled>);

impl ArgRegex {
    pub(crate) fn regex(&self) -> &Regex {
        &self.0.regex
    }
}

impl FromSteelVal for ArgRegex {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        super::ids::downcast::<SteelRegex>(val)
            .map(|re| ArgRegex(re.0))
            .ok_or_else(|| generic_err(format!("expected a regex, got {val}")))
    }
}

/// `(regex pattern)` -> `pattern` compiled as the `/` prompt reads it.
/// Raises for an empty or invalid pattern. The `m/` flag is accepted and
/// changes nothing, so one pattern serves both this and `search-forward`.
pub(crate) fn regex(pattern: SteelVal) -> SteelResult {
    let input = string_arg(pattern, "pattern")?;
    let (_flags, regex) = compile_search_input(&input)
        .map_err(|_| generic_err(format!("{input:?} is not a valid search pattern")))?;
    Ok(SteelRegex(Arc::new(Compiled { input, regex }))
        .into_steelval()
        .expect("SteelRegex into_steelval"))
}

/// `(string-matches text re)` -> every match of `re` in `text`, in order,
/// as `(start . end)` char offsets into `text`, end-exclusive. As at `/`,
/// none overlaps, none is empty, and each covers whole clusters.
pub(crate) fn string_matches(text: SteelVal, re: ArgRegex) -> SteelResult {
    let text = string_ref_arg(&text, "text")?;
    let pairs = find_all_str_matches(text, re.regex())
        .into_iter()
        .map(|range| {
            let chars = range.chars();
            cons_pair(
                SteelVal::IntV(chars.start.index() as isize),
                SteelVal::IntV(chars.end.index() as isize),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(list_of(pairs))
}

#[cfg(test)]
mod tests;
