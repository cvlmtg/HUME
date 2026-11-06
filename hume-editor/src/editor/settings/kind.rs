//! Parser kinds: what a `define_settings!` entry's `parser:` names.
//!
//! A kind is a zero-sized type implementing [`Kind`] for its value type: how
//! a value parses from `:set` text and from the value a script passes, which
//! values `:set` completion offers, and the [`Shape`] the value reads back
//! as. [`Kind::Script`] is the type a script writes the value in; [`parse_as`]
//! refuses a script value of any other type before reading what it holds.

use std::fmt;

use hume_core::{OptionInput, OptionType, OptionValue, StatuslineSections};
use hume_platform::process::batch::ShellCommand;

use super::{SettingText, parse_named};
use crate::statusline::{StatusElement, StatusLineConfig};

/// A setting's value as it reads back: as the [`OptionValue`] a script gets,
/// or as the text `:set` takes.
pub(in crate::editor) enum Shape {
    Bool(bool),
    Int(usize),
    /// A value already in the text its parser takes back.
    Text(String),
    Shell(ShellCommand),
    Statusline(StatusLineConfig),
}

impl Shape {
    /// The value as `get-option` reads it: `shell` as its words, the
    /// statusline as its sections.
    pub(in crate::editor) fn value(self) -> OptionValue {
        match self {
            Self::Bool(value) => OptionValue::Bool(value),
            Self::Int(value) => OptionValue::Int(value as i64),
            Self::Text(text) => OptionValue::Str(text),
            Self::Shell(shell) => OptionValue::List(shell.words()),
            Self::Statusline(cfg) => OptionValue::Sections(statusline_sections(&cfg)),
        }
    }

    /// The value as the text `:set` takes back.
    pub(in crate::editor) fn text(self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Text(text) => text,
            Self::Shell(shell) => shell.to_text(),
            Self::Statusline(cfg) => {
                let sections = statusline_sections(&cfg);
                [sections.left, sections.center, sections.right]
                    .map(|names| names.join(","))
                    .join("|")
            }
        }
    }
}

/// What a script value of one [`OptionType`] holds.
pub(super) trait Payload {
    const TYPE: OptionType;

    /// `value`'s payload, `None` for a value of another type.
    fn take(value: &OptionValue) -> Option<&Self>;
}

/// Implement [`Payload`] for the type each [`OptionValue`] variant holds.
macro_rules! payloads {
    ($($ty:ty => $variant:ident),+ $(,)?) => {
        $(
            impl Payload for $ty {
                const TYPE: OptionType = OptionType::$variant;

                fn take(value: &OptionValue) -> Option<&Self> {
                    match value {
                        OptionValue::$variant(payload) => Some(payload),
                        _ => None,
                    }
                }
            }
        )+
    };
}

payloads! {
    bool => Bool,
    i64 => Int,
    String => Str,
    Vec<String> => List,
    StatuslineSections => Sections,
}

/// How a setting whose value is a `T` is parsed, completed and read back.
pub(super) trait Kind<T> {
    /// What a script writes the value as.
    type Script: Payload;

    /// The values `:set <key>=<Tab>` offers; empty for free text.
    const NAMES: &'static [&'static str];

    /// The value `:set` text gives. The error does not name the setting.
    fn parse_text(text: &str) -> Result<T, String>;

    /// The value a script's value gives. The error does not name the setting.
    fn parse_script(value: &Self::Script) -> Result<T, String>;

    fn shape(value: &T) -> Shape;
}

/// Parse `input` for the setting `key`, whose kind is `K`. Every setting
/// write parses through here, so every parse error names its setting and the
/// kinds' parsers stay key-free. A script value of another type than `K`
/// takes is refused naming both types, before a list's or hash's contents
/// are read.
pub(super) fn parse_as<K: Kind<T>, T>(key: &str, input: &OptionInput) -> Result<T, String> {
    let takes = <K::Script as Payload>::TYPE;
    let expected = |given: &str| format!("expected {}, got {given}", takes.name());
    let parsed = match input {
        OptionInput::Text(text) => K::parse_text(text),
        OptionInput::Value(value) => K::Script::take(value).map_or_else(
            || Err(expected(value.option_type().name())),
            K::parse_script,
        ),
        OptionInput::Unreadable { ty, error } if *ty == takes => Err(error.clone()),
        OptionInput::Unreadable { ty, .. } => Err(expected(ty.name())),
        OptionInput::Foreign(given) => Err(expected(given)),
    };
    parsed.map_err(|e| format!("invalid value for '{key}': {e}"))
}

pub(super) struct Bool;

impl Kind<bool> for Bool {
    type Script = bool;
    const NAMES: &'static [&'static str] = &["true", "false"];

    fn parse_text(text: &str) -> Result<bool, String> {
        match text {
            "true" | "on" | "yes" | "1" => Ok(true),
            "false" | "off" | "no" | "0" => Ok(false),
            _ => Err(format!("expected true/false, got '{text}'")),
        }
    }

    fn parse_script(value: &bool) -> Result<bool, String> {
        Ok(*value)
    }

    fn shape(value: &bool) -> Shape {
        Shape::Bool(*value)
    }
}

pub(super) struct Usize;

impl Kind<usize> for Usize {
    type Script = i64;
    const NAMES: &'static [&'static str] = &[];

    fn parse_text(text: &str) -> Result<usize, String> {
        parse_usize(text)
    }

    fn parse_script(value: &i64) -> Result<usize, String> {
        script_count(*value)
    }

    fn shape(value: &usize) -> Shape {
        Shape::Int(*value)
    }
}

/// A count where 0 is refused.
pub(super) struct UsizeNonzero;

impl Kind<usize> for UsizeNonzero {
    type Script = i64;
    const NAMES: &'static [&'static str] = &[];

    fn parse_text(text: &str) -> Result<usize, String> {
        parse_usize(text).and_then(at_least_one)
    }

    fn parse_script(value: &i64) -> Result<usize, String> {
        script_count(*value).and_then(at_least_one)
    }

    fn shape(value: &usize) -> Shape {
        Shape::Int(*value)
    }
}

/// A width of 1 to 255 columns.
pub(super) struct TabWidth;

impl Kind<u8> for TabWidth {
    type Script = i64;
    const NAMES: &'static [&'static str] = &[];

    fn parse_text(text: &str) -> Result<u8, String> {
        text.parse::<u8>()
            .map_err(|_| not_a_tab_width(text))
            .and_then(at_least_one)
    }

    fn parse_script(value: &i64) -> Result<u8, String> {
        u8::try_from(*value)
            .map_err(|_| not_a_tab_width(value))
            .and_then(at_least_one)
    }

    fn shape(value: &u8) -> Shape {
        Shape::Int(usize::from(*value))
    }
}

/// A value whose type gives its own text form through [`SettingText`].
pub(super) struct Typed;

impl<T: SettingText> Kind<T> for Typed {
    type Script = String;
    const NAMES: &'static [&'static str] = T::VALUES;

    fn parse_text(text: &str) -> Result<T, String> {
        T::parse(text)
    }

    fn parse_script(value: &String) -> Result<T, String> {
        T::parse(value)
    }

    fn shape(value: &T) -> Shape {
        Shape::Text(value.to_text())
    }
}

/// Any text, taken as given.
pub(super) struct FreeText;

impl Kind<String> for FreeText {
    type Script = String;
    const NAMES: &'static [&'static str] = &[];

    fn parse_text(text: &str) -> Result<String, String> {
        Ok(text.to_owned())
    }

    fn parse_script(value: &String) -> Result<String, String> {
        Ok(value.clone())
    }

    fn shape(value: &String) -> Shape {
        Shape::Text(value.clone())
    }
}

/// Whether newlines render, written `none` or `all`. Unlike spaces and
/// tabs, a newline is always at the end of its line, so there is no
/// `trailing` value.
pub(super) struct ShowNewline;

impl Kind<bool> for ShowNewline {
    type Script = String;
    const NAMES: &'static [&'static str] = &["none", "all"];

    fn parse_text(text: &str) -> Result<bool, String> {
        parse_named(text, Self::NAMES, [false, true])
    }

    fn parse_script(value: &String) -> Result<bool, String> {
        Self::parse_text(value)
    }

    fn shape(value: &bool) -> Shape {
        Shape::Text(Self::NAMES[usize::from(*value)].to_owned())
    }
}

/// Extra word characters, validated unlike [`FreeText`]: whitespace is
/// refused, since a word run must end at whitespace.
pub(super) struct WordChars;

impl Kind<String> for WordChars {
    type Script = String;
    const NAMES: &'static [&'static str] = &[];

    fn parse_text(text: &str) -> Result<String, String> {
        hume_editing::word::WordChars::validate(text)?;
        Ok(text.to_owned())
    }

    fn parse_script(value: &String) -> Result<String, String> {
        Self::parse_text(value)
    }

    fn shape(value: &String) -> Shape {
        Shape::Text(value.clone())
    }
}

/// A command line: `:set`'s text, or a script's list of words.
pub(super) struct Shell;

impl Kind<ShellCommand> for Shell {
    type Script = Vec<String>;
    const NAMES: &'static [&'static str] = &[];

    fn parse_text(text: &str) -> Result<ShellCommand, String> {
        ShellCommand::parse(text)
    }

    fn parse_script(value: &Vec<String>) -> Result<ShellCommand, String> {
        ShellCommand::from_words(value.clone())
    }

    fn shape(value: &ShellCommand) -> Shape {
        Shape::Shell(value.clone())
    }
}

/// The statusline's sections: `:set`'s text, or a script's sections.
pub(super) struct Statusline;

impl Kind<StatusLineConfig> for Statusline {
    type Script = StatuslineSections;
    const NAMES: &'static [&'static str] = &[];

    /// Three `|`-separated sections, no more, each a comma-separated list of
    /// element names.
    fn parse_text(text: &str) -> Result<StatusLineConfig, String> {
        let names = |section: &str| -> Vec<String> {
            if section.is_empty() {
                Vec::new()
            } else {
                section.split(',').map(str::to_string).collect()
            }
        };
        let parts: Vec<&str> = text.splitn(4, '|').collect();
        let [left, center, right] = parts.as_slice() else {
            return Err(format!(
                "expected three sections separated by '|' \
                 (e.g. 'Mode,FileName||Position'), got '{text}'"
            ));
        };
        parse_statusline(&StatuslineSections {
            left: names(left),
            center: names(center),
            right: names(right),
        })
    }

    fn parse_script(value: &StatuslineSections) -> Result<StatusLineConfig, String> {
        parse_statusline(value)
    }

    fn shape(value: &StatusLineConfig) -> Shape {
        Shape::Statusline(value.clone())
    }
}

// ── Value parsers ─────────────────────────────────────────────────────────────

/// The error for a count that is not a non-negative integer.
fn not_a_count(value: impl fmt::Display) -> String {
    format!("expected a non-negative integer, got '{value}'")
}

fn parse_usize(value: &str) -> Result<usize, String> {
    value.parse::<usize>().map_err(|_| not_a_count(value))
}

/// A script's integer as a count, held to [`parse_usize`]'s rule.
fn script_count(value: i64) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| not_a_count(value))
}

/// `n`, refused when it is 0.
fn at_least_one<T: PartialEq + Default>(n: T) -> Result<T, String> {
    if n == T::default() {
        return Err("must be at least 1".into());
    }
    Ok(n)
}

/// The error for a tab width outside `u8`.
fn not_a_tab_width(value: impl fmt::Display) -> String {
    format!("expected 1–255, got '{value}'")
}

/// The statusline `sections` name. An empty section is allowed, an empty
/// name inside one (`Mode,,FileName`) is not.
fn parse_statusline(sections: &StatuslineSections) -> Result<StatusLineConfig, String> {
    let elements = |names: &[String]| -> Result<Vec<StatusElement>, String> {
        names
            .iter()
            .map(|name| {
                if name.is_empty() {
                    return Err(format!(
                        "section '{}' has an empty element name",
                        names.join(",")
                    ));
                }
                name.parse::<StatusElement>()
            })
            .collect()
    };
    Ok(StatusLineConfig {
        left: elements(&sections.left)?,
        center: elements(&sections.center)?,
        right: elements(&sections.right)?,
    })
}

/// The element names of each of `cfg`'s sections, as `get-option` reads them:
/// [`parse_statusline`]'s inverse.
fn statusline_sections(cfg: &StatusLineConfig) -> StatuslineSections {
    let names = |elems: &[StatusElement]| elems.iter().map(ToString::to_string).collect();
    StatuslineSections {
        left: names(&cfg.left),
        center: names(&cfg.center),
        right: names(&cfg.right),
    }
}
