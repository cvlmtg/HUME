//! Scope styles a script set, kept apart from any theme so they apply to
//! whichever theme is installed.

use hume_core::{ByOrigin, ConfigOrigin};
use rustc_hash::FxHashMap;

use crate::theme::style_spec::StyleSpec;

/// The style each origin set for a scope. The user's applies above the
/// theme's own entry for the scope; the highest of the others applies
/// beneath it, only where the theme has no entry of that exact name.
#[derive(Debug, Default)]
pub struct ScopeStyles(FxHashMap<Box<str>, ByOrigin<Option<StyleSpec>>>);

/// A scope's styles as [`super::Theme::apply_scope_styles`] layers them.
pub(crate) struct Layered<'a> {
    pub(crate) scope: &'a str,
    pub(crate) above_theme: Option<&'a StyleSpec>,
    pub(crate) below_theme: Option<&'a StyleSpec>,
}

impl ScopeStyles {
    /// Sets `scope`'s style from `by`, replacing the one `by` set before.
    /// Returns whether a higher origin's style for the scope hides it.
    pub fn set(&mut self, scope: &str, spec: StyleSpec, by: ConfigOrigin) -> bool {
        let layers = self.0.entry(scope.into()).or_default();
        *layers.get_mut(by) = Some(spec);
        layers.above(by).any(Option::is_some)
    }

    pub(crate) fn layered(&self) -> impl Iterator<Item = Layered<'_>> {
        self.0.iter().map(|(scope, layers)| {
            let highest = |above: bool| {
                layers
                    .iter()
                    .filter(|(origin, _)| (*origin == ConfigOrigin::User) == above)
                    .find_map(|(_, spec)| spec.as_ref())
            };
            Layered {
                scope,
                above_theme: highest(true),
                below_theme: highest(false),
            }
        })
    }
}
