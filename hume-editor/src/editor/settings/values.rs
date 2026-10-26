//! The value types settings take, and how each reads from and writes to
//! its text form (`SettingText`).

use hume_editing::tab_style::TabStyle;
use hume_editing::text::LineEnding;
use hume_engine::builtins::line_number::LineNumberStyle;
use hume_engine::pane::{WhitespaceRender, WrapMode};
use hume_ops::WhitespaceSide;
use hume_platform::process::batch::ShellCommand;

use super::{SettingText, or_list, word_list};
use crate::editor::lsp::diagnostics::DiagSeverity;

// ── SignColumnConfig ──────────────────────────────────────────────────────────

/// Whether the sign column stays visible or collapses when empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignColumnMode {
    /// Always visible, regardless of whether any signs exist.
    #[default]
    Always,
    /// Collapses to zero width when no signs are visible in the current
    /// viewport (a sign elsewhere in the buffer, scrolled out of view,
    /// does not keep the column open).
    Auto,
}

settings_enum!(SignColumnMode, [
    Always => "always",
    Auto => "auto",
]);

/// Sign column configuration: visibility mode and, optionally, a pinned
/// number of sign slots.
///
/// Wire format: `"always"`, `"always:N"`, `"auto"`, `"auto:N"` where N is the
/// number of sign slots (1–127). Bare `"always"`/`"auto"` (`pinned_slots:
/// None`) auto-sizes the column to the buffer's registered sign sources,
/// one slot per source, regardless of whether it has placed a sign anywhere
/// in the buffer (see `DecorationStores::sign_source_count`, the sole place
/// `slots_for`'s `None` branch reads); `":N"` pins the count instead,
/// hiding any source ranked at or past `N`. Default is bare `"always"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignColumnConfig {
    pub mode: SignColumnMode,
    /// Explicit slot count from a `:N` suffix, or `None` to auto-size.
    pub pinned_slots: Option<u8>,
}

impl Default for SignColumnConfig {
    fn default() -> Self {
        Self {
            mode: SignColumnMode::Always,
            pinned_slots: None,
        }
    }
}

impl SignColumnConfig {
    /// Upper bound on a pinned `:N` slot count, and on how many slots bare
    /// `gutter-signs=always`/`auto` auto-sizes to. The `:N` bound is the type
    /// domain `FromStr` parses into (`u8`, and `127` keeps a pinned column
    /// from outrunning what a terminal row can usefully show); auto-size
    /// reuses the same constant rather than defining its own smaller cap,
    /// since a buffer can't register more distinct sources than a user could
    /// otherwise pin explicitly.
    pub const MAX_SLOTS: u8 = 127;

    /// Resolves the configured slot count against `source_count`, the
    /// number of registered sign sources. An explicit `:N` pins the count
    /// regardless of `source_count`; auto-size clamps to `[1, MAX_SLOTS]`,
    /// never below 1, so the column stays visible under `always` even with
    /// zero registered sources. Returns a bare slot count, not a gutter
    /// width: `EditorState::sign_slots` (its only caller) hands it to
    /// `Editor::update_sign_providers`, which bounds the per-line `Vec<Sign>`
    /// it builds by it, and to `EditorState::gutter_inputs`, which converts
    /// it to a width with `SignColumn::width_for_slots`.
    pub fn slots_for(self, source_count: usize) -> u8 {
        self.pinned_slots
            .unwrap_or_else(|| source_count.clamp(1, Self::MAX_SLOTS as usize) as u8)
    }
}

impl SettingText for SignColumnConfig {
    /// Completion hints only, not an exhaustive list like the closed enums'
    /// `VALUES`, because `:N` accepts 1–127, which can't be listed in full.
    /// `:1`/`:2` are illustrative of the pinned-vs-auto-size distinction, not
    /// a reflection of `MAX_SLOTS`; raising that cap doesn't require
    /// extending this list.
    const VALUES: &'static [&'static str] =
        &["always", "auto", "always:1", "auto:1", "always:2", "auto:2"];

    fn parse(s: &str) -> Result<Self, String> {
        let (mode_str, slots_str) = match s.split_once(':') {
            Some((m, c)) => (m, Some(c)),
            None => (s, None),
        };
        let mode = SignColumnMode::parse(mode_str)?;
        let pinned_slots = match slots_str {
            Some(c) => {
                let n: u8 = c
                    .parse()
                    .map_err(|_| format!("expected 1–127 slots, got '{c}'"))?;
                if n == 0 || n > Self::MAX_SLOTS {
                    return Err(format!("expected 1–127 slots, got '{n}'"));
                }
                Some(n)
            }
            None => None,
        };
        Ok(Self { mode, pinned_slots })
    }

    fn to_text(&self) -> String {
        match self.pinned_slots {
            Some(n) => format!("{}:{n}", self.mode.to_text()),
            None => self.mode.to_text(),
        }
    }
}

settings_enum!(DiagSeverity, [
    Error => "error",
    Warning => "warning",
    Info => "info",
    Hint => "hint",
]);

settings_enum!(LineEnding, [
    Lf => "lf",
    CrLf => "crlf",
]);

settings_enum!(TabStyle, [
    Hard => "hard",
    Soft => "soft",
]);

settings_enum!(LineNumberStyle, [
    Absolute => "absolute",
    Relative => "relative",
    Hybrid => "hybrid",
]);

settings_enum!(WhitespaceRender, [
    None => "none",
    All => "all",
    Trailing => "trailing",
]);

/// `none`, a bare `soft`/`word`/`indent` (wrap at the content width), or
/// `soft:N`/`word:N`/`indent:N` (wrap at column N, `0` also meaning the
/// content width). `VALUES` lists the bare keywords only: completion leaves
/// the column count to the user. `to_text` writes the bare keyword for the
/// content width, the spelling a user writes for it.
impl SettingText for WrapMode {
    const VALUES: &'static [&'static str] = &["none", "soft", "word", "indent"];

    fn parse(s: &str) -> Result<Self, String> {
        let lower = s.to_ascii_lowercase();
        let (kind, width) = match lower.split_once(':') {
            Some((kind, rest)) => {
                let width = rest
                    .parse()
                    .map_err(|_| format!("width in '{s}' must be a column count, got '{rest}'"))?;
                (kind, Some(width))
            }
            None => (lower.as_str(), None),
        };
        match (kind, width) {
            ("none", None) => Ok(Self::None),
            ("soft", w) => Ok(Self::Soft {
                width: w.unwrap_or(0),
            }),
            ("word", w) => Ok(Self::Word {
                width: w.unwrap_or(0),
            }),
            ("indent", w) => Ok(Self::Indent {
                width: w.unwrap_or(0),
            }),
            _ => Err(format!(
                "expected none, soft[:N], word[:N], or indent[:N], got '{s}'"
            )),
        }
    }

    fn to_text(&self) -> String {
        match self {
            Self::None => "none".to_owned(),
            Self::Soft { width: 0 } => "soft".to_owned(),
            Self::Word { width: 0 } => "word".to_owned(),
            Self::Indent { width: 0 } => "indent".to_owned(),
            Self::Soft { width } => format!("soft:{width}"),
            Self::Word { width } => format!("word:{width}"),
            Self::Indent { width } => format!("indent:{width}"),
        }
    }
}

/// Where a forward object jump (`}`, `goto-next-<kind>`) leaves the viewport.
///
/// Exists because those motions land the selection head at the *start* of
/// the object just found (`hume_ops::motion::object::apply_object_motion`
/// builds it facing backward, so a following `w` walks into the object's
/// body), and the body then extends *below* that head. A
/// forward jump scrolls downward, so the default per-frame scroll parks the
/// head at `scroll-margin` rows from the *bottom*, hiding the very body the
/// head-first convention was chosen to show. The backward motions (`{`)
/// don't have this problem: their head lands at the object's start too, but
/// an upward scroll parks it at `scroll-margin` rows from the *top*, so the body
/// below it is already on screen. Hence only the forward motions read this
/// setting; see `CmdMeta::aligns_view` in `editor::registry::command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectJumpAlign {
    /// Pin the head at the viewport's top row, subject to `scroll-margin` on
    /// the next frame, exactly like `z k`.
    Top,
    /// Center the head in the viewport, like `z z`.
    #[default]
    Center,
    /// No extra alignment: the pre-existing per-frame `scroll-margin` scroll is
    /// all that runs.
    Off,
}

settings_enum!(ObjectJumpAlign, [
    Top => "top",
    Center => "center",
    Off => "off",
]);

/// The real terminal cursor's shape in Insert mode: Helix's
/// `editor.cursor-shape.insert`, minus the `hidden` variant Helix offers
/// mainly for IME positioning. Applies to every selection head, not just the
/// primary (HUME's own departure from Helix). See the Tier 1/0 comment in
/// `hume_engine::style::style_display_line` for why and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// Every selection head is painted from its own themed scope (the
    /// primary from `ui.cursor.primary.insert`, a secondary from
    /// `ui.cursor.insert`, both falling back through the usual ladder); the
    /// real terminal cursor is hidden.
    Block,
    /// Every selection head is left unpainted. The real terminal cursor
    /// shows as a thin bar, marking the primary; secondary heads are visible
    /// only where they fall inside a highlighted selection. This is HUME's
    /// long-standing default.
    #[default]
    Bar,
    /// Same as `Bar`, but the real terminal cursor shows as an underline.
    Underline,
}

settings_enum!(CursorShape, [
    Block => "block",
    Bar => "bar",
    Underline => "underline",
]);

/// When to show the tab bar (top row of the terminal area).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TablineVisibility {
    /// Always shown, even with a single tab open.
    Always,
    /// Never shown, regardless of how many tabs are open.
    Never,
    /// Shown only once more than one tab is open. HUME's default.
    #[default]
    Dynamic,
}

settings_enum!(TablineVisibility, [
    Always => "always",
    Never => "never",
    Dynamic => "dynamic",
]);

/// What the first Tab on the `:` line does when several candidates match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandCompletion {
    /// Extends the token to the longest prefix every candidate shares and
    /// opens the popup with nothing picked. HUME's default.
    #[default]
    CommonPrefix,
    /// Applies the first candidate and opens the popup on it.
    FirstCandidate,
}

settings_enum!(CommandCompletion, [
    CommonPrefix => "common-prefix",
    FirstCandidate => "first-candidate",
]);

/// Which whitespace the word family (`w`/`W`/`b`/`B`, `mm`/`MM`,
/// `maw`/`maW`) adds to the word it selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordSelectsWhitespace {
    /// The bare word.
    Off,
    /// The run before the word, else the run after it.
    #[default]
    Before,
    /// The run after the word, else the run before it.
    After,
}

impl WordSelectsWhitespace {
    pub(in crate::editor) fn side(self) -> Option<WhitespaceSide> {
        match self {
            Self::Off => None,
            Self::Before => Some(WhitespaceSide::Before),
            Self::After => Some(WhitespaceSide::After),
        }
    }
}

settings_enum!(WordSelectsWhitespace, [
    Off => "off",
    Before => "before",
    After => "after",
]);

impl SettingText for ShellCommand {
    const VALUES: &'static [&'static str] = &[];

    fn parse(s: &str) -> Result<Self, String> {
        Self::from_words(word_list::parse_word_list(s)?)
    }

    fn to_text(&self) -> String {
        word_list::format_word_list(&self.words())
    }
}
