//! Scope styles a script set, kept apart from any theme so they apply to
//! whichever theme is installed.

use std::collections::BTreeMap;

use hume_core::{ByOrigin, ConfigOrigin};

use crate::theme::style_spec::StyleSpec;

/// The style each origin set for a scope. The user's applies above the
/// theme's own entry for the scope; the highest of the others applies
/// beneath it, only where the theme has no entry of that exact name.
/// Ordered by scope name, so a theme install warns in that order.
#[derive(Debug, Default)]
pub struct ScopeStyles {
    styles: BTreeMap<Box<str>, ByOrigin<Option<StyleSpec>>>,
    /// The scopes set since [`super::Theme::apply_scope_styles`] last took
    /// them, in the order first set.
    unshown: Vec<Box<str>>,
}

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
        let hidden = self
            .styles
            .entry(scope.into())
            .or_default()
            .set(by, Some(spec));
        if !self.unshown.iter().any(|unshown| **unshown == *scope) {
            self.unshown.push(scope.into());
        }
        hidden
    }

    pub(crate) fn layered(&self) -> impl Iterator<Item = Layered<'_>> {
        self.styles
            .iter()
            .map(|(scope, layers)| Self::layer(scope, layers))
    }

    /// Whether a scope was set since a theme last took the styles.
    pub fn has_unshown(&self) -> bool {
        !self.unshown.is_empty()
    }

    /// The scopes set since the last call, emptying the list.
    pub(crate) fn take_unshown(&mut self) -> Vec<Box<str>> {
        std::mem::take(&mut self.unshown)
    }

    /// `scope`'s styles, `None` when no origin set one.
    pub(crate) fn layered_scope(&self, scope: &str) -> Option<Layered<'_>> {
        self.styles
            .get_key_value(scope)
            .map(|(scope, layers)| Self::layer(scope, layers))
    }

    fn layer<'a>(scope: &'a str, layers: &'a ByOrigin<Option<StyleSpec>>) -> Layered<'a> {
        let user = ConfigOrigin::User;
        Layered {
            scope,
            above_theme: layers.get(user).as_ref(),
            below_theme: layers
                .iter()
                .filter(|(origin, _)| *origin != user)
                .find_map(|(_, spec)| spec.as_ref()),
        }
    }
}
