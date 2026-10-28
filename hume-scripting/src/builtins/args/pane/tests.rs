use super::*;
use crate::builtins::ids::SteelPane;

/// Every pane argument type decodes only a pane: accepting any value would
/// let every argument decode as one. Its closed-buffer check is proven
/// through a real `ScriptingHost` in `builtins::tests`, and the host's pane
/// checks through a real editor in hume-editor.
#[test]
fn every_pane_argument_type_rejects_a_non_pane() {
    let val = SteelVal::StringV("not-a-pane".into());
    let errors = [
        ArgPane::from_steelval(&val).err(),
        LiveBuffer::from_steelval(&val).err(),
        RequesterPane::from_steelval(&val).err(),
        ShownPane::from_steelval(&val).err(),
        FocusedPane::from_steelval(&val).err(),
    ];
    for err in errors {
        let err = err.expect("a string is not a pane");
        assert!(err.to_string().contains("expected pane"), "got: {err}");
    }
}

#[test]
fn every_pane_argument_type_decodes_the_handle_it_was_given() {
    let handle = crate::test_support::default_pane_with_pane();
    let val = SteelPane::new(handle).into_steel_val();
    assert_eq!(
        [
            ArgPane::from_steelval(&val).unwrap().0,
            LiveBuffer::from_steelval(&val).unwrap().0,
            RequesterPane::from_steelval(&val).unwrap().0,
            ShownPane::from_steelval(&val).unwrap().0,
            FocusedPane::from_steelval(&val).unwrap().0,
        ],
        [handle; 5]
    );
}
