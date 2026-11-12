use super::SettingArg;

#[test]
fn a_lone_word_is_the_scope() {
    assert!(matches!(SettingArg::parse("glo"), SettingArg::Scope("glo")));
    assert!(matches!(
        SettingArg::parse("global"),
        SettingArg::Scope("global")
    ));
}

#[test]
fn the_key_follows_the_scope_past_extra_whitespace() {
    assert!(matches!(
        SettingArg::parse("global  th"),
        SettingArg::Key {
            scope: "global",
            key: "th"
        }
    ));
}

#[test]
fn the_value_is_everything_after_the_keys_equals() {
    assert!(matches!(
        SettingArg::parse("global theme=a=b c"),
        SettingArg::Value {
            scope: "global",
            key: "theme",
            value: "a=b c"
        }
    ));
}
