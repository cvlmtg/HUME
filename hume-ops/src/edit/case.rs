//! `make-text-lowercase`/`-uppercase`/`-capitalized` — case transforms
//! applied to each selection as a whole string.

use hume_editing::changeset::ChangeSet;
use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use unicode_segmentation::UnicodeSegmentation;

use super::apply_edit;

/// Which case transform [`transform_case`] applies.
enum CaseTransform {
    Lower,
    Upper,
    /// Title Case: uppercase the first letter of each word, lowercase the
    /// rest. See [`capitalize_words`] for what counts as a word.
    Capitalize,
}

/// Transform the case of each selection as a whole string, preserving
/// selection span and direction. Shared implementation for
/// `make-text-lowercase` / `make-text-uppercase` / `make-text-capitalized`.
///
/// Case mapping is applied to the *entire* selection text at once, not
/// grapheme-by-grapheme — Unicode case mapping is context-sensitive (Greek
/// sigma lowercases to `ς` at a word's end, `σ` elsewhere). Mapping one
/// grapheme at a time strips the surrounding context the "is this word-final"
/// check needs, so it silently falls back to the default (non-final) mapping
/// `σ` even at a word's end. `insert` (not `insert_char`) is used since case
/// mapping can also change the char count (e.g. `ß` → `SS`).
fn transform_case(
    buf: Text,
    sels: SelectionSet,
    kind: CaseTransform,
) -> (Text, SelectionSet, ChangeSet) {
    apply_edit(buf, sels, |b, buf, _i, sel, new_sels| {
        let sel_start = sel.start();
        let sel_end = next_grapheme_boundary(buf, sel.end()); // exclusive

        b.retain(sel_start - b.old_pos());
        let new_sel_start = b.new_pos();

        let text: String = buf.slice(sel_start..sel_end).chars().collect();
        let mapped = match kind {
            CaseTransform::Lower => text.to_lowercase(),
            CaseTransform::Upper => text.to_uppercase(),
            CaseTransform::Capitalize => capitalize_words(&text),
        };
        b.delete(sel_end - sel_start);
        b.insert(&mapped);

        let new_sel_end = b.new_pos() - 1;

        let forward = sel.anchor() <= sel.head();
        new_sels.push(Selection::directed(new_sel_start, new_sel_end, forward));
    })
}

/// Capitalize every alphanumeric word run in `text`: uppercase the first
/// grapheme, lowercase the rest — each as one `str` operation, not
/// grapheme-by-grapheme, so context-sensitive mappings stay correct (Greek
/// sigma lowercases to `ς` at a word's end, `σ` elsewhere). Non-word runs
/// (spaces, punctuation, newlines) pass through unchanged and reset the word
/// boundary, so consecutive words each get their own capital.
///
/// A "word" is a maximal run of alphanumeric graphemes — the simplest
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

/// Append `word` to `out` with its first grapheme uppercased and the rest
/// lowercased. No-op if `word` is empty.
fn push_capitalized(out: &mut String, word: &str) {
    let Some(first) = word.graphemes(true).next() else {
        return;
    };
    out.push_str(&first.to_uppercase());
    out.push_str(&word[first.len()..].to_lowercase());
}

/// Lowercase the text in each selection.
pub fn make_text_lowercase(buf: Text, sels: SelectionSet) -> (Text, SelectionSet, ChangeSet) {
    transform_case(buf, sels, CaseTransform::Lower)
}

/// Uppercase the text in each selection.
pub fn make_text_uppercase(buf: Text, sels: SelectionSet) -> (Text, SelectionSet, ChangeSet) {
    transform_case(buf, sels, CaseTransform::Upper)
}

/// Capitalize each word in each selection (Title Case).
pub fn make_text_capitalized(buf: Text, sels: SelectionSet) -> (Text, SelectionSet, ChangeSet) {
    transform_case(buf, sels, CaseTransform::Capitalize)
}
