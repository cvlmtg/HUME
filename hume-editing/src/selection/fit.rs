//! The one place a selection set is checked against its text.

use super::{Selection, SelectionSet};
use crate::error::InvariantViolation;
use crate::text::BufferText;

/// Checks that `selections` belongs to `text`.
///
/// A set tagged for another text is a bug in whoever paired them, and reading
/// it would act on the wrong text. Debug builds also check every invariant of
/// a set tagged for `text`.
///
/// # Panics
/// Panics if `selections` is tagged for another text.
pub(crate) fn assert_fits(text: &BufferText, selections: &SelectionSet) {
    assert_eq!(
        selections.version(),
        text.version(),
        "a selection set was paired with another text"
    );
    debug_assert_eq!(
        check_positions(text, selections),
        Ok(()),
        "a selection set breaks an invariant of the text it is tagged with"
    );
}

/// `selections`, each end clamped into `text` and floored to the start of the
/// cluster holding it, merged where they then share a cluster and tagged with
/// `text`'s version.
pub(crate) fn refit_parts(
    text: &BufferText,
    selections: &[Selection],
    primary: usize,
) -> SelectionSet {
    let fitted = selections
        .iter()
        .map(|sel| {
            sel.with_ends(
                text.snap(sel.first().offset()),
                text.snap(sel.last().offset()),
            )
        })
        .collect();
    SelectionSet::from_parts(fitted, primary, text.version())
}

/// Every invariant of `selections` against `text` except its version tag.
pub(crate) fn check_positions(
    text: &BufferText,
    selections: &SelectionSet,
) -> Result<(), InvariantViolation> {
    let sels = selections.selections();
    if sels.is_empty() {
        return Err(InvariantViolation::Empty);
    }
    if selections.primary_pos() >= sels.len() {
        return Err(InvariantViolation::PrimaryOutOfRange {
            primary: selections.primary_pos(),
            len: sels.len(),
        });
    }
    let len = text.len_chars();
    for (index, sel) in sels.iter().enumerate() {
        for end in [sel.anchor(), sel.head()] {
            if end.offset().index() >= len {
                return Err(InvariantViolation::OutOfBounds { index });
            }
            if text.snap(end.offset()) != end {
                return Err(InvariantViolation::SplitsCluster { index });
            }
        }
        if index > 0 && sels[index - 1].last() >= sel.first() {
            return Err(InvariantViolation::Overlapping { index });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
