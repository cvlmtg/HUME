//! A list with at least one item.

/// A list with at least one item: what a menu or drawer is built from, so
/// a selection index always names a row, and what an edit is built from,
/// so an empty edit list never reaches the code that applies one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T>(Vec<T>);

impl<T> NonEmpty<T> {
    /// `items`, or `None` when it is empty.
    pub fn new(items: Vec<T>) -> Option<Self> {
        (!items.is_empty()).then_some(Self(items))
    }

    pub fn first(&self) -> &T {
        &self.0[0]
    }

    pub fn into_vec(self) -> Vec<T> {
        self.0
    }

    pub fn map<U>(self, f: impl FnMut(T) -> U) -> NonEmpty<U> {
        NonEmpty(self.0.into_iter().map(f).collect())
    }

    /// [`Self::map`] over the items by reference.
    pub fn map_ref<'a, U>(&'a self, f: impl FnMut(&'a T) -> U) -> NonEmpty<U> {
        NonEmpty(self.0.iter().map(f).collect())
    }
}

/// Read-only: no `DerefMut`, so nothing can empty the list.
impl<T> std::ops::Deref for NonEmpty<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}
