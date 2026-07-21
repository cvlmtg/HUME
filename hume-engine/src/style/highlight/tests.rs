use super::*;
use crate::theme::ScopeRegistry;
use crate::types::ScopeId;

fn make_scope_ids(names: &[&'static str]) -> (ScopeRegistry, Vec<ScopeId>) {
    let mut reg = ScopeRegistry::new();
    let ids = names.iter().map(|&n| reg.intern(n)).collect();
    (reg, ids)
}

#[test]
fn interval_cursor_basic() {
    let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
    let (kw, fn_) = (ids[0], ids[1]);
    let intervals = vec![(2, 5, kw), (7, 9, fn_)];
    let mut cursor = IntervalCursor::new(&intervals);
    assert_eq!(cursor.scope_at(0), None);
    assert_eq!(cursor.scope_at(2), Some(kw));
    assert_eq!(cursor.scope_at(4), Some(kw));
    assert_eq!(cursor.scope_at(5), None);
    assert_eq!(cursor.scope_at(7), Some(fn_));
    assert_eq!(cursor.scope_at(9), None);
}

#[test]
fn interval_cursor_empty() {
    let mut cursor = IntervalCursor::<'_>::new(&[]);
    assert_eq!(cursor.scope_at(0), None);
    assert_eq!(cursor.scope_at(100), None);
}

#[test]
fn interval_cursor_adjacent_intervals() {
    // (2,5) and (5,8) are adjacent — byte 5 must match the second.
    let (_reg, ids) = make_scope_ids(&["kw", "fn"]);
    let (kw, fn_) = (ids[0], ids[1]);
    let intervals = vec![(2, 5, kw), (5, 8, fn_)];
    let mut cursor = IntervalCursor::new(&intervals);
    assert_eq!(cursor.scope_at(4), Some(kw));
    assert_eq!(cursor.scope_at(5), Some(fn_));
    assert_eq!(cursor.scope_at(7), Some(fn_));
    assert_eq!(cursor.scope_at(8), None);
}
