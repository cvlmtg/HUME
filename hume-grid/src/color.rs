/// A 24-bit true colour.
///
/// There is deliberately no palette-index or named-colour variant. HUME
/// requires true-colour terminals (see the project's terminal-compatibility
/// rule) and its theme loader only ever produces hex values, so a second
/// variant would exist purely to be unreachable — and it would cost
/// something real: with one variant [`Rgb::lerp`] is total, where a blend
/// over a colour enum has to pass non-RGB values through unchanged and so
/// silently skips the effect it was asked for.
///
/// "No colour" is spelled `Option<Rgb>` rather than a variant of this type,
/// so the same `None` reads as "inherit" in a style cascade and "the
/// terminal's own default" in a cell — see [`ResolvedStyle`](crate::ResolvedStyle).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Blend toward `target` by `factor` — `0.0` leaves `self` unchanged,
    /// `1.0` returns `target`.
    ///
    /// Used for the dim applied to every cell of a non-focused pane.
    pub fn lerp(self, target: Rgb, factor: f32) -> Rgb {
        let mix = |c: u8, t: u8| (c as f32 + (t as f32 - c as f32) * factor).round() as u8;
        Rgb(
            mix(self.0, target.0),
            mix(self.1, target.1),
            mix(self.2, target.2),
        )
    }
}

#[cfg(test)]
mod tests;
