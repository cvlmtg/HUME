//! The option vocabulary `:set` and scripts share: the options a buffer
//! stores itself, and the value an option is set to.

named_enum! {
    /// A buffer option stored on the buffer itself rather than in the settings
    /// tables. Each has no global default and is valid only for `:set buffer`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BufferIntrinsicOption {
        /// The buffer's language, by name ([`language_option_value`]).
        Language => "language",
        /// The line ending a save writes (`lf` or `crlf`).
        LineEnding => "line-ending",
    }
}

impl BufferIntrinsicOption {
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|opt| opt.name() == name)
    }
}

/// A `language` option value as a language name: `""` is the spelling of
/// "no language", so it decodes to `None`.
pub fn language_option_value(value: &str) -> Option<&str> {
    Some(value).filter(|name| !name.is_empty())
}

/// A value an option is set to, from `:set` or a script. A list stays a
/// list, so only a word-list option (`shell`) accepts one; every other value
/// is the text `:set` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionInput {
    Text(String),
    Words(Vec<String>),
}
