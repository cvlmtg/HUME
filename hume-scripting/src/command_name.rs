//! The rules for command names: what the `:` line reads as a typed
//! command's name, which names each kind of command may be registered
//! under, and the case folding every name lookup goes through.
//!
//! Command names ignore ASCII case. An editor command's name is lowercase,
//! so it is its own folded form; a typed command keeps the spelling it was
//! defined with and is found by its folded form.
//!
//! A typed name starts with a letter, `-` or `_` and continues with letters,
//! digits, `-` or `_`. One exception keeps `:b1` meaning `:b 1`: `b` (any
//! case) followed only by digits reads as the name `b`, the digits being its
//! argument. A digit-led line (`:42`) reads as an empty name, the line
//! number being the argument.

use std::borrow::Cow;

use rustc_hash::FxHashSet;

/// The typed command name at the start of `input`, possibly empty.
pub fn scan(input: &str) -> &str {
    let mut chars = input.char_indices();
    let end = match chars.next() {
        Some((_, first)) if first.is_ascii_alphabetic() || first == '-' || first == '_' => chars
            .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .map_or(input.len(), |(i, _)| i),
        _ => 0,
    };
    let name = &input[..end];
    if is_buffer_number(name) {
        &name[..1]
    } else {
        name
    }
}

/// `Ok` when the `:` line reads `name` back whole as a name, so a typed
/// command registered under it can be run; otherwise the reason it can't,
/// naming `name`.
pub fn check_typed(name: &str) -> Result<(), String> {
    if !name.is_empty() && scan(name) == name {
        Ok(())
    } else if is_buffer_number(name) {
        Err(format!(
            "'{name}' is not a valid typed command name: `:b` followed by a number switches buffer"
        ))
    } else {
        Err(format!(
            "'{name}' is not a valid typed command name: it must start with a letter, '-' or '_' \
             and continue with letters, digits, '-' or '_'"
        ))
    }
}

/// `Ok` when an editor command may be registered under `name`; otherwise
/// the reason it can't, naming `name`.
pub fn check_mappable(name: &str) -> Result<(), String> {
    check_quotable(name)?;
    if is_folded(name) {
        Ok(())
    } else {
        Err(format!(
            "'{name}' is not a valid editor command name: it must be lowercase"
        ))
    }
}

/// `Ok` when `name` contains neither `"` nor `\`, so it survives being
/// quoted into Steel source; otherwise the reason it doesn't, naming `name`.
/// [`check_typed`]'s grammar already excludes both.
pub(crate) fn check_quotable(name: &str) -> Result<(), String> {
    if name.contains(['"', '\\']) {
        Err(format!(
            "command name '{name}' must not contain '\"' or '\\'"
        ))
    } else {
        Ok(())
    }
}

/// The form `name` is stored and looked up under. Borrows `name` when it is
/// already folded, as every editor command name is.
pub fn fold(name: &str) -> Cow<'_, str> {
    if is_folded(name) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.to_ascii_lowercase())
    }
}

fn is_folded(name: &str) -> bool {
    !name.bytes().any(|b| b.is_ascii_uppercase())
}

/// A set of command names that answers membership in any case.
#[derive(Debug, Default, Clone)]
pub struct CommandNameSet(FxHashSet<String>);

impl CommandNameSet {
    pub fn contains(&self, name: &str) -> bool {
        self.0.contains(fold(name).as_ref())
    }
}

impl<'a> FromIterator<&'a str> for CommandNameSet {
    fn from_iter<I: IntoIterator<Item = &'a str>>(names: I) -> Self {
        Self(
            names
                .into_iter()
                .map(|name| fold(name).into_owned())
                .collect(),
        )
    }
}

fn is_buffer_number(name: &str) -> bool {
    name.strip_prefix(['b', 'B'])
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_name_set_answers_in_any_case() {
        let set: CommandNameSet = ["write", "FooBar"].into_iter().collect();
        for name in ["write", "WRITE", "foobar", "FOOBAR", "FooBar"] {
            assert!(set.contains(name), "{name}");
        }
        assert!(!set.contains("writ"));
    }
}
