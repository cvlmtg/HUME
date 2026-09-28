//! `(split-words line word-chars)`: native word tokenization for Steel
//! plugins.
//!
//! Reuses `hume-ops`'s own word-motion scan primitives (`word_runs`, built
//! on the same grapheme-cluster/combining-mark-safe stepping and
//! `WordChars::classify` that `w`/`b` motions and text objects already use)
//! rather than approximating classification in Steel, since Steel has no
//! Unicode character-category table, so a hand-rolled Steel classifier
//! either merges non-ASCII punctuation into words or has to special-case
//! every script by hand. A word this returns is what a `w` motion would
//! select. `word_chars` is threaded straight through, so tokenization is
//! settings-aware per call: `core:stdlib`'s `stdlib/split-words` passes the
//! calling buffer's own `word-chars` (`(get-buffer-option pane
//! "word-chars")`), which is how `core:buffer-words` tokenizes, so `foo-bar`
//! splits into two words by default but stays one wherever `word-chars`
//! includes `-` (a CSS buffer, say).

use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_ops::motion::word_runs;
use steel::rvals::SteelVal;

use super::SteelResult;
use super::args::{string_arg, string_list};
use super::errors::generic_err;

/// `(split-words line word-chars)` -> list of word strings, in order.
///
/// Validates `word_chars` first. A caller may pass any string, not only an
/// already-validated `get-buffer-option` value, so this input is untrusted
/// Steel-side data, the same as `buffer-lines`' range args. A value
/// `WordChars::classify` couldn't handle correctly (e.g. a whitespace
/// character other than the four `classify_char` recognizes) raises here
/// instead of misclassifying.
pub(crate) fn split_words(line: SteelVal, word_chars: SteelVal) -> SteelResult {
    let line = string_arg(line, "split-words line")?;
    let word_chars = string_arg(word_chars, "split-words word-chars")?;
    WordChars::validate(&word_chars)
        .map_err(|e| generic_err(format!("split-words word-chars: {e}")))?;
    let text = BufferText::from(line.as_str());
    let chars = WordChars::new(&word_chars);
    let words = word_runs(&text, chars)
        .into_iter()
        .map(|range| text.slice(range.to_exclusive()).to_string());
    Ok(string_list(words))
}

#[cfg(test)]
mod tests;
