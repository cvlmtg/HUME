//! A language's comment tokens.
//!
//! Every token is non-empty, holds no line break and has no blank at either
//! edge. Commenting with such a token never changes a text's line count, and
//! the token can match text whose surrounding blanks were trimmed.

use std::fmt;

use crate::is_space_char;

/// Why a token list was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentTokenError {
    /// The list has no token.
    Empty,
    /// A token is the empty string.
    EmptyToken,
    /// A token holds a `\n` or `\r`.
    LineBreak(String),
    /// A token starts or ends with a blank.
    EdgeBlank(String),
}

impl fmt::Display for CommentTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommentTokenError::Empty => write!(f, "expected at least one token"),
            CommentTokenError::EmptyToken => write!(f, "a token is empty"),
            CommentTokenError::LineBreak(token) => {
                write!(f, "token {token:?} contains a line break")
            }
            CommentTokenError::EdgeBlank(token) => {
                write!(f, "token {token:?} starts or ends with a blank")
            }
        }
    }
}

impl std::error::Error for CommentTokenError {}

fn check_token(token: &str) -> Result<(), CommentTokenError> {
    if token.is_empty() {
        return Err(CommentTokenError::EmptyToken);
    }
    if token.contains(['\n', '\r']) {
        return Err(CommentTokenError::LineBreak(token.to_owned()));
    }
    if token.starts_with(is_space_char) || token.ends_with(is_space_char) {
        return Err(CommentTokenError::EdgeBlank(token.to_owned()));
    }
    Ok(())
}

/// The tokens that start a line comment. The first is the one written; any
/// of them marks a line as commented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCommentTokens(Vec<String>);

impl LineCommentTokens {
    pub fn new(tokens: Vec<String>) -> Result<Self, CommentTokenError> {
        if tokens.is_empty() {
            return Err(CommentTokenError::Empty);
        }
        tokens.iter().try_for_each(|t| check_token(t))?;
        Ok(Self(tokens))
    }

    /// Tokens a test spells as literals. Panics on an invalid one.
    #[cfg(any(test, feature = "test-util"))]
    pub fn from_strs(tokens: &[&str]) -> Self {
        Self::new(tokens.iter().map(|t| (*t).to_owned()).collect())
            .expect("valid line comment tokens")
    }

    /// The token written when commenting.
    pub fn primary(&self) -> &str {
        &self.0[0]
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// The start and end tokens of one block comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockCommentPair {
    start: String,
    end: String,
}

impl BlockCommentPair {
    pub fn start(&self) -> &str {
        &self.start
    }

    pub fn end(&self) -> &str {
        &self.end
    }
}

/// The block comment pairs of a language. The first is the one written; any
/// of them marks a span as commented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockCommentTokens(Vec<BlockCommentPair>);

impl BlockCommentTokens {
    pub fn new(pairs: Vec<(String, String)>) -> Result<Self, CommentTokenError> {
        if pairs.is_empty() {
            return Err(CommentTokenError::Empty);
        }
        pairs
            .into_iter()
            .map(|(start, end)| {
                check_token(&start)?;
                check_token(&end)?;
                Ok(BlockCommentPair { start, end })
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    /// Pairs a test spells as literals. Panics on an invalid one.
    #[cfg(any(test, feature = "test-util"))]
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self::new(
            pairs
                .iter()
                .map(|(start, end)| ((*start).to_owned(), (*end).to_owned()))
                .collect(),
        )
        .expect("valid block comment tokens")
    }

    /// The pair written when commenting.
    pub fn primary(&self) -> &BlockCommentPair {
        &self.0[0]
    }

    pub fn iter(&self) -> impl Iterator<Item = &BlockCommentPair> {
        self.0.iter()
    }
}

/// The comment tokens one language has, at least one kind present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentTokens<'a> {
    Line(&'a LineCommentTokens),
    Block(&'a BlockCommentTokens),
    Both(&'a LineCommentTokens, &'a BlockCommentTokens),
}

impl<'a> CommentTokens<'a> {
    /// `None` when the language has neither kind.
    pub fn new(
        line: Option<&'a LineCommentTokens>,
        block: Option<&'a BlockCommentTokens>,
    ) -> Option<Self> {
        match (line, block) {
            (Some(line), Some(block)) => Some(Self::Both(line, block)),
            (Some(line), None) => Some(Self::Line(line)),
            (None, Some(block)) => Some(Self::Block(block)),
            (None, None) => None,
        }
    }

    pub fn line(self) -> Option<&'a LineCommentTokens> {
        match self {
            Self::Line(line) | Self::Both(line, _) => Some(line),
            Self::Block(_) => None,
        }
    }

    pub fn block(self) -> Option<&'a BlockCommentTokens> {
        match self {
            Self::Block(block) | Self::Both(_, block) => Some(block),
            Self::Line(_) => None,
        }
    }
}

#[cfg(test)]
mod tests;
