//! Conceal rendering: a per-pane CONCEAL-kind `DecorationSource` fed from the
//! flattened `conceals` decoration store, keyed by line so
//! `decorations_for_line` is a plain map lookup.

use rustc_hash::FxHashMap;

use hume_engine::lock::SharedSlot;
use hume_engine::providers::{Conceal, Decoration, DecorationKinds, DecorationSource};
use hume_rope::line::ContentLine;

pub(crate) type ConcealMap = SharedSlot<FxHashMap<ContentLine, Vec<Conceal>>>;

pub(crate) struct PaneConceals {
    pub(crate) data: ConcealMap,
}

impl DecorationSource for PaneConceals {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::CONCEAL
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if let Some(conceals) = self.data.read().get(&line_idx) {
            out.extend(conceals.iter().cloned().map(Decoration::Conceal));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hume_engine::providers::{ConcealSpan, ConcealText};
    use hume_rope::column::ByteCol;
    use hume_rope::offset::ExclusiveRange;

    fn hidden(start: usize, end: usize) -> Conceal {
        Conceal {
            span: ConcealSpan::Hidden {
                bytes: ExclusiveRange::new(ByteCol::new(start), ByteCol::new(end)),
                text: None,
            },
            reveal: None,
        }
    }

    fn inserted(at: usize) -> Conceal {
        Conceal {
            span: ConcealSpan::Inserted {
                at: ByteCol::new(at),
                text: ConcealText {
                    text: "*".into(),
                    scope: None,
                },
            },
            reveal: None,
        }
    }

    fn bytes_of(out: &[Decoration]) -> Vec<(usize, usize)> {
        out.iter()
            .map(|d| {
                let Decoration::Conceal(c) = d else {
                    panic!("a conceal source emitted another kind");
                };
                (c.bytes().start.index(), c.bytes().end.index())
            })
            .collect()
    }

    #[test]
    fn pane_conceals_answers_its_line_and_nothing_else() {
        let provider = PaneConceals {
            data: SharedSlot::new(FxHashMap::default()),
        };
        provider.data.set(FxHashMap::from_iter([
            (ContentLine::new(1), vec![hidden(0, 2), inserted(5)]),
            (ContentLine::new(3), vec![hidden(4, 6)]),
        ]));

        let mut out = Vec::new();
        provider.decorations_for_line(ContentLine::new(1), &mut out);
        assert_eq!(bytes_of(&out), [(0, 2), (5, 5)]);

        out.clear();
        provider.decorations_for_line(ContentLine::new(2), &mut out);
        assert!(out.is_empty());

        provider.decorations_for_line(ContentLine::new(3), &mut out);
        assert_eq!(bytes_of(&out), [(4, 6)]);
        assert_eq!(provider.kinds(), DecorationKinds::CONCEAL);
    }
}
