//! The grammar of a typed command's name: what the `:` line reads as the
//! name, and which names a typed command may be registered under.
//!
//! A name starts with a letter, `-` or `_` and continues with letters,
//! digits, `-` or `_`. One exception keeps `:b1` meaning `:b 1`: `b` (any
//! case) followed only by digits reads as the name `b`, the digits being its
//! argument. A digit-led line (`:42`) reads as an empty name, the line
//! number being the argument.

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

/// `Ok` when the `:` line reads `name` back whole as a name, so a command
/// registered under it can be run; otherwise the reason it can't, naming
/// `name`.
pub fn check(name: &str) -> Result<(), String> {
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

fn is_buffer_number(name: &str) -> bool {
    name.strip_prefix(['b', 'B'])
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}
