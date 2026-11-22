//! `G T` and a direction move the table rows or columns the selections
//! touch one place, the selections staying on what they selected.

use super::*;
use crate::editor::tests::before_save::warning_texts;

fn g_t(direction: KeyEvent) -> [KeyEvent; 3] {
    [key('G'), key('T'), direction]
}

#[test]
fn a_body_row_moves_down_with_its_cursor() {
    assert_eq!(
        after(
            "| h1 | h2 |\n|----|----|\n| -[a]>1 | a2 |\n| b1 | b2 |\n",
            "GTj"
        ),
        "| h1 | h2 |\n|----|----|\n| b1 | b2 |\n| -[a]>1 | a2 |\n"
    );
}

#[test]
fn the_last_row_moves_up_with_its_line_selection() {
    assert_eq!(
        after("| h |\n|---|\n| a |\n| -[b]> |\n", "xGTk"),
        "| h |\n|---|\n-[| b |\n]>| a |\n"
    );
}

#[test]
fn a_row_moves_in_a_table_followed_by_text() {
    assert_eq!(
        after("| h |\n|---|\n| a |\n| -[b]> |\n\ntext\n", "GTk"),
        "| h |\n|---|\n| -[b]> |\n| a |\n\ntext\n"
    );
    assert_eq!(
        after("| h |\n|---|\n| -[a]> |\n| b |\n\ntext\n", "GTj"),
        "| h |\n|---|\n| b |\n| -[a]> |\n\ntext\n"
    );
}

#[test]
fn the_arrows_move_rows_as_j_and_k_do() {
    assert_eq!(
        after_keys("| h |\n|---|\n| -[a]> |\n| b |\n", g_t(key_down())),
        "| h |\n|---|\n| b |\n| -[a]> |\n"
    );
    assert_eq!(
        after_keys("| h |\n|---|\n| a |\n| -[b]> |\n", g_t(key_up())),
        "| h |\n|---|\n| -[b]> |\n| a |\n"
    );
}

#[test]
fn adjacent_selected_rows_move_as_one_block() {
    assert_eq!(
        after("| h |\n|---|\n| -[a]> |\n| -{b}> |\n| c |\n", "GTj"),
        "| h |\n|---|\n| c |\n| -[a]> |\n| -{b}> |\n"
    );
}

#[test]
fn separate_blocks_each_move() {
    assert_eq!(
        after("| h |\n|---|\n| -[a]> |\n| b |\n| -{c}> |\n| d |\n", "GTj"),
        "| h |\n|---|\n| b |\n| -[a]> |\n| d |\n| -{c}> |\n"
    );
    assert_eq!(
        after("| h |\n|---|\n| a |\n| -[b]> |\n| c |\n| -{d}> |\n", "GTk"),
        "| h |\n|---|\n| -[b]> |\n| a |\n| -{d}> |\n| c |\n"
    );
}

#[test]
fn rows_move_in_each_table_a_selection_touches() {
    assert_eq!(
        after(
            "| h |\n|---|\n| -[a]> |\n| b |\n\n| g |\n|---|\n| -{x}> |\n| y |\n",
            "GTj"
        ),
        "| h |\n|---|\n| b |\n| -[a]> |\n\n| g |\n|---|\n| y |\n| -{x}> |\n"
    );
}

#[test]
fn a_row_at_the_tables_edge_stays() {
    let last = "| h |\n|---|\n| a |\n| -[b]> |\n";
    assert_eq!(after(last, "GTj"), last);
    let first = "| h |\n|---|\n| -[a]> |\n| b |\n";
    assert_eq!(after(first, "GTk"), first);
}

#[test]
fn the_header_and_the_delimiter_row_stay() {
    let header = "| -[h]> |\n|---|\n| a |\n";
    assert_eq!(after(header, "GTj"), header);
    let delimiter = "| h |\n|-[-]>--|\n| a |\n";
    assert_eq!(after(delimiter, "GTj"), delimiter);
    let spanning = "| -[h |\n|---|\n| a]> |\n| b |\n";
    assert_eq!(after(spanning, "GTj"), spanning);
}

#[test]
fn nothing_moves_when_one_block_is_at_the_edge() {
    let marked = "| h |\n|---|\n| -[a]> |\n| b |\n| -{c}> |\n";
    assert_eq!(after(marked, "GTk"), marked);
}

#[test]
fn a_quoted_row_moves_with_its_marker() {
    assert_eq!(
        after("> | h |\n> |---|\n> | -[a]> |\n> | b |\n", "GTj"),
        "> | h |\n> |---|\n> | b |\n> | -[a]> |\n"
    );
}

#[test]
fn repeating_a_row_move_moves_it_again() {
    assert_eq!(
        after("| h |\n|---|\n| -[a]> |\n| b |\n| c |\n", "GTj."),
        "| h |\n|---|\n| b |\n| c |\n| -[a]> |\n"
    );
}

#[test]
fn one_undo_takes_back_one_move() {
    let mut p = editing("| h |\n|---|\n| -[a]> |\n| b |\n| c |\n");
    p.ed.feed_keys("GTj.u".chars().map(key));
    assert_eq!(text(&p.ed), "| h |\n|---|\n| b |\n| a |\n| c |\n");
}

#[test]
fn a_column_moves_right_in_every_row_with_its_cursor() {
    assert_eq!(
        after(
            "| h1 | h2 | h3 |\n|:---|----|---:|\n| -[a]>1 | a2 | a3 |\n",
            "GTl"
        ),
        "| h2 | h1 | h3 |\n|----|:---|---:|\n| a2 | -[a]>1 | a3 |\n"
    );
}

#[test]
fn a_column_moves_left_in_every_row_with_its_cursor() {
    assert_eq!(
        after(
            "| h1 | h2 | h3 |\n|:---|----|---:|\n| a1 | a2 | -[a]>3 |\n",
            "GTh"
        ),
        "| h1 | h3 | h2 |\n|:---|---:|----|\n| a1 | -[a]>3 | a2 |\n"
    );
}

#[test]
fn the_arrows_move_columns_as_h_and_l_do() {
    assert_eq!(
        after_keys(
            "| h1 | h2 |\n|----|----|\n| -[a]>1 | a2 |\n",
            g_t(key_right())
        ),
        "| h2 | h1 |\n|----|----|\n| a2 | -[a]>1 |\n"
    );
    assert_eq!(
        after_keys(
            "| h1 | h2 |\n|----|----|\n| a1 | -[a]>2 |\n",
            g_t(key_left())
        ),
        "| h2 | h1 |\n|----|----|\n| -[a]>2 | a1 |\n"
    );
}

#[test]
fn a_selected_column_keeps_its_selections() {
    assert_eq!(
        after(
            "| h1 | h2 | h3 |\n|:---|----|---:|\n| -[a]>1 | a2 | a3 |\n",
            "m|GTl"
        ),
        "| h2 | -[h1]> | h3 |\n|----|:---|---:|\n| a2 | -{a1}> | a3 |\n"
    );
}

#[test]
fn repeating_a_column_move_moves_every_cursor_again() {
    assert_eq!(
        after(
            "| -[h]>1 | h2 | h3 |\n|:---|----|---:|\n| -{a}>1 | a2 | a3 |\n",
            "GTl."
        ),
        "| h2 | h3 | -[h]>1 |\n|----|---:|:---|\n| a2 | a3 | -{a}>1 |\n"
    );
}

#[test]
fn a_cell_selected_with_its_pipe_keeps_the_pipe() {
    assert_eq!(
        after("| h1 | h2 |\n|----|----|\n| a1 | -[a]>2 |\n", "ma|GTh"),
        "| h2 | h1 |\n|----|----|\n-[| a2 ]>| a1 |\n"
    );
    assert_eq!(
        after(
            "| h1 | h2 | h3 |\n|:---|----|---:|\n| -[a]>1 | a2 | a3 |\n",
            "ma|GTl"
        ),
        "| h2 | h1 | h3 |\n|----|:---|---:|\n| a2 -[| a1 ]>| a3 |\n"
    );
}

#[test]
fn columns_of_rows_without_outer_pipes_move_with_their_blanks() {
    assert_eq!(
        after("aa | bb\n-- | --\n-[c]>c | dd\n", "GTl"),
        " bb|aa \n --|-- \n dd|-[c]>c \n"
    );
    assert_eq!(
        after("aa | bb\n-- | --\ncc | -[d]>d\n", "GTh"),
        " bb|aa \n --|-- \n -[d]>d|cc \n"
    );
}

#[test]
fn an_escaped_pipe_moves_inside_its_cell() {
    assert_eq!(
        after("| a\\|b | -[c]> |\n|:--|--:|\n", "GTh"),
        "| -[c]> | a\\|b |\n|--:|:--|\n"
    );
}

#[test]
fn a_selection_across_two_cells_moves_both_columns() {
    assert_eq!(
        after(
            "| h1 | h2 | h3 |\n|:---|----|---:|\n| -[a1 | a]>2 | a3 |\n",
            "GTl"
        ),
        "| h3 | h1 | h2 |\n|---:|:---|----|\n| a3 | -[a1 | a]>2 |\n"
    );
}

#[test]
fn a_column_at_the_tables_edge_stays() {
    let last = "| h1 | h2 |\n|----|----|\n| a1 | -[a]>2 |\n";
    assert_eq!(after(last, "GTl"), last);
    let first = "| h1 | h2 |\n|----|----|\n| -[a]>1 | a2 |\n";
    assert_eq!(after(first, "GTh"), first);
}

#[test]
fn a_row_short_of_a_swapped_cell_stops_the_move_with_a_warning() {
    let marked = "| h1 | h2 |\n|----|----|\n| -[a]>1 |\n| b1 | b2 |\n";
    let mut p = editing(marked);
    p.ed.feed_keys("GTl".chars().map(key));
    assert_eq!(state(&p.ed), marked);
    assert_eq!(
        warning_texts(&p.ed),
        ["markdown: moved no column, since 1 row has too few cells"]
    );
}

#[test]
fn a_selection_outside_a_table_does_not_stop_the_move() {
    assert_eq!(
        after(
            "te-[x]>t\n\n| h1 | h2 |\n|----|----|\n| -{a}>1 | a2 |\n",
            "GTl"
        ),
        "te-[x]>t\n\n| h2 | h1 |\n|----|----|\n| a2 | -{a}>1 |\n"
    );
}
