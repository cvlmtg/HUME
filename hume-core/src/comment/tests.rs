use super::*;

fn strings(tokens: &[&str]) -> Vec<String> {
    tokens.iter().map(|t| (*t).to_owned()).collect()
}

fn pairs(tokens: &[(&str, &str)]) -> Vec<(String, String)> {
    tokens
        .iter()
        .map(|(s, e)| ((*s).to_owned(), (*e).to_owned()))
        .collect()
}

#[test]
fn line_tokens_reject_an_empty_list() {
    assert_eq!(
        LineCommentTokens::new(vec![]),
        Err(CommentTokenError::Empty)
    );
}

#[test]
fn block_tokens_reject_an_empty_list() {
    assert_eq!(
        BlockCommentTokens::new(vec![]),
        Err(CommentTokenError::Empty)
    );
}

#[test]
fn line_tokens_reject_an_empty_token() {
    assert_eq!(
        LineCommentTokens::new(strings(&["//", ""])),
        Err(CommentTokenError::EmptyToken)
    );
}

#[test]
fn block_tokens_reject_an_empty_end() {
    assert_eq!(
        BlockCommentTokens::new(pairs(&[("/*", "")])),
        Err(CommentTokenError::EmptyToken)
    );
}

#[test]
fn tokens_reject_a_line_break() {
    assert_eq!(
        LineCommentTokens::new(strings(&["/\n/"])),
        Err(CommentTokenError::LineBreak("/\n/".to_owned()))
    );
    assert_eq!(
        BlockCommentTokens::new(pairs(&[("/*", "*\r/")])),
        Err(CommentTokenError::LineBreak("*\r/".to_owned()))
    );
}

#[test]
fn tokens_reject_a_blank_edge() {
    assert_eq!(
        LineCommentTokens::new(strings(&[" //"])),
        Err(CommentTokenError::EdgeBlank(" //".to_owned()))
    );
    assert_eq!(
        BlockCommentTokens::new(pairs(&[("/*\t", "*/")])),
        Err(CommentTokenError::EdgeBlank("/*\t".to_owned()))
    );
    assert_eq!(
        LineCommentTokens::new(strings(&["#\u{a0}"])),
        Err(CommentTokenError::EdgeBlank("#\u{a0}".to_owned()))
    );
}

#[test]
fn a_blank_inside_a_token_is_accepted() {
    let tokens = LineCommentTokens::new(strings(&["- -"])).expect("valid");
    assert_eq!(tokens.primary(), "- -");
}

#[test]
fn line_tokens_keep_their_order_and_the_first_is_primary() {
    let tokens = LineCommentTokens::new(strings(&["//", "///", "//!"])).expect("valid");
    assert_eq!(tokens.primary(), "//");
    assert_eq!(tokens.iter().collect::<Vec<_>>(), ["//", "///", "//!"]);
}

#[test]
fn block_pairs_keep_their_order_and_the_first_is_primary() {
    let tokens = BlockCommentTokens::new(pairs(&[("/*", "*/"), ("/**", "*/")])).expect("valid");
    assert_eq!(
        (tokens.primary().start(), tokens.primary().end()),
        ("/*", "*/")
    );
    let all: Vec<_> = tokens.iter().map(|p| (p.start(), p.end())).collect();
    assert_eq!(all, [("/*", "*/"), ("/**", "*/")]);
}

#[test]
fn comment_tokens_need_at_least_one_kind() {
    let line = LineCommentTokens::new(strings(&["#"])).expect("valid");
    let block = BlockCommentTokens::new(pairs(&[("<!--", "-->")])).expect("valid");
    assert_eq!(CommentTokens::new(None, None), None);
    assert_eq!(
        CommentTokens::new(Some(&line), None),
        Some(CommentTokens::Line(&line))
    );
    assert_eq!(
        CommentTokens::new(None, Some(&block)),
        Some(CommentTokens::Block(&block))
    );
    assert_eq!(
        CommentTokens::new(Some(&line), Some(&block)),
        Some(CommentTokens::Both(&line, &block))
    );
}
