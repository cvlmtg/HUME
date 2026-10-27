//! Data that two or more crates share and no domain crate owns.
//!
//! A type lives here when it is plain data with no behaviour tied to a
//! domain crate (rope positions, the grid, paths, the buffer text), and
//! either two crates would otherwise keep a mirrored copy of it or the crate
//! that defined it is a dependency of some consumer only for that type. A
//! rule such a type validates by lives here with it ([`is_space_char`]), and
//! the lower crates that share the rule call it here. The crate depends on no
//! workspace crate, so any crate can use it; third-party crates are fine.

mod comment;
mod completion;
mod ids;
mod keymap;
mod language;
mod origin;
mod picker;
mod popup;
mod severity;
mod side;
mod text;
mod timer;
mod unsaved;

pub use comment::{
    BlockCommentPair, BlockCommentTokens, CommentTokenError, CommentTokens, LineCommentTokens,
};
pub use completion::MatchKind;
pub use ids::{BufferId, PaneId};
pub use keymap::BindMode;
pub use language::{
    FileExtension, FileGlob, GrammarSource, InvalidExtension, InvalidShebangInterpreter,
    LanguageField, LanguageFields, LanguageIdentity, OptionalQuery, QueryFiles, ShebangInterpreter,
};
pub use origin::{ByOrigin, ConfigOrigin, Ranked, Tie, WriteOutcome};
pub use picker::TruncateEnd;
pub use popup::PopupLayout;
pub use severity::{DiagSeverity, Severity};
pub use side::Side;
pub use text::is_space_char;
pub use timer::{TimerId, TimerIds};
pub use unsaved::OnUnsaved;
