//! The two severity scales HUME shares across crates: how a message is
//! logged and shown, and how serious a language server's diagnostic is.

/// Severity level for a message, controlling both logging and display.
///
/// | Severity | Logged? | Shown as `status_msg`? |
/// |----------|---------|------------------------|
/// | Info     | No      | Yes                    |
/// | Warning  | Yes     | Yes                    |
/// | Error    | Yes     | Yes                    |
/// | Trace    | Yes     | No                     |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Ephemeral confirmation (e.g. "Written 42 lines"). Shown, not logged.
    Info,
    /// Something the user should review (e.g. unknown config key). Logged and shown.
    Warning,
    /// A failure the user must address (e.g. script error). Logged and shown.
    Error,
    /// Verbose diagnostic detail (e.g. stack trace). Logged only, not shown in statusline.
    Trace,
}

impl Severity {
    pub const fn name(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Trace => "trace",
        }
    }
}

/// How serious a diagnostic is. Ordered least-to-most-lenient so
/// `severity <= floor` means "at least as severe as floor": e.g. `floor =
/// Warning` keeps `Error` and `Warning`, drops `Info`/`Hint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl DiagSeverity {
    /// Every severity, in [`Self::rank`] order.
    pub const ALL: [Self; 4] = [Self::Error, Self::Warning, Self::Info, Self::Hint];

    /// [`Self::name`] of each severity, in [`Self::ALL`]'s order: the one
    /// spelling the `:set` value, the theme scopes and scripts share.
    pub const NAMES: [&'static str; 4] = ["error", "warning", "info", "hint"];

    pub const fn name(self) -> &'static str {
        Self::NAMES[self.rank() as usize]
    }

    /// Place on the scale, `0` for `Error` up to `3` for `Hint`: lower is
    /// more severe, and it indexes [`Self::ALL`].
    pub const fn rank(self) -> u8 {
        self as u8
    }
}
