//! Who may write to the terminal, and `print!`, HUME's way to write a line
//! there.
//!
//! Steel's own print functions write wherever their port points, the
//! editor's screen included. A plugin reports
//! through `log!`, and writes to the terminal with `print!` or a program
//! run by `run-inline-output!`, both only inside an `#:inline-output`
//! command.

use std::io::Write;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::ClaimRefused;

use super::SteelResult;
use super::errors::generic_err;

/// Claims the terminal for `builtin`, which writes to it: only an
/// `#:inline-output` command may, its screen left first. One rule whether
/// or not a screen is up, so a call cannot work at startup and then raise
/// on `:reload-config`. A host with no terminal state refuses.
pub(crate) fn claim_terminal_for(ctx: &mut SteelCtx, builtin: &str) -> Result<(), SteelErr> {
    let claim = ctx
        .host
        .output()
        .map_or(Err(ClaimRefused::NoFrame), |output| output.claim_terminal());
    claim.map_err(|refused| match refused {
        ClaimRefused::NoFrame => generic_err(format!(
            "{builtin}: runs only inside an #:inline-output command"
        )),
        ClaimRefused::Enter(e) => generic_err(format!("{builtin}: {e}")),
    })
}

/// `(print! text)`: writes `text` and a line break to the terminal, inside
/// an `#:inline-output` command.
///
/// # Errors
/// Called outside an `#:inline-output` command, or the write failed.
pub(crate) fn print(ctx: &mut SteelCtx, text: String) -> SteelResult {
    claim_terminal_for(ctx, "print!")?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{text}")
        .and_then(|()| stdout.flush())
        .map_err(|e| generic_err(format!("print!: {e}")))?;
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
