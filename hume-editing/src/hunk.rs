//! Line hunks between two texts, with the word spans inside them. Two
//! producers fill the same shape: [`change_hunks`] from a changeset, and
//! [`text_hunks`] from a line diff.

use std::ops::Range;
use std::time::{Duration, Instant};

use ropey::RopeSlice;

use hume_rope::column::CharCol;
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::lines::{line_token_content, strip_line_break};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::changeset::{ChangeSet, EditedRegion};
use crate::diff::{LineHunk, LineHunkKind, diff_lines, word_change_ranges};
use crate::text::BufferText;

/// Time [`text_hunks`] spends word-diffing all its hunks together. The call
/// runs on the main thread after each debounced edit, and spans are only a
/// highlight, so the budget is one 60 Hz frame. Hunks left when it runs out
/// get no spans.
const HUNK_WORDS_BUDGET: Duration = Duration::from_millis(16);

/// One run of changed lines between two texts, with 0-based starts. Lines
/// carry no line break, and `Equal` runs are never represented. A side with
/// no lines marks the point the other side was inserted at or deleted from, so
/// a start may equal that text's content line count, which is why starts are
/// minted trusted (`ContentLine::new`) and not through
/// `ContentLine::checked`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeHunk {
    /// Line in the old text where the hunk starts.
    pub old_start: ContentLine,
    /// Line in the new text where the hunk starts.
    pub new_start: ContentLine,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    /// Lists are empty when no span lies strictly inside a line.
    pub words: WordSpans,
}

/// The changed text inside a hunk's lines, per side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordSpans {
    pub old: Vec<LineSpan>,
    /// Char ranges of the new text.
    pub new: Vec<ExclusiveRange<CharOffset>>,
}

/// A char-column range inside one line of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSpan {
    pub line: ContentLine,
    pub start: CharCol,
    pub end: CharCol,
}

/// The whole-line changes `cs` makes to `text`, as hunks of `text`'s own lines
/// (the new side) against the lines of the text `cs` produces (the old side).
/// A hunk's old-side start is a line of the produced text.
///
/// Word spans are what `cs` itself deletes (new side, char ranges of `text`)
/// and inserts (old side, columns of the produced text's lines), so no text
/// diff runs.
pub fn change_hunks(cs: &ChangeSet, text: &BufferText) -> Vec<ChangeHunk> {
    let regions = cs.edited_regions();
    let lines: Vec<_> = regions.iter().map(|r| affected_lines(r, text)).collect();
    let mut hunks = Vec::new();
    // Old-side lines minus new-side lines over the hunks emitted so far.
    let mut skew = 0isize;
    let mut i = 0;
    while i < regions.len() {
        let (first, mut end) = lines[i];
        let mut j = i + 1;
        while j < regions.len() && lines[j].0 <= end {
            end = end.max(lines[j].1);
            j += 1;
        }
        if let Some(hunk) = group_hunk(text, &regions[i..j], first, end, skew) {
            skew += hunk.old_lines.len() as isize - hunk.new_lines.len() as isize;
            hunks.push(hunk);
        }
        i = j;
    }
    hunks
}

/// The lines of `text` a region touches, `first..end`. The line the region's
/// end lands on is left out when the region ends at that line's start and the
/// text before it ends a line, so a pure insertion of whole lines touches none.
fn affected_lines(region: &EditedRegion<'_>, text: &BufferText) -> (RopeyLine, RopeyLine) {
    let first = text.ropey_char_to_line(region.old.start);
    let last = text.ropey_char_to_line(region.old.end);
    let ends_at_line_start = text.line_to_char(last) == region.old.end;
    let before_end = region.inserted.chars().next_back().or_else(|| {
        region
            .old
            .start
            .index()
            .checked_sub(1)
            .and_then(|i| text.char_at(CharOffset::new(i)))
    });
    let line_start_intact = ends_at_line_start && before_end.is_none_or(|ch| ch == '\n');
    (
        first,
        if line_start_intact {
            last
        } else {
            last.advance(1)
        },
    )
}

/// One group of regions over the lines `first..end` as a hunk, or `None` when
/// the regions cancel out and both sides are the same lines.
fn group_hunk(
    text: &BufferText,
    regions: &[EditedRegion<'_>],
    first: RopeyLine,
    end: RopeyLine,
    skew: isize,
) -> Option<ChangeHunk> {
    let start = text.line_to_char(first);
    let stop = text.line_to_char(end);
    let slice = |from: CharOffset, to: CharOffset| text.slice(ExclusiveRange::new(from, to));

    let new_text = slice(start, stop).to_string();
    let mut old_text = String::new();
    let mut old_chars = 0;
    let mut cursor = start;
    let (mut old_ranges, mut new_ranges) = (Vec::new(), Vec::new());
    for region in regions {
        old_chars += region.old.start.chars_since(cursor);
        slice(cursor, region.old.start)
            .chunks()
            .for_each(|c| old_text.push_str(c));
        let inserted_chars = region.inserted.chars().count();
        old_ranges.push(old_chars..old_chars + inserted_chars);
        old_chars += inserted_chars;
        old_text.push_str(&region.inserted);
        new_ranges.push(region.old.start.chars_since(start)..region.old.end.chars_since(start));
        cursor = region.old.end;
    }
    slice(cursor, stop)
        .chunks()
        .for_each(|c| old_text.push_str(c));

    let old_lines: Vec<&str> = split_lines(&old_text);
    let new_lines: Vec<&str> = split_lines(&new_text);

    let lead = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(o, n)| o == n)
        .count();
    let trail = old_lines[lead..]
        .iter()
        .rev()
        .zip(new_lines[lead..].iter().rev())
        .take_while(|(o, n)| o == n)
        .count();
    let old_kept = &old_lines[lead..old_lines.len() - trail];
    let new_kept = &new_lines[lead..new_lines.len() - trail];
    if old_kept.is_empty() && new_kept.is_empty() {
        return None;
    }

    let new_start = first.index() + lead;
    let old_start = new_start
        .checked_add_signed(skew)
        .expect("an old-side start is never negative");
    let (old_start, new_start) = (ContentLine::new(old_start), ContentLine::new(new_start));
    Some(ChangeHunk {
        old_start,
        new_start,
        old_lines: old_kept.iter().map(|l| (*l).to_owned()).collect(),
        new_lines: new_kept.iter().map(|l| (*l).to_owned()).collect(),
        words: WordSpans {
            old: line_spans(
                clipped(&old_lines, &old_ranges, lead..lead + old_kept.len()),
                lead,
                old_start,
            ),
            new: char_ranges(
                clipped(&new_lines, &new_ranges, lead..lead + new_kept.len()),
                start,
            ),
        },
    })
}

/// The line hunks of `new` against `old`, `Equal` runs dropped. Both texts
/// are buffer content, so a caller holding a raw string normalizes it through
/// [`BufferText::from`]: line endings become LF and a missing final newline
/// is added, so a ref blob without one shows no phantom trailing hunk.
///
/// Word spans come from a word diff of each hunk's two sides joined by line
/// breaks, so an edit is marked where it is whatever the line counts on
/// either side. All hunks share `HUNK_WORDS_BUDGET`.
pub fn text_hunks(old: &BufferText, new: &BufferText) -> Vec<ChangeHunk> {
    text_hunks_with_budget(old, new, HUNK_WORDS_BUDGET)
}

fn text_hunks_with_budget(old: &BufferText, new: &BufferText, budget: Duration) -> Vec<ChangeHunk> {
    let old_tokens: Vec<RopeSlice<'_>> = old.line_tokens().collect();
    let new_tokens: Vec<RopeSlice<'_>> = new.line_tokens().collect();
    let deadline = Instant::now() + budget;
    diff_lines(&old_tokens, &new_tokens)
        .hunks
        .into_iter()
        .filter(|hunk| hunk.kind != LineHunkKind::Equal)
        .map(
            |LineHunk {
                 old: old_range,
                 new: new_range,
                 ..
             }| {
                let old_lines = token_lines(&old_tokens, old_range.clone());
                let new_lines = token_lines(&new_tokens, new_range.clone());
                // The phantom trailing line is an empty token on both sides, so it
                // only ever matches as `Equal`: a start is a content line or the
                // one-past-last insertion point.
                let old_start = ContentLine::new(old_range.start);
                let new_start = ContentLine::new(new_range.start);
                ChangeHunk {
                    old_start,
                    new_start,
                    words: text_hunk_words(
                        new,
                        (old_start, &old_lines),
                        (new_start, &new_lines),
                        deadline,
                    ),
                    old_lines,
                    new_lines,
                }
            },
        )
        .collect()
}

/// `tokens[range]` as owned lines without their line breaks.
fn token_lines(tokens: &[RopeSlice<'_>], range: Range<usize>) -> Vec<String> {
    tokens[range]
        .iter()
        .copied()
        .map(line_token_content)
        .collect()
}

/// Empty when a side has no lines or `deadline` passes first. Each side is
/// its first line in its text and its lines; `new_text` is the text the new
/// side's lines are from.
fn text_hunk_words(
    new_text: &BufferText,
    (old_start, old_lines): (ContentLine, &[String]),
    (new_start, new_lines): (ContentLine, &[String]),
    deadline: Instant,
) -> WordSpans {
    let left = deadline.saturating_duration_since(Instant::now());
    if old_lines.is_empty() || new_lines.is_empty() || left.is_zero() {
        return WordSpans::default();
    }
    let Some((old_ranges, new_ranges)) =
        word_change_ranges(&old_lines.join("\n"), &new_lines.join("\n"), left)
    else {
        return WordSpans::default();
    };
    WordSpans {
        old: line_spans(
            clipped(old_lines, &old_ranges, 0..old_lines.len()),
            0,
            old_start,
        ),
        new: char_ranges(
            clipped(new_lines, &new_ranges, 0..new_lines.len()),
            new_text.line_to_char(RopeyLine::from(new_start)),
        ),
    }
}

/// `s` split at its line breaks, without them. A trailing break ends the last
/// line instead of starting an empty one.
fn split_lines(s: &str) -> Vec<&str> {
    s.split_inclusive('\n').map(strip_line_break).collect()
}

/// One part of a word range that lies inside one line's content. `index` is
/// the line's index in its side's lines, and `line_start`, `start` and `end`
/// are char offsets into those lines joined by one break each.
struct Clipped {
    index: usize,
    line_start: usize,
    start: usize,
    end: usize,
}

/// The parts of `ranges` (char ranges of `lines` joined by one break each)
/// that fall inside the contents of the lines in `kept`. A part covering a
/// whole line is dropped: the line already counts as changed.
fn clipped(lines: &[impl AsRef<str>], ranges: &[Range<usize>], kept: Range<usize>) -> Vec<Clipped> {
    let mut parts = Vec::new();
    let mut line_start = 0;
    let mut next_range = 0;
    for (index, line) in lines.iter().enumerate().take(kept.end) {
        let len = line.as_ref().chars().count();
        if index >= kept.start {
            while next_range < ranges.len() && ranges[next_range].end <= line_start {
                next_range += 1;
            }
            for range in ranges[next_range..]
                .iter()
                .take_while(|r| r.start < line_start + len)
            {
                let (start, end) = (range.start.max(line_start), range.end.min(line_start + len));
                let whole = start == line_start && end == line_start + len;
                if start < end && !whole {
                    parts.push(Clipped {
                        index,
                        line_start,
                        start,
                        end,
                    });
                }
            }
        }
        line_start += len + 1;
    }
    parts
}

/// `parts` as char columns of their lines, where the line at index
/// `kept_start` is `first` in its text.
fn line_spans(parts: Vec<Clipped>, kept_start: usize, first: ContentLine) -> Vec<LineSpan> {
    parts
        .into_iter()
        .map(|part| LineSpan {
            line: first.advance(part.index - kept_start),
            start: CharCol::new(part.start - part.line_start),
            end: CharCol::new(part.end - part.line_start),
        })
        .collect()
}

/// `parts` as char ranges of a text whose char `base` is where their side's
/// first line starts.
fn char_ranges(parts: Vec<Clipped>, base: CharOffset) -> Vec<ExclusiveRange<CharOffset>> {
    parts
        .into_iter()
        .map(|part| {
            ExclusiveRange::new(
                base.shift(part.start as isize),
                base.shift(part.end as isize),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests;
