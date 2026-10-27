//! Data that two or more crates share and that no crate they can all see
//! owns.
//!
//! An item lives in the crate whose domain it is, and here when a crate that
//! uses it cannot depend on that crate: a store's vocabulary that a lower
//! crate reads too, or a type two crates would otherwise mirror. A dependency
//! the users have on each other for other reasons never keeps an item in a
//! crate whose domain it is not. Data whose behaviour belongs to a domain
//! crate stays there (rope positions, the grid, paths, the buffer text). A
//! rule such a type validates by lives here with it ([`is_space_char`]), and
//! the lower crates that share the rule call it here. The crate depends on no
//! workspace crate, so any crate can use it; third-party crates are fine.

/// Declares an enum whose variants each have a name, and generates its
/// `ALL`, `NAMES`, `NAMED` and `name()` from one `Variant => "name"` list,
/// so no variant can be missing from a table or spelled two ways.
macro_rules! named_enum {
    (
        $(#[$meta:meta])*
        pub enum $enum:ident {
            $($(#[$variant_meta:meta])* $variant:ident => $name:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        pub enum $enum {
            $($(#[$variant_meta])* $variant),+
        }

        impl $enum {
            /// Every variant, in declaration order.
            pub const ALL: [Self; [$(Self::$variant),+].len()] = [$(Self::$variant),+];

            /// [`Self::name`] of each variant, in [`Self::ALL`]'s order.
            pub const NAMES: [&'static str; [$(Self::$variant),+].len()] = [$($name),+];

            /// Every `(name, variant)` pair, the table a name is decoded
            /// through.
            pub const NAMED: [(&'static str, Self); [$(Self::$variant),+].len()] =
                [$(($name, Self::$variant)),+];

            /// The variant's name as users and scripts spell it.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }
    };
}

mod command;
mod comment;
mod completion;
mod ids;
mod keymap;
mod language;
mod lsp;
mod non_empty;
mod origin;
mod picker;
mod popup;
mod setting;
mod severity;
mod side;
mod text;
mod timer;
mod trigger;
mod unsaved;

pub use command::{CommandArg, MAX_COUNT};
pub use comment::{
    BlockCommentPair, BlockCommentTokens, CommentTokenError, CommentTokens, LineCommentTokens,
};
pub use completion::{AnswerEnd, FirstTab, MatchKind, NamedPrefix};
pub use ids::{BufferId, HostToken, PaneHandle, PaneId};
pub use keymap::BindMode;
pub use language::{
    BUFFER_LANGUAGE_TIE, FileExtension, FileGlob, GrammarSource, InvalidExtension,
    InvalidShebangInterpreter, LanguageField, LanguageFields, LanguageIdentity, OptionalQuery,
    QueryFiles, ShebangInterpreter,
};
pub use lsp::{
    FeatureFilter, ListEntry, ListTier, LspFeature, LspFeatureSet, LspServerConfig,
    SERVER_REGISTRATION_TIE, ServerName,
};
pub use non_empty::NonEmpty;
pub use origin::{ByOrigin, ConfigOrigin, Ranked, Tie, WriteOutcome};
pub use picker::TruncateEnd;
pub use popup::PopupLayout;
pub use setting::{BufferIntrinsicOption, OptionInput, language_option_value};
pub use severity::{DiagSeverity, Severity};
pub use side::Side;
pub use text::is_space_char;
pub use timer::{TimerId, TimerIds};
pub use trigger::TriggerListener;
pub use unsaved::OnUnsaved;
