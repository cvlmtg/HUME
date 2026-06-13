pub(crate) const HUME_CTX: &str = "*hume.ctx*";

/// Internal Steel global name for the i-th argument bound during a hook fire.
pub(super) fn hook_arg_name(i: usize) -> String {
    format!("*hume.ha{i}*")
}

/// Internal Steel global name for the i-th handler proc bound during a hook fire.
pub(super) fn hook_proc_name(i: usize) -> String {
    format!("*hume.hp{i}*")
}

/// Internal Steel global name for the i-th positional arg bound during a command call.
pub(crate) fn cmd_arg_global_name(i: usize) -> String {
    format!("*hume.ca{i}*")
}

/// Build the composite hook invocation program for `handler_count` handlers
/// and `arg_count` arguments.  The result is deterministic and cacheable.
pub(super) fn build_hook_program(arg_count: usize, handler_count: usize) -> String {
    // 14 = len("*hume.ha99* ") worst-case per arg; 18 = len("(*hume.hp99*)\n") per handler.
    let mut arg_exprs = String::with_capacity(arg_count * 14);
    for i in 0..arg_count {
        if i > 0 {
            arg_exprs.push(' ');
        }
        arg_exprs.push_str(&hook_arg_name(i));
    }
    let mut program = String::with_capacity(handler_count * (18 + arg_exprs.len()));
    for i in 0..handler_count {
        if i > 0 {
            program.push('\n');
        }
        program.push('(');
        program.push_str(&hook_proc_name(i));
        if arg_count > 0 {
            program.push(' ');
            program.push_str(&arg_exprs);
        }
        program.push(')');
    }
    program
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Global name formatters ────────────────────────────────────────────────

    #[test]
    fn hook_arg_name_formats_correctly() {
        assert_eq!(hook_arg_name(0), "*hume.ha0*");
        assert_eq!(hook_arg_name(5), "*hume.ha5*");
        assert_eq!(hook_arg_name(99), "*hume.ha99*");
    }

    #[test]
    fn hook_proc_name_formats_correctly() {
        assert_eq!(hook_proc_name(0), "*hume.hp0*");
        assert_eq!(hook_proc_name(3), "*hume.hp3*");
        assert_eq!(hook_proc_name(99), "*hume.hp99*");
    }

    #[test]
    fn cmd_arg_global_name_formats_correctly() {
        assert_eq!(cmd_arg_global_name(0), "*hume.ca0*");
        assert_eq!(cmd_arg_global_name(7), "*hume.ca7*");
    }

    // ── build_hook_program ────────────────────────────────────────────────────

    /// Zero handlers → empty program.
    ///
    /// Fail oracle: return a non-empty string even for 0 handlers → the eval
    /// attempts to call undefined procs.
    #[test]
    fn build_hook_program_zero_handlers_is_empty() {
        assert_eq!(build_hook_program(0, 0), "");
        assert_eq!(build_hook_program(2, 0), "");
    }

    /// One handler, no args → `(*hume.hp0*)`.
    #[test]
    fn build_hook_program_one_handler_no_args() {
        assert_eq!(build_hook_program(0, 1), "(*hume.hp0*)");
    }

    /// One handler, one arg → `(*hume.hp0* *hume.ha0*)`.
    #[test]
    fn build_hook_program_one_handler_one_arg() {
        assert_eq!(build_hook_program(1, 1), "(*hume.hp0* *hume.ha0*)");
    }

    /// Two handlers, two args: each call gets both args; handlers are separated by `\n`.
    ///
    /// Fail oracle: wrong separator or missing args → Steel parse error.
    #[test]
    fn build_hook_program_two_handlers_two_args() {
        let prog = build_hook_program(2, 2);
        assert_eq!(
            prog,
            "(*hume.hp0* *hume.ha0* *hume.ha1*)\n(*hume.hp1* *hume.ha0* *hume.ha1*)"
        );
    }

    /// The program is deterministic — calling twice with the same args produces identical output.
    #[test]
    fn build_hook_program_is_deterministic() {
        assert_eq!(build_hook_program(3, 4), build_hook_program(3, 4));
    }
}
