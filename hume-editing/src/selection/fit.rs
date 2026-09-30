//! The one place a selection set is checked against its text.

use super::{Selection, SelectionSet};
use crate::error::InvariantViolation;
use crate::text::BufferText;

/// Checks that `selections` belongs to `text`.
///
/// A set tagged for another text is a bug in whoever paired them, and reading
/// it would act on the wrong text.
///
/// # Panics
/// Panics if `selections` is tagged for another text, or as
/// [`assert_positions`] does.
pub(crate) fn assert_fits(text: &BufferText, selections: &SelectionSet) {
    assert_eq!(
        selections.version(),
        text.version(),
        "a selection set was paired with another text"
    );
    assert_positions(text, selections);
}

/// Checks `selections`' positions against `text`, whatever its version tag.
///
/// # Panics
/// Panics if a selection ends past the text end, the check that stays cheap
/// in release builds. Debug builds also check every other invariant.
pub(crate) fn assert_positions(text: &BufferText, selections: &SelectionSet) {
    let len = text.len_chars();
    assert!(
        selections
            .selections()
            .iter()
            .all(|sel| sel.last().offset().index() < len),
        "a selection set reaches past the end of its text"
    );
    debug_assert_eq!(
        check_positions(text, selections),
        Ok(()),
        "a selection set breaks an invariant of the text it is paired with"
    );
}

/// Every invariant of `selections` against `text`, the version tag included.
pub(crate) fn check_fit(
    text: &BufferText,
    selections: &SelectionSet,
) -> Result<(), InvariantViolation> {
    if selections.version() != text.version() {
        return Err(InvariantViolation::VersionMismatch);
    }
    check_positions(text, selections)
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
                text.snap(sel.start().offset()),
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
    for (index, sel) in sels.iter().enumerate() {
        check_selection(text, sel, index)?;
        if index > 0 && sels[index - 1].last() >= sel.start() {
            return Err(InvariantViolation::Overlapping { index });
        }
    }
    Ok(())
}

/// Whether both ends of `sel`, the selection at `index`, start clusters of
/// `text`.
pub(crate) fn check_selection(
    text: &BufferText,
    sel: &Selection,
    index: usize,
) -> Result<(), InvariantViolation> {
    let len = text.len_chars();
    for end in [sel.anchor(), sel.head()] {
        if end.offset().index() >= len {
            return Err(InvariantViolation::OutOfBounds { index });
        }
        if text.snap(end.offset()) != end {
            return Err(InvariantViolation::SplitsCluster { index });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
