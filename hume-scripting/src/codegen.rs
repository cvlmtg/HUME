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
