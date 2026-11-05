//! Which buffers a plugin entry's code acts on.
//!
//! An entry declared with `#:languages` acts only on buffers of those
//! languages: every other buffer looks closed to it. Its hooks never fire
//! for one, its buffer completion sources are never called for one, and
//! `(buffers)` and `(panes)` leave it out. A language change that moves a
//! buffer out of an entry's languages reaches the entry as that buffer's
//! close, and one that moves it in as its language set then its open.

use std::rc::Rc;

/// The buffers a piece of code sees, resolved from its owner by
/// `LazyRegistry::visibility_of`.
///
/// Resolving it once, when a hook or a source is registered, is sound
/// because an entry's languages are fixed by its declaration, which comes
/// before any of its code runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BufferVisibility(Option<Rc<[String]>>);

impl BufferVisibility {
    /// The buffers an entry declared with `languages` acts on: only those
    /// of a listed language, never one with no language, unless the list
    /// does not [limit](Self::limits) it.
    pub(crate) fn declared(languages: Rc<[String]>) -> Self {
        if Self::limits(&languages) {
            Self(Some(languages))
        } else {
            Self::default()
        }
    }

    /// Whether a `#:languages` list limits the buffers its entry acts on:
    /// it does unless it is empty or names the wildcard `"*"`.
    pub(crate) fn limits(languages: &[String]) -> bool {
        !languages.is_empty() && !languages.iter().any(|lang| lang == "*")
    }

    /// Whether a buffer whose language is `language` (`None`: it has none)
    /// is visible.
    pub fn sees(&self, language: Option<&str>) -> bool {
        match &self.0 {
            None => true,
            Some(languages) => language.is_some_and(|lang| languages.iter().any(|l| l == lang)),
        }
    }
}

/// Which handlers of a hook one delivery reaches, judged by each handler's
/// [`BufferVisibility`].
#[derive(Debug, Clone, Copy)]
pub enum Audience<'a> {
    /// An event about no buffer: every handler.
    Everyone,
    /// An event about a buffer of this language: the handlers that see it.
    Seeing(Option<&'a str>),
    /// The handlers that see a buffer of language `sees` but not one of
    /// `misses`. A language change delivers its open to those that see only
    /// its new language, and its close to those that see only its old one.
    Crossing {
        sees: Option<&'a str>,
        misses: Option<&'a str>,
    },
}

impl Audience<'_> {
    pub(crate) fn admits(self, visibility: &BufferVisibility) -> bool {
        match self {
            Self::Everyone => true,
            Self::Seeing(language) => visibility.sees(language),
            Self::Crossing { sees, misses } => visibility.sees(sees) && !visibility.sees(misses),
        }
    }
}

#[cfg(test)]
mod tests;
