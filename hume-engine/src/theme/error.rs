use std::fmt;

/// Errors that can occur while loading a TOML theme file.
#[derive(Debug)]
pub enum ThemeError {
    /// No file named `<name>.toml` was found in any search path.
    NotFound { name: String },
    /// TOML parse error (syntax / schema mismatch).
    Parse(toml::de::Error),
    /// The `inherits` chain contains a cycle (e.g. A → B → A).
    Cycle { name: String },
    /// The `inherits` chain exceeds the maximum allowed depth.
    MaxDepth { name: String },
    /// A color value could not be parsed (bad hex, unknown palette name).
    BadColor { key: String, value: String },
    /// A scope entry has the wrong TOML value type (must be String or Table).
    BadScopeValue { key: String, value: String },
    /// An unknown modifier name was encountered.
    BadModifier { key: String, value: String },
    /// An unknown underline style name was encountered.
    BadUnderline { key: String, value: String },
    /// I/O error while reading the theme file.
    Io { name: String, error: std::io::Error },
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeError::NotFound { name } => {
                write!(f, "theme '{name}' not found in any search path")
            }
            ThemeError::Parse(e) => write!(f, "TOML parse error: {e}"),
            ThemeError::Cycle { name } => {
                write!(f, "theme '{name}' creates an inherits cycle")
            }
            ThemeError::MaxDepth { name } => {
                write!(f, "theme '{name}' exceeds maximum inherits depth")
            }
            ThemeError::BadColor { key, value } => {
                write!(f, "theme key '{key}': bad color value '{value}'")
            }
            ThemeError::BadScopeValue { key, value } => {
                write!(f, "theme key '{key}': unsupported value type '{value}' (expected string or table)")
            }
            ThemeError::BadModifier { key, value } => {
                write!(f, "theme key '{key}': unknown modifier '{value}'")
            }
            ThemeError::BadUnderline { key, value } => {
                write!(f, "theme key '{key}': unknown underline style '{value}'")
            }
            ThemeError::Io { name, error } => {
                write!(f, "theme '{name}': I/O error: {error}")
            }
        }
    }
}

impl std::error::Error for ThemeError {}
