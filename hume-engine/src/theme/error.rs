use std::fmt;
use std::path::PathBuf;

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
    /// A reserved top-level key (`inherits`, `palette`) has the wrong TOML
    /// value type.
    BadReservedKey {
        key: &'static str,
        expected: &'static str,
    },
    /// An unknown modifier name was encountered.
    BadModifier { key: String, value: String },
    /// An unknown underline style name was encountered.
    BadUnderline { key: String, value: String },
    /// A style field has the wrong TOML value type (`fg`, `bg`, `underline`,
    /// `modifiers`, or the extended `underline.color`/`underline.style`), or a
    /// scope's style table carries a field that is not a style field at all —
    /// in which case `field` is the offending name and `expected` lists the
    /// ones that are.
    BadStyleField {
        key: String,
        field: String,
        expected: &'static str,
    },
    /// I/O error while reading `path`, the candidate file being tried for
    /// theme `name` when the read failed.
    Io {
        name: String,
        path: PathBuf,
        error: std::io::Error,
    },
    /// `error` was produced while loading `path` — attached wherever the
    /// source file is known: a document's own parse/validation, or a
    /// resolve-time failure traced back through the `inherits` merge to the
    /// document that actually defined the offending key.
    InFile {
        path: PathBuf,
        error: Box<ThemeError>,
    },
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
                write!(
                    f,
                    "theme key '{key}': unsupported value type '{value}' (expected string or table)"
                )
            }
            ThemeError::BadReservedKey { key, expected } => {
                write!(f, "theme key '{key}': expected {expected}")
            }
            ThemeError::BadModifier { key, value } => {
                write!(f, "theme key '{key}': unknown modifier '{value}'")
            }
            ThemeError::BadUnderline { key, value } => {
                write!(f, "theme key '{key}': unknown underline style '{value}'")
            }
            ThemeError::BadStyleField {
                key,
                field,
                expected,
            } => {
                write!(f, "theme key '{key}': field '{field}' must be {expected}")
            }
            ThemeError::Io { name, path, error } => {
                write!(
                    f,
                    "theme '{name}': I/O error reading {}: {error}",
                    path.display()
                )
            }
            ThemeError::InFile { path, error } => {
                write!(f, "{}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for ThemeError {}
