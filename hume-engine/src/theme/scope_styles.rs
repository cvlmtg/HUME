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
    styles: BTreeMap<Box<str>, ScopeEntry>,
    /// The scopes with an origin flagged unshown, in the order first set.
    unshown: Vec<Box<str>>,
}

#[derive(Debug, Default)]
struct ScopeEntry {
    specs: ByOrigin<Option<StyleSpec>>,
    /// The origins that set a style since [`ScopeStyles::mark_shown`].
    unshown: ByOrigin<bool>,
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
        let entry = match self.styles.get_mut(scope) {
            Some(entry) => entry,
            None => self.styles.entry(scope.into()).or_default(),
        };
        if !entry.unshown.iter().any(|(_, unshown)| *unshown) {
            self.unshown.push(scope.into());
        }
        *entry.unshown.get_mut(by) = true;
        entry.specs.set(by, Some(spec))
    }

    pub(crate) fn layered(&self) -> impl Iterator<Item = Layered<'_>> {
        self.styles
            .iter()
            .map(|(scope, entry)| Self::layer(scope, entry, |_| true))
    }

    /// The scopes set since [`Self::mark_shown`], in the order first set,
    /// each with only the styles that changed what it shows: a style a
    /// higher origin hides changes nothing.
    pub(crate) fn unshown(&self) -> impl Iterator<Item = Layered<'_>> {
        self.unshown.iter().filter_map(|scope| {
            let (scope, entry) = self.styles.get_key_value(scope)?;
            Some(Self::layer(scope, entry, |origin| {
                *entry.unshown.get(origin)
            }))
        })
    }

    /// Whether a scope was set since [`Self::mark_shown`].
    pub fn has_unshown(&self) -> bool {
        !self.unshown.is_empty()
    }

    /// Records that the theme on screen took every style set so far.
    pub fn mark_shown(&mut self) {
        for scope in self.unshown.drain(..) {
            if let Some(entry) = self.styles.get_mut(&scope) {
                entry.unshown = ByOrigin::default();
            }
        }
    }

    /// `entry`'s highest style above the theme and beneath it, each kept
    /// only when `keep` accepts the origin that set it.
    fn layer<'a>(
        scope: &'a str,
        entry: &'a ScopeEntry,
        keep: impl Fn(ConfigOrigin) -> bool,
    ) -> Layered<'a> {
        let user = ConfigOrigin::User;
        let below = entry
            .specs
            .iter()
            .filter(|(origin, _)| *origin != user)
            .find_map(|(origin, spec)| spec.as_ref().map(|spec| (origin, spec)));
        Layered {
            scope,
            above_theme: entry.specs.get(user).as_ref().filter(|_| keep(user)),
            below_theme: below
                .filter(|(origin, _)| keep(*origin))
                .map(|(_, spec)| spec),
        }
    }
}
