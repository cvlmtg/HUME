//! The option vocabulary `:set` and scripts share: the options a buffer
//! stores itself, the value an option is set to, and the value a script
//! reads back.

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

/// A value an option is set to: the text `:set` carries, or what a script
/// passes. The option's parser reads either, so a script value of another
/// type than the option takes is refused there, as a bad text value is, and
/// only once the option is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionInput {
    Text(String),
    Value(OptionValue),
    /// A list or hash whose contents did not decode. An option that takes
    /// `ty` is refused with `error`; any other names `ty` as the type given.
    Unreadable {
        ty: OptionType,
        error: String,
    },
    /// A script value of no option type (a symbol, void, …), as an error
    /// names it.
    Foreign(String),
}

/// An option's value as scripts write and read it. `Str` is every named
/// value, free text and `name:N` forms alike, in the spelling `:set` takes;
/// `List` is a word list (`shell`); `Sections` is the `statusline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<String>),
    Sections(StatuslineSections),
}

impl OptionValue {
    pub fn option_type(&self) -> OptionType {
        match self {
            Self::Bool(_) => OptionType::Bool,
            Self::Int(_) => OptionType::Int,
            Self::Str(_) => OptionType::Str,
            Self::List(_) => OptionType::List,
            Self::Sections(_) => OptionType::Sections,
        }
    }
}

/// Which [`OptionValue`] variant a value is, or an option takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionType {
    Bool,
    Int,
    Str,
    List,
    Sections,
}

impl OptionType {
    /// Every type, in [`OptionValue`]'s order.
    pub const ALL: [Self; 5] = [Self::Bool, Self::Int, Self::Str, Self::List, Self::Sections];

    /// The type as an error names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bool => "a boolean",
            Self::Int => "an integer",
            Self::Str => "a string",
            Self::List => "a list of strings",
            Self::Sections => "a statusline sections hash",
        }
    }
}

/// The `statusline` option as scripts write and read it: the element names
/// of its left, center and right sections, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatuslineSections {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}
