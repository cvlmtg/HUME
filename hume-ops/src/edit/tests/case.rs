use super::super::*;
use hume_test_fixtures::assert_state;

// ── make_text_lowercase / make_text_uppercase / make_text_capitalized ──────

#[test]
fn lowercase_uppercase_selection() {
    assert_state!(
        "-[HELLO]> world\n",
        |(text, sels)| make_text_lowercase(text, sels),
        "-[hello]> world\n"
    );
}

#[test]
fn lowercase_mixed_case_selection() {
    assert_state!(
        "-[HeLLo]>\n",
        |(text, sels)| make_text_lowercase(text, sels),
        "-[hello]>\n"
    );
}

#[test]
fn uppercase_lowercase_selection() {
    assert_state!(
        "-[hello]> world\n",
        |(text, sels)| make_text_uppercase(text, sels),
        "-[HELLO]> world\n"
    );
}

#[test]
fn uppercase_preserves_backward_selection_direction() {
    // Backward selection anchor=5, head=0; direction preserved after transform.
    assert_state!(
        "<[hello]-\n",
        |(text, sels)| make_text_uppercase(text, sels),
        "<[HELLO]-\n"
    );
}

#[test]
fn uppercase_multiline_selection_skips_newline() {
    // The '\n' between lines is retained; each line's content is uppercased
    // independently, so line structure is unaffected.
    assert_state!(
        "-[hello\nworld]>\n",
        |(text, sels)| make_text_uppercase(text, sels),
        "-[HELLO\nWORLD]>\n"
    );
}

#[test]
fn capitalize_multi_word_selection() {
    // Each word's first letter is uppercased, the rest lowercased (Title Case).
    assert_state!(
        "-[hELLO wORLD]>\n",
        |(text, sels)| make_text_capitalized(text, sels),
        "-[Hello World]>\n"
    );
}

#[test]
fn capitalize_single_char_cursor() {
    // A one-char selection is a one-word selection: it is its own word start,
    // so it is uppercased with no special-casing needed.
    assert_state!(
        "-[h]>i\n",
        |(text, sels)| make_text_capitalized(text, sels),
        "-[H]>i\n"
    );
}

#[test]
fn capitalize_non_word_chars_break_words() {
    // '-' is not alphanumeric, so it breaks the word run: each side of it
    // gets its own capital letter.
    assert_state!(
        "-[abc-def]>\n",
        |(text, sels)| make_text_capitalized(text, sels),
        "-[Abc-Def]>\n"
    );
}

#[test]
fn capitalize_multiline_selection_resets_word_state_at_newline() {
    // Word state resets across the skipped '\n', so the first word of the
    // second line is capitalized independently of the first line's ending.
    assert_state!(
        "-[hello\nworld]>\n",
        |(text, sels)| make_text_capitalized(text, sels),
        "-[Hello\nWorld]>\n"
    );
}

#[test]
fn uppercase_grows_selection_when_case_mapping_changes_char_count() {
    // ß has no single-char uppercase form: it maps to "SS" (two chars).
    // transform_case must re-insert (not substitute in place), and the
    // resulting selection must grow to cover both.
    assert_state!(
        "-[ß]>\n",
        |(text, sels)| make_text_uppercase(text, sels),
        "-[SS]>\n"
    );
}

// Case mapping is context-sensitive: Greek sigma (Σ/σ) lowercases to the
// final form 'ς' only at a word's end, and to 'σ' everywhere else. Mapping
// grapheme-by-grapheme strips the surrounding context that check needs, so
// it silently falls back to the default 'σ' even when the grapheme is at a
// word's end — correct mid-word only by accident, wrong at word-final
// position.

#[test]
fn lowercase_resolves_mid_word_sigma_by_context() {
    assert_state!(
        "-[ΟΣΟ]>\n",
        |(text, sels)| make_text_lowercase(text, sels),
        "-[οσο]>\n"
    );
}

#[test]
fn lowercase_resolves_word_final_sigma_by_context() {
    // A per-grapheme loop yields "οοσ" here (default mapping, no
    // final-sigma context) instead of "οος".
    assert_state!(
        "-[ΟΟΣ]>\n",
        |(text, sels)| make_text_lowercase(text, sels),
        "-[οος]>\n"
    );
}

#[test]
fn uppercase_sigma_variants_both_map_to_capital_sigma() {
    assert_state!(
        "-[οσο]>\n",
        |(text, sels)| make_text_uppercase(text, sels),
        "-[ΟΣΟ]>\n"
    );
    assert_state!(
        "-[οος]>\n",
        |(text, sels)| make_text_uppercase(text, sels),
        "-[ΟΟΣ]>\n"
    );
}

#[test]
fn capitalize_resolves_mid_word_sigma_by_context() {
    // First grapheme uppercases to 'Ο'; the rest ("σο") lowercases as one
    // string, so the mid-word sigma stays 'σ', not the final form 'ς'.
    assert_state!(
        "-[οσο]>\n",
        |(text, sels)| make_text_capitalized(text, sels),
        "-[Οσο]>\n"
    );
}

#[test]
fn capitalize_resolves_word_final_sigma_by_context() {
    // A per-grapheme loop yields "Οοσ" here instead of "Οος" (same
    // default-mapping trap as lowercase).
    assert_state!(
        "-[ΟΟΣ]>\n",
        |(text, sels)| make_text_capitalized(text, sels),
        "-[Οος]>\n"
    );
}
