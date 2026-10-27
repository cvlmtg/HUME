//! The option vocabulary `:set` and scripts share: the options a buffer
//! stores itself, and the value an option is set to.

named_enum! {
    /// A buffer option stored on the buffer itself rather than in the settings
    /// tables. Each has no global default and is valid only for `:set buffer`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BufferIntrinsicOption {
        /// The buffer's language, by name ([`LanguageOptionText`]).
        Language => "language",
        /// The line ending a save writes (`lf` or `crlf`).
        LineEnding => "line-ending",
    }
}

/// The `language` option's text: a language name, with `""` spelling no
/// language. `:set` and scripts read and write the value through this pair.
pub struct LanguageOptionText;

impl LanguageOptionText {
    const NONE: &'static str = "";

    pub fn decode(text: &str) -> Option<&str> {
        Some(text).filter(|name| *name != Self::NONE)
    }

    pub fn encode(name: Option<&str>) -> &str {
        name.unwrap_or(Self::NONE)
    }
}

/// A value an option is set to, from `:set` or a script. A list stays a
/// list, so only a word-list option (`shell`) accepts one; every other value
/// is the text `:set` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionInput {
    Text(String),
    Words(Vec<String>),
}
