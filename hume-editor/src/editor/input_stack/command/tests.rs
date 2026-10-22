use super::*;

#[test]
fn parse_typed_command_table() {
    let cases: &[(&str, &str, bool, Option<&str>)] = &[
        ("", "", false, None),                         // empty
        ("!", "", true, None),                         // lone bang
        ("e", "e", false, None),                       // bare command
        ("e!", "e", true, None),                       // force, no arg
        ("e!path", "e", true, Some("path")),           // force adjacent to arg
        ("e foo", "e", false, Some("foo")),            // space-separated arg
        ("e   foo  ", "e", false, Some("foo")),        // arg trimming
        ("list-buffers", "list-buffers", false, None), // hyphenated name
        ("b#", "b", false, Some("#")),                 // non-alpha arg
        ("b#alt", "b", false, Some("#alt")),           // alternate-buffer style
        ("p1", "p1", false, None),                     // trailing digit
        ("utf8-x arg", "utf8-x", false, Some("arg")),  // inner digit
        ("p1!", "p1", true, None),                     // digit then force
        ("b1", "b", false, Some("1")),                 // buffer number
        ("B12", "B", false, Some("12")),               // buffer number, any case
        ("b1!", "b", false, Some("1!")),               // buffer number keeps `!` in the arg
        ("b12x", "b12x", false, None),                 // `b` + digits + more is a name
        ("42", "", false, Some("42")),                 // line number
    ];
    for &(input, cmd, force, arg) in cases {
        let (got_cmd, got_force, got_arg) = parse_typed_command(input);
        assert_eq!(
            (got_cmd, got_force, got_arg),
            (cmd, force, arg),
            "parse_typed_command({input:?})"
        );
    }
}
