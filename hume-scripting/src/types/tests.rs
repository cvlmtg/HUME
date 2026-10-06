use super::{FeatureFilter, LspFeature, LspFeatureSet, ServerName};

#[test]
fn server_name_rejects_empty_and_whitespace() {
    assert!(ServerName::parse("").is_err());
    assert!(ServerName::parse("rust analyzer").is_err());
    assert!(ServerName::parse("ruff\t").is_err());
    assert_eq!(
        ServerName::parse("rust-analyzer").map(|n| n.as_str().to_owned()),
        Ok("rust-analyzer".to_owned())
    );
}

#[test]
fn feature_filter_only_and_except_admit_inversely() {
    let set: LspFeatureSet = [LspFeature::Format, LspFeature::Diagnostics]
        .into_iter()
        .collect();
    let only = FeatureFilter::Only(set);
    let except = FeatureFilter::Except(set);
    for feature in LspFeature::ALL {
        let listed = matches!(feature, LspFeature::Format | LspFeature::Diagnostics);
        assert_eq!(only.admits(feature), listed, "{}", feature.name());
        assert_eq!(except.admits(feature), !listed, "{}", feature.name());
        assert!(FeatureFilter::All.admits(feature));
    }
}

#[test]
fn feature_names_round_trip_through_the_table() {
    assert_eq!(LspFeature::ALL.len(), 21);
    for (name, feature) in LspFeature::NAMED {
        assert_eq!(feature.name(), name);
    }
}
