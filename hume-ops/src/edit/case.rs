//! `make-text-lowercase`/`-uppercase`/`-capitalized`: case transforms
//! applied to each selection as a whole string.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::state::EditState;
use unicode_segmentation::UnicodeSegmentation;
use unicode_titlecase::TitleCase;

use super::apply_edit;

/// Which case transform [`transform_case`] applies.
enum CaseTransform {
    Lower,
    Upper,
    /// Title Case: titlecase the first letter of each word, lowercase the
    /// rest. See [`capitalize_words`] for what counts as a word.
    Capitalize,
}

/// Transform the case of each selection as a whole string, preserving
/// selection span and direction. Shared implementation for
/// `make-text-lowercase` / `make-text-uppercase` / `make-text-capitalized`.
///
/// Case mapping is applied to the *entire* selection text at once, not
/// grapheme-by-grapheme, because Unicode case mapping is context-sensitive (Greek
/// sigma lowercases to `ς` at a word's end, `σ` elsewhere). Mapping one
/// grapheme at a time strips the surrounding context the "is this word-final"
/// check needs, so it silently falls back to the default (non-final) mapping
/// `σ` even at a word's end. Each selection is replaced whole, since case
/// mapping can also change the char count (e.g. `ß` → `SS`).
fn transform_case(state: EditState, kind: CaseTransform) -> Edited {
    apply_edit(state, |b, sel| {
        let selected = sel.slice().to_string();
        let mapped = match kind {
            CaseTransform::Lower => selected.to_lowercase(),
            CaseTransform::Upper => selected.to_uppercase(),
            CaseTransform::Capitalize => capitalize_words(&selected),
        };
        let mark = b.replace(sel.covered(), &mapped);
        Landing::covering(mark, sel.facing())
    })
}

/// Capitalize every alphanumeric word run in `text`: titlecase the first
/// char of the first grapheme, lowercase the rest, each as one `str`
/// operation, not grapheme-by-grapheme, so context-sensitive mappings stay
/// correct (Greek sigma lowercases to `ς` at a word's end, `σ` elsewhere).
/// Non-word runs (spaces, punctuation, newlines) pass through unchanged and
/// reset the word boundary, so consecutive words each get their own capital.
///
/// A "word" is a maximal run of alphanumeric graphemes: the simplest
/// definition that gives sensible results without a full word-motion
/// classifier, though it means an apostrophe counts as a word break
/// (`don't` → `Don'T`).
fn capitalize_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for g in text.graphemes(true) {
        if g.chars().next().is_some_and(char::is_alphanumeric) {
            word.push_str(g);
        } else {
            push_capitalized(&mut out, &word);
            word.clear();
            out.push_str(g);
        }
    }
    push_capitalized(&mut out, &word);
    out
}

/// Append `word` to `out` with its first char titlecased, the rest of its
/// first grapheme kept, and the remainder lowercased. No-op if `word` is
/// empty.
fn push_capitalized(out: &mut String, word: &str) {
    let Some(first) = word.graphemes(true).next() else {
        return;
    };
    let mut chars = first.chars();
    let base = chars.next().expect("a grapheme has at least one char");
    out.extend(TitleCase::to_titlecase(base));
    out.push_str(chars.as_str());
    out.push_str(&word[first.len()..].to_lowercase());
}

/// Lowercase the text in each selection.
pub fn make_text_lowercase(state: EditState) -> Edited {
    transform_case(state, CaseTransform::Lower)
}

/// Uppercase the text in each selection.
pub fn make_text_uppercase(state: EditState) -> Edited {
    transform_case(state, CaseTransform::Upper)
}

/// Capitalize each word in each selection (Title Case).
pub fn make_text_capitalized(state: EditState) -> Edited {
    transform_case(state, CaseTransform::Capitalize)
}
