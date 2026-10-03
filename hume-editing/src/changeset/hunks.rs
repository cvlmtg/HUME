use hume_rope::column::CharCol;
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::lines::strip_line_break;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::{ChangeSet, EditedRegion};
use crate::text::BufferText;

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
    /// `None` for a hunk that came from a text diff, which knows no spans.
    /// Every hunk of a changeset carries `Some`, its lists empty when no
    /// span lies strictly inside a line.
    pub words: Option<WordSpans>,
}

/// The text a changeset inserted (old side) and deleted (new side), by line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpans {
    pub old: Vec<LineSpan>,
    pub new: Vec<LineSpan>,
}

/// A char-column range inside one line of a hunk side, `line` counted from the
/// hunk's first line on that side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSpan {
    pub line: usize,
    pub start: CharCol,
    pub end: CharCol,
}

/// The whole-line changes `cs` makes to `text`, as hunks of `text`'s own lines
/// (the new side) against the lines of the text `cs` produces (the old side).
/// A hunk's old-side start is a line of the produced text.
///
/// Word spans are the char columns `cs` itself deletes (new side) and inserts
/// (old side), so no text diff runs.
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
        old_ranges.push((old_chars, old_chars + inserted_chars));
        old_chars += inserted_chars;
        old_text.push_str(&region.inserted);
        new_ranges.push((
            region.old.start.chars_since(start),
            region.old.end.chars_since(start),
        ));
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
    Some(ChangeHunk {
        old_start: ContentLine::new(old_start),
        new_start: ContentLine::new(new_start),
        old_lines: old_kept.iter().map(|l| (*l).to_owned()).collect(),
        new_lines: new_kept.iter().map(|l| (*l).to_owned()).collect(),
        words: Some(WordSpans {
            old: line_spans(&old_lines, &old_ranges, lead, old_kept.len()),
            new: line_spans(&new_lines, &new_ranges, lead, new_kept.len()),
        }),
    })
}

/// `s` split at its line breaks, without them. A trailing break ends the last
/// line instead of starting an empty one.
fn split_lines(s: &str) -> Vec<&str> {
    s.split_inclusive('\n').map(strip_line_break).collect()
}

/// The parts of `ranges` (char ranges of `lines` joined by one break each)
/// that fall inside line contents, as spans of the lines `skip..skip + keep`
/// numbered from `skip`. A span covering a whole line is dropped: the line
/// already counts as changed.
fn line_spans(
    lines: &[&str],
    ranges: &[(usize, usize)],
    skip: usize,
    keep: usize,
) -> Vec<LineSpan> {
    let mut spans = Vec::new();
    let mut line_start = 0;
    let mut first = 0;
    for (index, line) in lines.iter().enumerate().take(skip + keep) {
        let len = line.chars().count();
        if index >= skip {
            while first < ranges.len() && ranges[first].1 <= line_start {
                first += 1;
            }
            for &(from, to) in ranges[first..]
                .iter()
                .take_while(|r| r.0 < line_start + len)
            {
                let (start, end) = (from.max(line_start), to.min(line_start + len));
                let whole = start == line_start && end == line_start + len;
                if start < end && !whole {
                    spans.push(LineSpan {
                        line: index - skip,
                        start: CharCol::new(start - line_start),
                        end: CharCol::new(end - line_start),
                    });
                }
            }
        }
        line_start += len + 1;
    }
    spans
}

#[cfg(test)]
mod tests;
