//! Integer literals: decimal with an optional sign, or `0x`/`0o`/`0b`
//! prefixed and unsigned, each with optional `_` digit groups.

use super::{Sym, ascii_at};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Radix {
    Binary,
    Octal,
    Decimal,
    Hex,
}

impl Radix {
    fn base(self) -> u32 {
        match self {
            Self::Binary => 2,
            Self::Octal => 8,
            Self::Decimal => 10,
            Self::Hex => 16,
        }
    }
}

#[derive(Clone)]
pub(super) struct Number {
    radix: Radix,
    /// The literal's own `0x`/`0X`/`0o`/... spelling; empty for decimal.
    prefix: String,
    negative: bool,
    /// Whether the number stops at zero: a prefixed one, or one whose minus
    /// sign would read back as a separator.
    unsigned: bool,
    /// The digits, without `_`.
    digits: String,
    /// The width of the last `_` group, when the literal has groups.
    group: Option<usize>,
    /// Whether a hex digit is uppercase, which uppercases the result's.
    upper: bool,
}

/// Whether `syms[i]` is a minus sign: a `-` before a digit, not after the
/// end of an operand. After one (`x-1`, `1-1`, `f(x)-1`) it separates.
pub(super) fn is_sign(syms: &[Sym], i: usize) -> bool {
    ascii_at(syms, i) == Some('-')
        && ascii_at(syms, i + 1).is_some_and(|c| c.is_ascii_digit())
        && !ends_operand(syms, i)
}

/// Whether the cluster before `syms[i]` ends an operand: a word char or a
/// closing bracket.
fn ends_operand(syms: &[Sym], i: usize) -> bool {
    i.checked_sub(1)
        .and_then(|k| syms.get(k))
        .is_some_and(|s| s.word || matches!(s.ascii, Some(')' | ']' | '}')))
}

/// The number starting at `syms[i]`, and the index just past it.
pub(super) fn scan(syms: &[Sym], i: usize) -> Option<(Number, usize)> {
    prefixed(syms, i).or_else(|| decimal(syms, i))
}

/// A prefixed number at `syms[i]`. Both of its ends sit at a word boundary,
/// so `a0x1` and `0xfg` are not one.
pub(super) fn prefixed(syms: &[Sym], i: usize) -> Option<(Number, usize)> {
    if (i > 0 && syms[i - 1].word) || ascii_at(syms, i) != Some('0') {
        return None;
    }
    let letter = ascii_at(syms, i + 1)?;
    let radix = match letter.to_ascii_lowercase() {
        'b' => Radix::Binary,
        'o' => Radix::Octal,
        'x' => Radix::Hex,
        _ => return None,
    };
    let (digits, group, end) = digit_run(syms, i + 2, radix)?;
    if syms.get(end).is_some_and(|s| s.word) {
        return None;
    }
    let upper = digits.chars().any(|c| c.is_ascii_uppercase());
    let number = Number {
        radix,
        prefix: format!("0{letter}"),
        negative: false,
        unsigned: true,
        digits,
        group,
        upper,
    };
    Some((number, end))
}

fn decimal(syms: &[Sym], i: usize) -> Option<(Number, usize)> {
    let negative = is_sign(syms, i);
    let from = if negative { i + 1 } else { i };
    let (digits, group, end) = digit_run(syms, from, Radix::Decimal)?;
    let number = Number {
        radix: Radix::Decimal,
        prefix: String::new(),
        negative,
        unsigned: !negative && ends_operand(syms, i),
        digits,
        group,
        upper: false,
    };
    Some((number, end))
}

/// The digits from `syms[from]` on, a `_` allowed only between two digits,
/// with the width of the last group when there was a `_`, and the index just
/// past the last digit. `None` when `syms[from]` is not a digit.
fn digit_run(syms: &[Sym], from: usize, radix: Radix) -> Option<(String, Option<usize>, usize)> {
    let digit = |k: usize| ascii_at(syms, k).filter(|c| c.is_digit(radix.base()));
    digit(from)?;
    let mut digits = String::new();
    let mut has_groups = false;
    let mut since_underscore = 0;
    let mut k = from;
    loop {
        if let Some(c) = digit(k) {
            digits.push(c);
            since_underscore += 1;
            k += 1;
        } else if ascii_at(syms, k) == Some('_') && digit(k + 1).is_some() {
            has_groups = true;
            since_underscore = 0;
            k += 1;
        } else {
            break;
        }
    }
    Some((digits, has_groups.then_some(since_underscore), k))
}

impl Number {
    /// The literal moved by `amount`, keeping its prefix, its width when it
    /// has leading zeros, its digit case and its group width. An unsigned
    /// number stops at zero.
    pub(super) fn stepped(&self, amount: i128) -> Option<String> {
        let (negative, magnitude) = if self.unsigned {
            let value = u128::from_str_radix(&self.digits, self.radix.base()).ok()?;
            let stepped = if amount >= 0 {
                value.checked_add(amount.unsigned_abs())?
            } else {
                value.saturating_sub(amount.unsigned_abs())
            };
            (false, stepped)
        } else {
            let value = self.digits.parse::<i128>().ok()?;
            let value = if self.negative { -value } else { value };
            let stepped = value.checked_add(amount)?;
            (stepped < 0, stepped.unsigned_abs())
        };
        let width = if self.digits.len() > 1 && self.digits.starts_with('0') {
            self.digits.len()
        } else {
            0
        };
        let digits = match self.radix {
            Radix::Binary => format!("{magnitude:0width$b}"),
            Radix::Octal => format!("{magnitude:0width$o}"),
            Radix::Decimal => format!("{magnitude:0width$}"),
            Radix::Hex if self.upper => format!("{magnitude:0width$X}"),
            Radix::Hex => format!("{magnitude:0width$x}"),
        };
        let digits = match self.group {
            Some(size) => grouped(&digits, size),
            None => digits,
        };
        let sign = if negative { "-" } else { "" };
        Some(format!("{sign}{}{digits}", self.prefix))
    }
}

/// `digits` with a `_` before every `size` digits counted from the right.
fn grouped(digits: &str, size: usize) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / size);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(size) {
            out.push('_');
        }
        out.push(c);
    }
    out
}
