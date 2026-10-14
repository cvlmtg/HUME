use std::ops::RangeInclusive;

/// The bytes of the current text that recorded edits may have changed:
/// every inserted byte, the byte right after each insertion, and the byte
/// now at each point where text was deleted. Closed intervals in the
/// current text's coordinates, sorted, no two overlapping or adjacent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditedBytes {
    regions: Vec<RangeInclusive<usize>>,
}

impl EditedBytes {
    /// Record `edit`, moving the regions already recorded into the edited
    /// text's coordinates. The edits of one change are recorded in the
    /// order `Tree::edit` takes them, right to left.
    pub(crate) fn record(&mut self, edit: &tree_sitter::InputEdit) {
        let (start, old_end, new_end) = (edit.start_byte, edit.old_end_byte, edit.new_end_byte);
        let moved = |byte: usize| byte - old_end + new_end;
        let mut old = std::mem::take(&mut self.regions).into_iter().peekable();
        let mut regions = Vec::with_capacity(old.len() + 1);
        while let Some(region) = old.next_if(|region| *region.end() < start) {
            regions.push(region);
        }
        let mut edited = start..=new_end;
        while let Some(region) = old.next_if(|region| *region.start() <= old_end) {
            let (first, last) = region.into_inner();
            let last = if last > old_end { moved(last) } else { new_end };
            edited = first.min(*edited.start())..=last.max(*edited.end());
        }
        push_merging(&mut regions, edited);
        for region in old {
            push_merging(&mut regions, moved(*region.start())..=moved(*region.end()));
        }
        self.regions = regions;
    }

    /// Whether an edited byte lies in one of `ranges`, in the part of the
    /// range's start row before it, or right after it. The lexer counts a
    /// column over the included text of its row only, so text outside the
    /// ranges changes the parse through a line break before a range on its
    /// start row. An edit adding or removing that line break meets the
    /// row's start.
    pub(crate) fn touches(&self, ranges: &[tree_sitter::Range]) -> bool {
        ranges.iter().any(|range| {
            let row_start = range.start_byte.saturating_sub(range.start_point.column);
            self.within(row_start..=range.end_byte).next().is_some()
        })
    }

    /// The parts of the regions inside `hull`, in order.
    pub(crate) fn within(
        &self,
        hull: RangeInclusive<usize>,
    ) -> impl Iterator<Item = RangeInclusive<usize>> + '_ {
        let (low, high) = hull.into_inner();
        let first = self.regions.partition_point(|region| *region.end() < low);
        self.regions[first..]
            .iter()
            .take_while(move |region| *region.start() <= high)
            .map(move |region| *region.start().max(&low)..=*region.end().min(&high))
    }

    #[cfg(test)]
    pub(crate) fn regions(&self) -> &[RangeInclusive<usize>] {
        &self.regions
    }
}

/// Push `region` onto `regions`, which it does not start before, merging
/// it into the last one when the two overlap or are adjacent.
pub(crate) fn push_merging(
    regions: &mut Vec<RangeInclusive<usize>>,
    region: RangeInclusive<usize>,
) {
    debug_assert!(
        regions
            .last()
            .is_none_or(|last| last.start() <= region.start()),
        "push_merging: {region:?} starts before the last region"
    );
    match regions.last_mut() {
        Some(last) if *region.start() <= last.end() + 1 => {
            *last = *last.start()..=*last.end().max(region.end());
        }
        _ => regions.push(region),
    }
}

#[cfg(test)]
mod tests;
