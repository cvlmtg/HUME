use hume_core::ScopeStyleInput;
use hume_grid::Rgb;

use super::*;

fn input(edit: impl FnOnce(&mut ScopeStyleInput)) -> ScopeStyleInput {
    let mut input = ScopeStyleInput {
        scope: "keyword".to_owned(),
        ..Default::default()
    };
    edit(&mut input);
    input
}

#[test]
fn an_input_parses_every_field() {
    let spec = StyleSpec::from_input(&input(|i| {
        i.fg = Some("#ff0000".to_owned());
        i.bg = Some("gold".to_owned());
        i.underline_color = Some("#00f".to_owned());
        i.underline_style = Some("curl".to_owned());
        i.modifiers = vec!["bold".to_owned(), "crossed_out".to_owned()];
    }))
    .unwrap();
    assert_eq!(
        spec,
        StyleSpec {
            fg: Some(ColorRef::Rgb(Rgb(0xff, 0, 0))),
            bg: Some(ColorRef::Name("gold".into())),
            underline: UnderlineStyle::Wavy,
            underline_color: Some(ColorRef::Rgb(Rgb(0, 0, 0xff))),
            modifiers: Modifiers::BOLD | Modifiers::STRIKETHROUGH,
        }
    );
}

#[test]
fn the_underlined_modifier_sets_a_solid_underline_unless_a_style_is_given() {
    let plain = StyleSpec::from_input(&input(|i| i.modifiers = vec!["underlined".to_owned()]));
    assert_eq!(plain.unwrap().underline, UnderlineStyle::Solid);

    let styled = StyleSpec::from_input(&input(|i| {
        i.underline_style = Some("dotted".to_owned());
        i.modifiers = vec!["underlined".to_owned()];
    }));
    assert_eq!(styled.unwrap().underline, UnderlineStyle::Dotted);
}

#[test]
fn a_malformed_hex_color_is_refused() {
    let err = StyleSpec::from_input(&input(|i| i.underline_color = Some("#12".to_owned())));
    assert!(
        matches!(&err, Err(ThemeError::BadColor { field: Some("underline.color"), value, .. }) if value == "#12"),
        "{err:?}"
    );
}

#[test]
fn an_unknown_modifier_is_refused() {
    let err = StyleSpec::from_input(&input(|i| i.modifiers = vec!["blod".to_owned()]));
    assert!(
        matches!(&err, Err(ThemeError::BadModifier { value, .. }) if value == "blod"),
        "{err:?}"
    );
}

#[test]
fn an_unknown_underline_style_is_refused() {
    let err = StyleSpec::from_input(&input(|i| i.underline_style = Some("wavy".to_owned())));
    assert!(
        matches!(&err, Err(ThemeError::BadUnderline { value, .. }) if value == "wavy"),
        "{err:?}"
    );
}
