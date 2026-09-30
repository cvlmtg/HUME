//! Text samples covering the grapheme-cluster classes a text primitive can
//! mishandle. Every sample is written with `\u{..}` escapes so the source
//! shows the codepoints, and each is one grapheme cluster unless its
//! doc says otherwise.

/// `é` as one codepoint (U+00E9).
pub const PRECOMPOSED: &str = "\u{e9}";

/// `é` as `e` + combining acute: two chars, one cluster.
pub const COMBINING: &str = "e\u{301}";

/// `ä` with an extra acute stacked on top: three chars, one cluster.
pub const STACKED: &str = "a\u{308}\u{301}";

/// A combining mark with no base: a cluster of its own at the start of the
/// text or after a newline, and part of the previous cluster anywhere else.
pub const LONE_MARK: &str = "\u{301}";

/// Family emoji joined by ZWJ: five chars, one cluster.
pub const ZWJ_FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";

/// Regional-indicator pair `IT`: two chars, one cluster.
pub const FLAG_IT: &str = "\u{1f1ee}\u{1f1f9}";

/// Two flag pairs (`IT` `FR`): four chars, two clusters. Deleting the
/// first indicator re-pairs the rest.
pub const FLAG_RUN: &str = "\u{1f1ee}\u{1f1f9}\u{1f1eb}\u{1f1f7}";

/// A single regional indicator: pairs with a neighbour when one appears.
pub const LONE_INDICATOR: &str = "\u{1f1eb}";

/// Waving hand with a skin-tone modifier: two chars, one cluster.
pub const SKIN_TONE: &str = "\u{1f44b}\u{1f3fd}";

/// Heart with variation selector 16: two chars, one cluster.
pub const VS16: &str = "\u{2764}\u{fe0f}";

/// Wide CJK ideograph: one char, two terminal cells, three UTF-8 bytes.
pub const CJK: &str = "\u{6f22}";

/// Fullwidth `A` (U+FF21): one char, two terminal cells.
pub const FULLWIDTH: &str = "\u{ff21}";

/// Grinning face: one char, four UTF-8 bytes, a UTF-16 surrogate pair.
pub const ASTRAL: &str = "\u{1f600}";

/// Hangul jamo sequence (choseong, jungseong, jongseong): three chars, one cluster.
pub const HANGUL_JAMO: &str = "\u{1100}\u{1161}\u{11a8}";

/// No-break space.
pub const NBSP: &str = "\u{a0}";

/// Ideographic space (U+3000).
pub const IDEO_SPACE: &str = "\u{3000}";

/// Arabic number sign (a Prepend character) followed by `x`: the prepend
/// glues to the next char, so this is one cluster.
pub const PREPEND: &str = "\u{600}x";

/// Every sample above, for property tests that draw atoms from the corpus.
pub const ALL: &[&str] = &[
    PRECOMPOSED,
    COMBINING,
    STACKED,
    LONE_MARK,
    ZWJ_FAMILY,
    FLAG_IT,
    FLAG_RUN,
    LONE_INDICATOR,
    SKIN_TONE,
    VS16,
    CJK,
    FULLWIDTH,
    ASTRAL,
    HANGUL_JAMO,
    NBSP,
    IDEO_SPACE,
    PREPEND,
];

/// The samples that are one grapheme cluster on their own.
pub fn single_clusters() -> impl Iterator<Item = &'static str> {
    ALL.iter().copied().filter(|s| clusters(s) == 1)
}

/// The samples that keep their own cluster boundary after a char other than
/// a newline: every one but [`LONE_MARK`], which would attach to it.
pub fn standalone() -> impl Iterator<Item = &'static str> {
    ALL.iter().copied().filter(|&s| s != LONE_MARK)
}

/// The [`standalone`] samples that are not blank: all but [`NBSP`] and
/// [`IDEO_SPACE`].
pub fn non_blank() -> impl Iterator<Item = &'static str> {
    standalone().filter(|&s| s != NBSP && s != IDEO_SPACE)
}

fn clusters(s: &str) -> usize {
    unicode_segmentation::UnicodeSegmentation::graphemes(s, true).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_sample_has_the_cluster_and_char_count_its_doc_states() {
        let expected: &[(&str, usize, usize)] = &[
            (PRECOMPOSED, 1, 1),
            (COMBINING, 1, 2),
            (STACKED, 1, 3),
            (LONE_MARK, 1, 1),
            (ZWJ_FAMILY, 1, 5),
            (FLAG_IT, 1, 2),
            (FLAG_RUN, 2, 4),
            (LONE_INDICATOR, 1, 1),
            (SKIN_TONE, 1, 2),
            (VS16, 1, 2),
            (CJK, 1, 1),
            (FULLWIDTH, 1, 1),
            (ASTRAL, 1, 1),
            (HANGUL_JAMO, 1, 3),
            (NBSP, 1, 1),
            (IDEO_SPACE, 1, 1),
            (PREPEND, 1, 2),
        ];
        assert_eq!(expected.len(), ALL.len());
        assert_eq!(single_clusters().count(), ALL.len() - 1, "all but FLAG_RUN");
        assert_eq!(standalone().count(), ALL.len() - 1, "all but LONE_MARK");
        assert_eq!(non_blank().count(), ALL.len() - 3, "and not the two spaces");
        for &(sample, cluster_count, char_count) in expected {
            assert!(ALL.contains(&sample), "{sample:?} missing from ALL");
            assert_eq!(clusters(sample), cluster_count, "clusters in {sample:?}");
            assert_eq!(sample.chars().count(), char_count, "chars in {sample:?}");
        }
    }
}
