/// Log-message severity, analogous to [`crate::editor::Severity`] but defined
/// in the scripting layer so scripting code does not depend on the editor crate.
///
/// The editor maps this to [`crate::editor::Severity`] when draining
/// `ScriptingHost::take_pending_messages()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Trace,
}
