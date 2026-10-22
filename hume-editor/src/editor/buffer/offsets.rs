//! Checking char offsets and ranges a script computed against the text
//! they claim to address.

use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, ExclusiveRange};

/// A char offset a script computed for `text`, which may be `text`'s length
/// but not past it: past it is a caller bug, so `Err`.
pub(in crate::editor) fn checked_char_offset(
    text: &BufferText,
    idx: usize,
) -> Result<CharOffset, String> {
    CharOffset::checked(text.rope(), idx).ok_or_else(|| {
        format!(
            "offset {idx} is past the end of the buffer ({} chars)",
            text.len_chars()
        )
    })
}

/// The char range `start..end` a script computed for `text`: each end checked
/// as by [`checked_char_offset`], and `end` not before `start`. The error
/// names the offending end or the reversed range, for the caller to prefix
/// with what the range is.
pub(in crate::editor) fn checked_char_range(
    text: &BufferText,
    start: usize,
    end: usize,
) -> Result<ExclusiveRange<CharOffset>, String> {
    let start_at = checked_char_offset(text, start).map_err(|e| format!("start: {e}"))?;
    let end_at = checked_char_offset(text, end).map_err(|e| format!("end: {e}"))?;
    if end_at < start_at {
        return Err(format!("{start}..{end} ends before it starts"));
    }
    Ok(ExclusiveRange::new(start_at, end_at))
}

/// `range`, a checked range of `text`, on cluster boundaries: an edge inside
/// a grapheme cluster widens it to the whole cluster, and an empty range
/// inside one moves to the cluster's start, so no range it returns splits one.
pub(in crate::editor) fn widened_to_clusters(
    text: &BufferText,
    range: ExclusiveRange<CharOffset>,
) -> ExclusiveRange<CharOffset> {
    if range.is_empty() {
        let at = if range.start == text.end() {
            range.start
        } else {
            text.snap(range.start).offset()
        };
        return ExclusiveRange::new(at, at);
    }
    text.covering(range)
        .expect("a non-empty range within the text covers a cluster")
        .chars()
}
