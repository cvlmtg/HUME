use hume_editing::selection::Selection;

use super::*;

const LINES: [&str; 6] = [
    r#"[{"id":1234,"v":-56},{"t":"2024-01-31 23:59:58"},{"n":0x1F}]"#,
    "x-1 f(x)-1 a[i]-2 -0x1f 0xfg a0x1 -2024-01-01 {a}-3 $-5 =-6",
    "\u{e9}-1 e\u{301}5 1\u{301}2 \u{2082}3 12:34:56:78 1:23:45 0x1f\u{e9}",
    "item-3 v1.2.3 1_000_000 -_1 --5 2024/02/30T10:00 99:99 2024-01-01 9",
    "a 2024-01-31T23:59 b 23:59, 2024-12-31 x   -7",
    "no literal here, none at all",
];

/// The first literal of a line whose last cluster is cluster `at` or one
/// after it, as a cluster index range, from a scan of the whole line.
fn whole_line_reach(syms: &[Sym], at: usize) -> Option<Range<usize>> {
    literals(syms)
        .find(|found| found.end > at)
        .map(|found| found.start..found.end)
}

/// The literal [`target`] picks for the selection over clusters
/// `anchor..=head`, as a cluster index range.
fn windowed_reach(
    text: &BufferText,
    clusters: &[Cluster],
    anchor: usize,
    head: usize,
    chars: WordChars<'_>,
) -> Option<Range<usize>> {
    let state = EditState::at_text_start(text.clone()).with_selections(
        vec![Selection::new(
            clusters[anchor].start(),
            clusters[head].start(),
        )],
        0,
    );
    let (target, _) = target(state.view().primary(), chars)?;
    let index = |offset: CharOffset| {
        clusters
            .iter()
            .position(|cluster| cluster.start().offset() == offset)
            .expect("a part starts at a cluster")
    };
    let first = target.parts.first().expect("a literal has a part").start();
    let last = target.parts.last().expect("a literal has a part").last();
    Some(index(first.offset())..index(last.offset()) + 1)
}

#[test]
fn a_scan_from_the_break_before_a_selection_finds_what_a_whole_line_scan_finds() {
    for word_chars in ["", "-=", "$"] {
        let chars = WordChars::new(word_chars);
        for line in LINES {
            let text = BufferText::from(format!("{line}\n").as_str());
            let clusters: Vec<Cluster> = text
                .clusters()
                .graphemes_at(text.clusters().first().into())
                .take_while(|cluster| cluster.first() != '\n')
                .collect();
            let syms: Vec<Sym> = clusters.iter().map(|c| Sym::of(c, chars)).collect();
            for at in 0..clusters.len() {
                let expected = whole_line_reach(&syms, at);
                assert_eq!(
                    windowed_reach(&text, &clusters, at, at, chars),
                    expected,
                    "cursor at cluster {at} of {line:?}, word-chars {word_chars:?}"
                );
                for head in at + 1..clusters.len().min(at + 6) {
                    let expected = expected.clone().filter(|range| range.start <= head);
                    assert_eq!(
                        windowed_reach(&text, &clusters, at, head, chars),
                        expected,
                        "selection over clusters {at}..={head} of {line:?}, \
                         word-chars {word_chars:?}"
                    );
                }
            }
        }
    }
}
