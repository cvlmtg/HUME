//! `m i |` selects the table cell under each selection, `m |` the column of
//! the primary selection's cell.

use super::*;

/// The state after `keys` in Normal mode.
fn after(marked: &str, keys: &[char]) -> String {
    let mut p = editing(marked);
    p.ed.feed_keys(keys.iter().map(|&c| key(c)));
    state(&p.ed)
}

fn cell(marked: &str) -> String {
    after(marked, &['m', 'i', '|'])
}

fn column(marked: &str) -> String {
    after(marked, &['m', '|'])
}

#[test]
fn a_cell_between_outer_pipes() {
    assert_eq!(
        cell("| aa | bb |\n|----|----|\n| cc | d-[d]> |\n"),
        "| aa | bb |\n|----|----|\n| cc | -[dd]> |\n"
    );
}

#[test]
fn a_blank_before_a_cell_belongs_to_that_cell() {
    assert_eq!(
        cell("| aa |-[ ]>bb |\n|----|----|\n"),
        "| aa | -[bb]> |\n|----|----|\n"
    );
}

#[test]
fn a_pipe_belongs_to_the_cell_before_it() {
    assert_eq!(
        cell("| aa -[|]> bb |\n|----|----|\n"),
        "| -[aa]> | bb |\n|----|----|\n"
    );
}

#[test]
fn the_leading_pipe_belongs_to_the_first_cell() {
    assert_eq!(
        cell("-[|]> aa | bb |\n|----|----|\n"),
        "| -[aa]> | bb |\n|----|----|\n"
    );
}

#[test]
fn the_first_and_last_cell_of_a_row_without_outer_pipes() {
    assert_eq!(
        cell("aa | bb\n-- | --\n-[c]>c | dd\n"),
        "aa | bb\n-- | --\n-[cc]> | dd\n"
    );
    assert_eq!(
        cell("aa | bb\n-- | --\ncc | -[d]>d\n"),
        "aa | bb\n-- | --\ncc | -[dd]>\n"
    );
}

#[test]
fn an_escaped_pipe_stays_inside_its_cell() {
    assert_eq!(
        cell("| a\\|-[b]> | c |\n|---|---|\n"),
        "| -[a\\|b]> | c |\n|---|---|\n"
    );
}

#[test]
fn a_blank_cell_gives_a_cursor_in_it() {
    assert_eq!(
        cell("| a | -[ ]>  | c |\n|---|---|---|\n"),
        "| a |   -[ ]>| c |\n|---|---|---|\n"
    );
}

#[test]
fn a_cell_of_the_delimiter_row() {
    assert_eq!(
        cell("| aa | bb |\n|----|-[-]>---|\n"),
        "| aa | bb |\n|----|-[----]>|\n"
    );
}

#[test]
fn a_cell_of_a_quoted_table() {
    assert_eq!(
        cell("> | aa | bb |\n> |----|----|\n> | cc | -[d]>d |\n"),
        "> | aa | bb |\n> |----|----|\n> | cc | -[dd]> |\n"
    );
}

#[test]
fn outside_a_table_nothing_changes() {
    let marked = "te-[x]>t\n\n| a |\n|---|\n";
    assert_eq!(cell(marked), marked);
}

#[test]
fn every_selection_gets_its_own_cell() {
    assert_eq!(
        cell("| -[a]>a | bb |\n|----|----|\n| cc | d-{d}> |\n"),
        "| -[aa]> | bb |\n|----|----|\n| cc | -{dd}> |\n"
    );
}

#[test]
fn a_column_holds_the_header_and_the_body_rows() {
    assert_eq!(
        column("| aa | bb |\n|----|----|\n| -[c]>c | dd |\n| ee | ff |\n"),
        "| -[aa]> | bb |\n|----|----|\n| -{cc}> | dd |\n| -[ee]> | ff |\n"
    );
}

#[test]
fn the_last_column_of_rows_without_outer_pipes() {
    assert_eq!(
        column("aa | -[b]>b\n-- | --\ncc | dd\n"),
        "aa | -{bb}>\n-- | --\ncc | -[dd]>\n"
    );
}

#[test]
fn a_column_with_an_escaped_pipe_in_a_cell() {
    assert_eq!(
        column("| a\\|b | -[c]> |\n|---|---|\n| d | e |\n"),
        "| a\\|b | -{c}> |\n|---|---|\n| d | -[e]> |\n"
    );
    assert_eq!(
        column("| a\\|-[b]> | c |\n|---|---|\n| d | e |\n"),
        "| -{a\\|b}> | c |\n|---|---|\n| -[d]> | e |\n"
    );
}

#[test]
fn a_blank_cell_of_a_column_gives_a_cursor() {
    assert_eq!(
        column("| aa | -[b]>b |\n|----|----|\n| cc |    |\n"),
        "| aa | -{bb}> |\n|----|----|\n| cc |   -[ ]>|\n"
    );
}

#[test]
fn a_column_of_a_quoted_table() {
    assert_eq!(
        column("> | a | b |\n> |---|---|\n> | c | -[d]> |\n"),
        "> | a | -[b]> |\n> |---|---|\n> | c | -{d}> |\n"
    );
}

#[test]
fn a_row_with_no_cell_in_the_column_is_skipped_with_a_warning() {
    let mut p = editing("| a | -[b]>b |\n|---|----|\n| c |\n");
    p.ed.feed_keys([key('m'), key('|')]);
    assert_eq!(state(&p.ed), "| a | -[bb]> |\n|---|----|\n| c |\n");
    assert_eq!(
        warning_log(&p.ed),
        ["markdown: skipped 1 row with no selectable cell in this column"]
    );
}

#[test]
fn an_empty_cell_is_skipped_with_a_warning() {
    let mut p = editing("| a | -[b]>b | c |\n|---|----|---|\n|*e*||&amp;|\n");
    p.ed.feed_keys([key('m'), key('|')]);
    assert_eq!(
        state(&p.ed),
        "| a | -[bb]> | c |\n|---|----|---|\n|*e*||&amp;|\n"
    );
    assert_eq!(
        warning_log(&p.ed),
        ["markdown: skipped 1 row with no selectable cell in this column"]
    );
}

#[test]
fn a_column_of_a_wide_table_under_soft_wrap() {
    let mut p = preview().sized(20, 12);
    p.open_marked(
        "a.md",
        "| a long first cell | a long second cell |\n|---|---|\n| x | -[y]> |\n",
    );
    pin_wrap(&mut p.ed, WrapMode::Soft { width: 0 });
    p.ed.feed_keys([key('m'), key('|')]);
    assert_eq!(
        state(&p.ed),
        "| a long first cell | -[a long second cell]> |\n|---|---|\n| x | -{y}> |\n"
    );
}

#[test]
fn one_selection_undo_restores_the_selections_before_a_column() {
    let mut p = editing("| aa | bb |\n|----|----|\n| -[c]>c | dd |\n");
    p.ed.feed_keys([key('m'), key('i'), key('w')]);
    p.ed.feed_keys([key('m'), key('|'), key('m'), key('u')]);
    assert_eq!(state(&p.ed), "| aa | bb |\n|----|----|\n| -[cc]> | dd |\n");
}

fn around(marked: &str) -> String {
    after(marked, &['m', 'a', '|'])
}

#[test]
fn around_a_cell_takes_the_pipe_before_it_and_its_blanks() {
    assert_eq!(
        around("| aa | b-[b]> | cc |\n|----|----|----|\n| x | y | z |\n"),
        "| aa -[| bb ]>| cc |\n|----|----|----|\n| x | y | z |\n"
    );
    assert_eq!(
        around("| a-[a]> | bb |\n|----|----|\n| x | y |\n"),
        "-[| aa ]>| bb |\n|----|----|\n| x | y |\n"
    );
    assert_eq!(
        around("| aa | b-[b]> |\n|----|----|\n| x | y |\n"),
        "| aa -[| bb ]>|\n|----|----|\n| x | y |\n"
    );
}

#[test]
fn around_a_cell_of_a_row_without_outer_pipes() {
    assert_eq!(
        around("a-[a]> | bb\n-- | --\nx | y\n"),
        "-[aa ]>| bb\n-- | --\nx | y\n"
    );
    assert_eq!(
        around("aa | b-[b]>\n-- | --\nx | y\n"),
        "aa -[| bb]>\n-- | --\nx | y\n"
    );
}

#[test]
fn around_a_blank_cell() {
    assert_eq!(
        around("| a | -[ ]>  | c |\n|---|---|---|\n| x | y | z |\n"),
        "| a -[|    ]>| c |\n|---|---|---|\n| x | y | z |\n"
    );
}

#[test]
fn around_the_header_cell_of_a_table_without_body_rows_takes_its_delimiter_cell() {
    assert_eq!(
        around("| aa | b-[b]> |\n|----|----|\n"),
        "| aa -{| bb }>|\n|-----[|----]>|\n"
    );
}

#[test]
fn around_a_whole_column_takes_its_delimiter_cell_so_d_deletes_the_column() {
    let mut p = editing("| aa | b-[b]> |\n|----|----|\n| cc | dd |\n");
    p.ed.feed_keys("m|ma|d".chars().map(key));
    assert_eq!(state(&p.ed), "| aa -{|}>\n|-----[|]>\n| cc -[|]>\n");
}

#[test]
fn deleting_the_first_column_of_a_table_without_outer_pipes() {
    let mut p = editing("a-[a]> | bb\n-- | --\ncc | dd\n");
    p.ed.feed_keys("m|ma|d".chars().map(key));
    assert_eq!(state(&p.ed), "-{|}> bb\n-[|]> --\n-[|]> dd\n");
}

#[test]
fn the_first_cell_of_a_quoted_row_without_pipes_leaves_the_marker_out() {
    assert_eq!(
        cell("> aa | bb\n> -- | --\n> -[c]>c | dd\n"),
        "> aa | bb\n> -- | --\n> -[cc]> | dd\n"
    );
}

#[test]
fn the_first_cell_of_a_list_items_row_leaves_the_marker_out() {
    assert_eq!(
        cell("- a-[a]> | bb\n  -- | --\n  cc | dd\n"),
        "- -[aa]> | bb\n  -- | --\n  cc | dd\n"
    );
}

#[test]
fn deleting_the_first_column_of_a_quoted_table_keeps_the_quote() {
    let mut p = editing("> a-[a]> | bb\n> -- | --\n> cc | dd\n");
    p.ed.feed_keys("m|ma|d".chars().map(key));
    assert_eq!(state(&p.ed), "> -{|}> bb\n> -[|]> --\n> -[|]> dd\n");
}

#[test]
fn a_backslash_ending_a_row_stays_in_its_cell() {
    assert_eq!(
        column("aa | bb\n-- | --\ncc | -[d]>\\\n"),
        "aa | -[bb]>\n-- | --\ncc | -{d\\}>\n"
    );
}

#[test]
fn two_adjacent_pipes_hold_an_empty_cell_in_every_row() {
    let marked = "| a | b | c |\n|---|---|---|\n| c |-[|]> d |\n";
    assert_eq!(cell(marked), marked);
}

#[test]
fn deleting_a_column_takes_the_pipe_of_its_empty_cells() {
    let mut p = editing("| a | -[b]>b | c |\n|---|----|---|\n|*e*||&amp;|\n");
    p.ed.feed_keys("m|ma|d".chars().map(key));
    assert_eq!(
        state(&p.ed),
        "| a -{|}> c |\n|----[|]>---|\n|*e*-[|]>&amp;|\n"
    );
}

#[test]
fn lines_with_pipes_but_no_delimiter_row_are_not_a_table() {
    let marked = "x-[a]> | b\nc | d\n";
    assert_eq!(cell(marked), marked);
}

#[test]
fn selecting_cells_records_no_jump() {
    let mut p = editing("x\n\n| aa | bb |\n|----|----|\n| -[c]>c | -{d}>d |\n");
    let jumps = |ed: &Editor| ed.state.panes.entries[ed.view.focused_pane()].jumps.len();
    let before = jumps(&p.ed);
    p.ed.feed_keys("mi|".chars().map(key));
    assert_eq!(jumps(&p.ed), before);
}

#[test]
fn selecting_a_cell_on_screen_keeps_the_view_where_it_is() {
    let mut text = String::from("-[l]>ine\n");
    text.push_str(&"line\n".repeat(61));
    text.push_str("| aa | bb |\n|----|----|\n| cc | dd |\n");
    text.push_str(&"tail\n".repeat(60));
    let mut p = editing(&text);
    p.ed.feed_keys("64j".chars().map(key));
    p.render();
    let top = p.ed.viewport().top();
    p.ed.feed_keys("mi|".chars().map(key));
    p.render();
    assert_eq!(p.ed.viewport().top(), top);
}
