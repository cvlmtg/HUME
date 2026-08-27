use crate::color::Rgb;
use bitflags::bitflags;

/// A fully-resolved cell style.
///
/// One type end to end: what the theme cascade composes, what a
/// [`Cell`](crate::Cell) stores, and what the emitter turns into SGR
/// parameters. There is no separate "backend style" to convert into, which
/// is what keeps the underline *shape* below reaching the terminal at all —
/// it survives only because nothing downstream narrows this type.
///
/// `None` means two things, which agree. In a cascade
/// (see [`ResolvedStyle::layer`]) it means "inherit whatever is underneath";
/// in a cell it means "the terminal's own default". Those compose because
/// the terminal default *is* the bottom layer of every cascade — so a
/// cascade that resolves to `None` and a cell that was never given a colour
/// are the same state, and one type can carry both without ambiguity.
///
/// A cell holds one of these outright — storing it replaces whatever the
/// cell held before, with nothing implicitly inherited. Composition, where a
/// partial style is resolved against what is already painted, belongs to the
/// drawing layer above (`hume_engine::render::Canvas`), which is the only
/// place that knows the difference between "the caller had no opinion" and
/// "the caller wants the terminal's default".
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ResolvedStyle {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub underline: UnderlineStyle,
    pub underline_color: Option<Rgb>,
    pub modifiers: Modifiers,
}

impl ResolvedStyle {
    /// Layer `over` on top of `self`. Non-None / non-default fields in `over` win.
    /// This is the primitive compositing operation for the style cascade.
    pub fn layer(self, over: ResolvedStyle) -> ResolvedStyle {
        ResolvedStyle {
            fg: over.fg.or(self.fg),
            bg: over.bg.or(self.bg),
            underline: if over.underline != UnderlineStyle::None {
                over.underline
            } else {
                self.underline
            },
            underline_color: over.underline_color.or(self.underline_color),
            modifiers: self.modifiers | over.modifiers,
        }
    }

    /// Compose `self` over `under`: unset colours fall back to `under`'s;
    /// underline shape and modifiers are `self`'s outright.
    ///
    /// The style-cascade counterpart of [`layer`](Self::layer) for
    /// resolving a partial per-write style against whatever a cell already
    /// holds, rather than against another cascade layer — the caller has an
    /// opinion (`self`) that may leave some fields unset, and `under` is
    /// what's already painted there. See
    /// `hume_engine::render::Canvas::over_painted`.
    pub fn over(self, under: ResolvedStyle) -> ResolvedStyle {
        ResolvedStyle {
            fg: self.fg.or(under.fg),
            bg: self.bg.or(under.bg),
            underline_color: self.underline_color.or(under.underline_color),
            ..self
        }
    }

    /// Drop state that cannot be seen, so two styles that render identically
    /// also compare equal.
    ///
    /// An `underline_color` with no underline to paint is the only such case
    /// today: a theme can set one on a scope that doesn't underline, and a
    /// cascade can inherit one past a layer that turned the underline off.
    /// Left in place it would make the diff repaint a cell whose appearance
    /// never changed, and emit an SGR 58 the terminal has nothing to apply
    /// it to. Applied by every [`Cell`](crate::Cell) constructor, so cell
    /// equality is appearance equality.
    pub(crate) fn normalized(mut self) -> ResolvedStyle {
        if self.underline == UnderlineStyle::None {
            self.underline_color = None;
        }
        self
    }
}

/// Underline variants supported by modern terminals.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum UnderlineStyle {
    #[default]
    None,
    Solid,
    Wavy,
    Dotted,
    Dashed,
}

bitflags! {
    /// Text modifiers that compose independently. Mirrors Helix's modifier set
    /// (minus `underlined`, which is tracked via `UnderlineStyle`).
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const STRIKETHROUGH = 0b0000_0100;
        const DIM           = 0b0000_1000;
        const REVERSED      = 0b0001_0000;
        const HIDDEN        = 0b0010_0000;
        const SLOW_BLINK    = 0b0100_0000;
        const RAPID_BLINK   = 0b1000_0000;
    }
}

#[cfg(test)]
mod tests;
