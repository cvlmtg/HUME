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
