use super::*;

fn rust_only() -> BufferVisibility {
    BufferVisibility::declared(Rc::from(vec!["rust".to_string()]))
}

#[test]
fn unscoped_sees_every_buffer_and_one_without_language() {
    let v = BufferVisibility::default();
    assert!(v.sees(Some("rust")));
    assert!(v.sees(Some("markdown")));
    assert!(v.sees(None));
}

#[test]
fn scoped_sees_only_its_languages() {
    let v = BufferVisibility::declared(Rc::from(vec!["rust".to_string(), "toml".to_string()]));
    assert!(v.sees(Some("rust")));
    assert!(v.sees(Some("toml")));
    assert!(!v.sees(Some("markdown")));
}

#[test]
fn scoped_does_not_see_a_buffer_without_language() {
    assert!(!rust_only().sees(None));
}

#[test]
fn everyone_admits_a_handler_whatever_its_languages() {
    assert!(Audience::Everyone.admits(&rust_only()));
}

#[test]
fn seeing_admits_the_handlers_that_see_the_language() {
    assert!(Audience::Seeing(Some("rust")).admits(&rust_only()));
    assert!(!Audience::Seeing(Some("go")).admits(&rust_only()));
    assert!(Audience::Seeing(Some("go")).admits(&BufferVisibility::default()));
}

#[test]
fn crossing_admits_only_handlers_that_see_one_language_and_not_the_other() {
    let crossing = |sees, misses| Audience::Crossing { sees, misses };
    assert!(crossing(Some("rust"), Some("go")).admits(&rust_only()));
    assert!(crossing(Some("rust"), None).admits(&rust_only()));
    assert!(
        !crossing(Some("rust"), Some("rust")).admits(&rust_only()),
        "a handler that sees both"
    );
    assert!(!crossing(Some("go"), Some("rust")).admits(&rust_only()));
    assert!(
        !crossing(Some("rust"), Some("go")).admits(&BufferVisibility::default()),
        "an unscoped handler sees both"
    );
}

#[test]
fn a_list_naming_the_wildcard_sees_every_buffer() {
    let v = BufferVisibility::declared(Rc::from(vec!["rust".to_string(), "*".to_string()]));
    assert_eq!(v, BufferVisibility::default());
}
