use super::*;
use pretty_assertions::assert_eq;

// ── :b with no argument ───────────────────────────────────────────────────────

#[test]
fn buffer_no_arg_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", None).unwrap_err();
    assert!(
        err.to_string().contains("usage"),
        "must show usage, got: {err}"
    );
}

// ── :b by 1-based index ───────────────────────────────────────────────────────

#[test]
fn buffer_index_zero_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", Some("0")).unwrap_err();
    assert!(
        err.to_string().contains("index"),
        "must mention 'index', got: {err}"
    );
}

#[test]
fn buffer_index_out_of_range_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", Some("99")).unwrap_err();
    assert!(
        err.to_string().contains("99"),
        "must mention the bad index, got: {err}"
    );
}

// ── :b by full absolute path ──────────────────────────────────────────────────

// ── :b by exact basename ──────────────────────────────────────────────────────

// ── :b by basename prefix ─────────────────────────────────────────────────────

#[test]
fn buffer_no_match_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("b", Some("definitely_not_a_buffer_xyz"))
        .unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        "must say 'no buffer matching', got: {err}"
    );
}

// ── :b scratch buffer ─────────────────────────────────────────────────────────

// ── :b current buffer is a no-op ─────────────────────────────────────────────

#[test]
fn buffer_current_buffer_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    // The only buffer is the scratch buffer; :b *scratch* should be a no-op.
    let before_id = ed.focused_buffer_id();
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        before_id,
        ":b to current buffer must not change focus"
    );
}

// ── :b and :buffer aliases ────────────────────────────────────────────────────

#[test]
fn buffer_long_alias_accepted() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("buffer", Some("xyz_no_such_buf"))
        .unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        "canonical name 'buffer' must work too, got: {err}"
    );
}

#[test]
fn buffer_bang_force_is_ignored() {
    // `:b` takes a `force` flag for syntactic compatibility with the
    // `<cmd>!` convention, but there is nothing to force on a plain
    // buffer switch — `:b!` must behave identically to `:b`.
    let mut ed = editor_from("-[h]>ello\n");
    let before_id = ed.focused_buffer_id();
    ed.execute_typed("b!", Some("*scratch*")).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        before_id,
        ":b! to current buffer must be a no-op, same as :b"
    );
    let err = ed.execute_typed("b!", Some("xyz_no_such_buf")).unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        ":b! must still report resolution errors, got: {err}"
    );
}

// ── :b on a buffer whose backing file has been deleted ───────────────────────

// ── Ctrl+O restores position after :b ────────────────────────────────────────
