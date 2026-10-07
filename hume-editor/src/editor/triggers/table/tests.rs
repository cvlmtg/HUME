use std::collections::BTreeSet;
use std::sync::Arc;

use proptest::prelude::*;

use super::*;

fn hook(name: &str) -> Listener {
    Listener::Hook(Arc::from(name))
}

fn completion(name: &str) -> Listener {
    Listener::Completion(Arc::from(name))
}

fn firing_on(table: &TriggerTable, ch: char) -> Vec<Listener> {
    table.fires_on(ch).cloned().collect()
}

#[test]
fn listeners_order_hooks_before_completions_then_by_name() {
    let mut table = TriggerTable::default();
    for listener in [completion("a"), hook("z"), completion("b"), hook("m")] {
        table.set(listener, ['.']);
    }
    assert_eq!(
        firing_on(&table, '.'),
        [hook("m"), hook("z"), completion("a"), completion("b")]
    );
}

#[test]
fn a_set_replaces_the_previous_characters() {
    let mut table = TriggerTable::default();
    table.set(hook("sig"), ['(', ',']);
    table.set(hook("sig"), [')']);
    assert!(firing_on(&table, '(').is_empty());
    assert_eq!(firing_on(&table, ')'), [hook("sig")]);
    assert_eq!(table.len(), 1);
}

#[test]
fn an_empty_set_removes_the_listener() {
    let mut table = TriggerTable::default();
    table.set(hook("sig"), ['(']);
    table.set(hook("sig"), []);
    assert!(table.is_empty());
}

#[test]
fn a_hook_and_a_completion_source_of_one_name_are_distinct_listeners() {
    let mut table = TriggerTable::default();
    table.set(hook("both"), ['(']);
    table.set(completion("both"), ['.']);
    assert_eq!(firing_on(&table, '('), [hook("both")]);
    assert_eq!(firing_on(&table, '.'), [completion("both")]);
    assert_eq!(table.len(), 2);
}

#[test]
fn clear_empties_the_table() {
    let mut table = TriggerTable::default();
    table.set(hook("sig"), ['(']);
    table.clear();
    assert!(table.is_empty());
    assert!(firing_on(&table, '(').is_empty());
}

/// The specification written the slow way: an association list with
/// replace-on-set semantics and no invariant to maintain.
#[derive(Default)]
struct Model(Vec<(Listener, BTreeSet<char>)>);

impl Model {
    fn set(&mut self, listener: Listener, chars: BTreeSet<char>) {
        self.0.retain(|(l, _)| *l != listener);
        if !chars.is_empty() {
            self.0.push((listener, chars));
        }
    }

    fn fires_on(&self, ch: char) -> Vec<Listener> {
        let mut firing: Vec<Listener> = self
            .0
            .iter()
            .filter(|(_, chars)| chars.contains(&ch))
            .map(|(listener, _)| listener.clone())
            .collect();
        firing.sort();
        firing
    }
}

#[derive(Debug, Clone)]
enum Op {
    Set(Listener, Vec<char>),
    Clear,
}

/// A small alphabet, so sets overlap and repeat often.
const ALPHABET: [char; 4] = ['.', ':', '(', ')'];

fn arb_op() -> impl Strategy<Value = Op> {
    let listener = prop_oneof![
        Just(hook("a")),
        Just(hook("b")),
        Just(completion("a")),
        Just(completion("b")),
    ];
    let chars = proptest::collection::vec(proptest::sample::select(ALPHABET.to_vec()), 0..=5);
    prop_oneof![
        9 => (listener, chars).prop_map(|(l, c)| Op::Set(l, c)),
        1 => Just(Op::Clear),
    ]
}

proptest! {
    /// After every operation the table answers as the naive model
    /// does, for every character, and its size matches.
    #[test]
    fn prop_table_matches_the_model(ops in proptest::collection::vec(arb_op(), 1..=30)) {
        let mut table = TriggerTable::default();
        let mut model = Model::default();
        for op in ops {
            match op {
                Op::Set(listener, chars) => {
                    table.set(listener.clone(), chars.iter().copied());
                    model.set(listener, chars.into_iter().collect());
                }
                Op::Clear => {
                    table.clear();
                    model = Model::default();
                }
            }
            for ch in ALPHABET.into_iter().chain(['x']) {
                prop_assert_eq!(firing_on(&table, ch), model.fires_on(ch));
            }
            prop_assert_eq!(table.len(), model.0.len());
            prop_assert_eq!(table.is_empty(), model.0.is_empty());
        }
    }

    /// A character set is non-empty, sorted and duplicate-free, and agrees
    /// with the input on membership.
    #[test]
    fn prop_char_set_is_the_sorted_deduplicated_input(
        chars in proptest::collection::vec(proptest::char::any(), 0..=8),
        probe in proptest::char::any(),
    ) {
        let expected: Vec<char> = chars.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
        match CharSet::new(chars.iter().copied()) {
            None => prop_assert!(chars.is_empty()),
            Some(set) => {
                prop_assert_eq!(set.as_slice(), expected.as_slice());
                prop_assert_eq!(set.contains(probe), chars.contains(&probe));
            }
        }
    }
}
