//! Helpers shared by this module's unix tests.

use nix::sys::signal::kill;
use nix::unistd::Pid;

/// Whether a process with this id exists. A zombie still counts.
pub(super) fn is_alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}
