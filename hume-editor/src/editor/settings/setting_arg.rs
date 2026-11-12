//! The argument grammar `:set` and `:get` share: `<scope> <key>=<value>`,
//! read whole on Enter and up to the cursor by Tab completion.

/// Text the `:set`/`:get` argument grammar splits: the argument on Enter,
/// or the part of it before the cursor while completing.
pub(in crate::editor) trait ArgSplit<'a>: Copy {
    /// The text before the first `sep`, and the rest after it.
    fn split_once(self, sep: char) -> Option<(&'a str, Self)>;

    fn trim_start(self) -> Self;
}

impl<'a> ArgSplit<'a> for &'a str {
    fn split_once(self, sep: char) -> Option<(&'a str, Self)> {
        str::split_once(self, sep)
    }

    fn trim_start(self) -> Self {
        str::trim_start(self)
    }
}

/// How far a `:set`/`:get` argument goes. Whitespace before the key is
/// tolerated, and a value may contain `=` and spaces.
pub(in crate::editor) enum SettingArg<'a, T> {
    Scope(T),
    Key {
        scope: &'a str,
        key: T,
    },
    Value {
        scope: &'a str,
        key: &'a str,
        value: T,
    },
}

impl<'a, T: ArgSplit<'a>> SettingArg<'a, T> {
    pub(in crate::editor) fn parse(arg: T) -> Self {
        let Some((scope, rest)) = arg.split_once(' ') else {
            return Self::Scope(arg);
        };
        let rest = rest.trim_start();
        match rest.split_once('=') {
            None => Self::Key { scope, key: rest },
            Some((key, value)) => Self::Value { scope, key, value },
        }
    }
}

#[cfg(test)]
mod tests;
