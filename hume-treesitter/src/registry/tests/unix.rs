//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::{LanguageRegistry, detect_at};

/// An absolute glob names a Unix root, which no Windows path starts with.
#[test]
fn an_absolute_glob_matches_the_whole_path_only() {
    let mut reg = LanguageRegistry::new();
    reg.register_identity("ini", &[], &["/etc/buckconfig"], &[])
        .unwrap();
    assert_eq!(detect_at("/etc/buckconfig", None, &reg), reg.id_of("ini"));
    assert_eq!(detect_at("/home/etc/buckconfig", None, &reg), None);
}
