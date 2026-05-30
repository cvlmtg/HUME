/// Log-message severity, defined in the scripting layer so scripting code does
/// not depend on the editor crate.
///
/// The editor maps this to its own `Severity` enum when draining
/// `ScriptingHost::take_pending_messages()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Trace,
}
