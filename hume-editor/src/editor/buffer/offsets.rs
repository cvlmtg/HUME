//! Char ranges a script computed, fitted to whole grapheme clusters.

use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterRange;
use hume_rope::offset::{CharOffset, ExclusiveRange};

/// The non-empty char range `start..end` a script computed for `text`,
/// checked as by [`BufferText::checked_range`] and widened to the clusters
/// it touches. The error is worded as [`BufferText::checked_range`]'s.
pub(in crate::editor) fn checked_cluster_range(
    text: &BufferText,
    start: usize,
    end: usize,
) -> Result<ClusterRange, String> {
    let range = text.checked_range(start, end)?;
    text.covering(range)
        .ok_or_else(|| format!("{start}..{end} is empty"))
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
