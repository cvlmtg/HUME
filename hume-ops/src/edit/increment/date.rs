//! Date and time literals: `YYYY-MM-DD` or `YYYY/MM/DD`, optionally followed
//! by a space or `T` and `HH:MM[:SS]`, and a standalone `HH:MM[:SS]`. Every
//! field is zero padded to its fixed width.

use std::ops::Range;

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};

use super::{Sym, ascii_at};
use ClockField::{Hour, Minute, Second};
use DateField::{Day, Month, Year};

#[derive(Clone, Copy)]
enum Shape {
    Date,
    DateTime { seconds: bool },
    Time { seconds: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Date(DateField),
    Clock(ClockField),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DateField {
    Year,
    Month,
    Day,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClockField {
    Hour,
    Minute,
    Second,
}

/// Each field of a shape and the index within the literal it starts at,
/// largest field first.
const DATE: [(Field, usize); 3] = [
    (Field::Date(Year), 0),
    (Field::Date(Month), 5),
    (Field::Date(Day), 8),
];
const DATE_TIME: [(Field, usize); 6] = [
    (Field::Date(Year), 0),
    (Field::Date(Month), 5),
    (Field::Date(Day), 8),
    (Field::Clock(Hour), 11),
    (Field::Clock(Minute), 14),
    (Field::Clock(Second), 17),
];
const TIME: [(Field, usize); 3] = [
    (Field::Clock(Hour), 0),
    (Field::Clock(Minute), 3),
    (Field::Clock(Second), 6),
];

impl Field {
    /// How many clusters the field spans.
    fn width(self) -> usize {
        match self {
            Self::Date(Year) => 4,
            _ => 2,
        }
    }

    /// The field of `at`, zero padded to its width.
    fn text(self, at: NaiveDateTime) -> String {
        match self {
            Self::Date(Year) => format!("{:04}", at.year()),
            Self::Date(Month) => format!("{:02}", at.month()),
            Self::Date(Day) => format!("{:02}", at.day()),
            Self::Clock(Hour) => format!("{:02}", at.hour()),
            Self::Clock(Minute) => format!("{:02}", at.minute()),
            Self::Clock(Second) => format!("{:02}", at.second()),
        }
    }
}

#[derive(Clone)]
pub(super) struct Moment {
    shape: Shape,
    /// A standalone time's date is never read.
    at: NaiveDateTime,
}

/// The date or time starting at `syms[i]`, and the index just past it.
pub(super) fn scan(syms: &[Sym], i: usize) -> Option<(Moment, usize)> {
    date_at(syms, i).or_else(|| time_at(syms, i))
}

fn is_digit(syms: &[Sym], k: usize) -> bool {
    ascii_at(syms, k).is_some_and(|c| c.is_ascii_digit())
}

/// The `n` digits from `syms[i]` on, as a number.
fn fixed(syms: &[Sym], i: usize, n: usize) -> Option<u32> {
    (i..i + n).try_fold(0, |acc, k| {
        ascii_at(syms, k)?.to_digit(10).map(|d| acc * 10 + d)
    })
}

/// A date not inside a longer run of digits. A valid time after it joins
/// it; an invalid one leaves the date on its own.
fn date_at(syms: &[Sym], i: usize) -> Option<(Moment, usize)> {
    if i > 0 && is_digit(syms, i - 1) {
        return None;
    }
    let year = fixed(syms, i, 4)?;
    let sep = ascii_at(syms, i + 4).filter(|c| matches!(c, '-' | '/'))?;
    let month = fixed(syms, i + 5, 2)?;
    if ascii_at(syms, i + 7) != Some(sep) {
        return None;
    }
    let day = fixed(syms, i + 8, 2)?;
    let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day)?;
    let end = i + 10;
    if ascii_at(syms, end).is_some_and(|c| matches!(c, ' ' | 'T'))
        && let Some((time, seconds, time_end)) = clock(syms, end + 1)
    {
        let moment = Moment {
            shape: Shape::DateTime { seconds },
            at: date.and_time(time),
        };
        return Some((moment, time_end));
    }
    if is_digit(syms, end) {
        return None;
    }
    let moment = Moment {
        shape: Shape::Date,
        at: date.and_time(NaiveTime::MIN),
    };
    Some((moment, end))
}

/// A standalone time. One after a digit or a `:` is part of a longer run
/// (`1:23:45`), not a time.
fn time_at(syms: &[Sym], i: usize) -> Option<(Moment, usize)> {
    if i > 0 && (is_digit(syms, i - 1) || ascii_at(syms, i - 1) == Some(':')) {
        return None;
    }
    let (time, seconds, end) = clock(syms, i)?;
    let moment = Moment {
        shape: Shape::Time { seconds },
        at: NaiveDate::default().and_time(time),
    };
    Some((moment, end))
}

/// `HH:MM[:SS]` from `syms[i]` on, whether it has seconds, and the index
/// just past it. `None` when a digit or a `:` follows it.
fn clock(syms: &[Sym], i: usize) -> Option<(NaiveTime, bool, usize)> {
    let hour = fixed(syms, i, 2)?;
    if ascii_at(syms, i + 2) != Some(':') {
        return None;
    }
    let minute = fixed(syms, i + 3, 2)?;
    let (second, end) = match (ascii_at(syms, i + 5), fixed(syms, i + 6, 2)) {
        (Some(':'), Some(second)) => (Some(second), i + 8),
        _ => (None, i + 5),
    };
    if is_digit(syms, end) || ascii_at(syms, end) == Some(':') {
        return None;
    }
    let time = NaiveTime::from_hms_opt(hour, minute, second.unwrap_or(0))?;
    Some((time, second.is_some(), end))
}

impl Moment {
    /// The literal's fields and the index each starts at, largest first.
    fn layout(&self) -> &'static [(Field, usize)] {
        match self.shape {
            Shape::Date => &DATE,
            Shape::DateTime { seconds: true } => &DATE_TIME,
            Shape::DateTime { seconds: false } => &DATE_TIME[..5],
            Shape::Time { seconds: true } => &TIME,
            Shape::Time { seconds: false } => &TIME[..2],
        }
    }

    /// Where each field lies, as indices into the literal.
    pub(super) fn parts(&self) -> impl Iterator<Item = Range<usize>> {
        self.layout()
            .iter()
            .map(|&(field, start)| start..start + field.width())
    }

    /// The index in [`Self::parts`] of the field holding index `touched` of
    /// the literal, a separator counting as the field before it. With no
    /// index, the day of a date and the minute of a time.
    pub(super) fn part_at(&self, touched: Option<usize>) -> usize {
        let layout = self.layout();
        let default = match self.shape {
            Shape::Time { .. } => Field::Clock(Minute),
            Shape::Date | Shape::DateTime { .. } => Field::Date(Day),
        };
        touched
            .and_then(|at| layout.iter().rposition(|&(_, start)| start <= at))
            .or_else(|| layout.iter().position(|&(field, _)| field == default))
            .expect("every layout holds its default field")
    }

    /// The text of each of [`Self::parts`] once the fields at `parts` are
    /// moved by `amount`, largest field first, so a month step clamps the day
    /// before a day step moves it.
    pub(super) fn stepped(&self, parts: &[usize], amount: i128) -> Option<Vec<String>> {
        let layout = self.layout();
        let mut parts = parts.to_vec();
        parts.sort_unstable();
        parts.dedup();
        let at = parts
            .into_iter()
            .try_fold(self.at, |at, part| self.step(at, layout[part].0, amount))?;
        (0..=9999)
            .contains(&at.year())
            .then(|| layout.iter().map(|&(field, _)| field.text(at)).collect())
    }

    /// `at` with `field` moved by `amount`. A day or clock step carries into
    /// the larger fields, a month or year step clamps the day to its month,
    /// and a standalone time wraps around midnight.
    fn step(&self, at: NaiveDateTime, field: Field, amount: i128) -> Option<NaiveDateTime> {
        match field {
            Field::Date(Year) => add_months(at, amount.checked_mul(12)?),
            Field::Date(Month) => add_months(at, amount),
            Field::Date(Day) => {
                at.checked_add_signed(TimeDelta::try_days(i64::try_from(amount).ok()?)?)
            }
            Field::Clock(clock) => {
                let n = i64::try_from(amount).ok()?;
                let delta = match clock {
                    Hour => TimeDelta::try_hours(n),
                    Minute => TimeDelta::try_minutes(n),
                    Second => TimeDelta::try_seconds(n),
                }?;
                match self.shape {
                    Shape::Time { .. } => {
                        let (time, _) = at.time().overflowing_add_signed(delta);
                        Some(at.date().and_time(time))
                    }
                    Shape::Date | Shape::DateTime { .. } => at.checked_add_signed(delta),
                }
            }
        }
    }
}

/// `at` moved by `months`, the day clamped to the new month.
fn add_months(at: NaiveDateTime, months: i128) -> Option<NaiveDateTime> {
    let count = Months::new(u32::try_from(months.unsigned_abs()).ok()?);
    if months >= 0 {
        at.checked_add_months(count)
    } else {
        at.checked_sub_months(count)
    }
}
