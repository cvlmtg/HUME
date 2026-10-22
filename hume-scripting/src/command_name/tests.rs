use super::*;

#[test]
fn command_name_set_answers_in_any_case() {
    let set: CommandNameSet = ["write", "FooBar"].into_iter().collect();
    for name in ["write", "WRITE", "foobar", "FOOBAR", "FooBar"] {
        assert!(set.contains(name), "{name}");
    }
    assert!(!set.contains("writ"));
}
