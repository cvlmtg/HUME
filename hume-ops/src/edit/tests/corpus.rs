//! Every command runs over the shared unicode corpus: a cursor (or selection)
//! on each sample between two ASCII chars.

use super::super::*;
use crate::auto_pairs::insert_pair_close;
use crate::edit::yank_selections;
use crate::register::Piece;
use crate::surround::wrap_each_selection;
use test_fixtures::assert_state;
use test_fixtures::testing::parse_state;
use test_fixtures::unicode::{ALL, LONE_MARK, single_clusters};
use unicode_segmentation::UnicodeSegmentation;

fn clusters(s: &str) -> usize {
    s.graphemes(true).count()
}

#[test]
fn delete_selection_removes_the_sample() {
    for s in ALL {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
            "x\n-[b]>\n"
        );
    }
}

#[test]
fn delete_char_forward_removes_one_cluster() {
    for s in single_clusters() {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
            "x\n-[b]>\n"
        );
    }
}

#[test]
fn delete_char_backward_removes_one_cluster() {
    for s in single_clusters() {
        assert_state!(
            &format!("x\n{s}-[b]>\n"),
            |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
            "x\n-[b]>\n"
        );
    }
}

#[test]
fn yank_returns_the_sample() {
    for s in ALL {
        let (text, sels) = parse_state(&format!("x\n-[{s}]>b\n"));
        assert_eq!(
            yank_selections(&hume_editing::state::EditState::bind(&text, sels.clone())),
            vec![s.to_string()]
        );
    }
}

#[test]
fn replace_selections_writes_one_char_per_cluster() {
    for s in ALL {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| replace_selections(test_fixtures::testing::state(text, sels), 'x'),
            &format!("x\n-[{}]>b\n", "x".repeat(clusters(s)))
        );
    }
}

#[test]
fn insert_char_replaces_a_selection_ending_on_the_sample() {
    for s in ALL {
        assert_state!(
            &format!("-[x\n{s}]>b\n"),
            |(text, sels)| insert_char(test_fixtures::testing::state(text, sels), 'Z'),
            "Z-[b]>\n"
        );
    }
}

#[test]
fn paste_after_lands_past_the_whole_cluster() {
    for s in single_clusters() {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| paste_after(
                test_fixtures::testing::state(text, sels),
                &[Piece::from("Z")]
            ),
            &format!("x\n{s}-[Z]>b\n")
        );
    }
}

#[test]
fn paste_before_selects_the_pasted_cluster() {
    for s in single_clusters() {
        let expected = if s == LONE_MARK {
            "x\n-[Z\u{301}]>b\n".to_string()
        } else {
            format!("x\n-[Z]>{s}b\n")
        };
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| paste_before(
                test_fixtures::testing::state(text, sels),
                &[Piece::from("Z")]
            ),
            &expected
        );
    }
}

#[test]
fn wrap_each_selection_surrounds_the_whole_sample() {
    for s in ALL {
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| wrap_each_selection(test_fixtures::testing::state(text, sels), '(', ')'),
            &format!("x\n({s}-[)]>b\n")
        );
    }
}

#[test]
fn insert_pair_close_lands_on_the_close_before_the_sample() {
    for s in ALL {
        let expected = if *s == LONE_MARK {
            "x\n(-[)\u{301}]>b\n".to_string()
        } else {
            format!("x\n(-[)]>{s}b\n")
        };
        assert_state!(
            &format!("x\n-[{s}]>b\n"),
            |(text, sels)| insert_pair_close(test_fixtures::testing::state(text, sels), '(', ')'),
            &expected
        );
    }
}
