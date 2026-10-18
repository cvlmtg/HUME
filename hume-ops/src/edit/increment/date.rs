//! Date and time literals: `YYYY-MM-DD` or `YYYY/MM/DD`, optionally followed
//! by a space or `T` and `HH:MM[:SS]`, and a standalone `HH:MM[:SS]`. Every
//! field is zero padded to its fixed width.

use std::ops::Range;

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};

use super::{Sym, ascii_at, is_digit};
use Field::{Day, Hour, Minute, Month, Second, Year};

#[derive(Clone, Copy)]
enum Shape {
    Date,
    DateTime { seconds: bool },
    Time { seconds: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

/// Each field of a shape and the index within the literal it starts at,
/// largest field first. A date is the first three.
const DATE_TIME: [(Field, usize); 6] = [
    (Year, 0),
    (Month, 5),
    (Day, 8),
    (Hour, 11),
    (Minute, 14),
    (Second, 17),
];
const TIME: [(Field, usize); 3] = [(Hour, 0), (Minute, 3), (Second, 6)];

impl Field {
    /// How many clusters the field spans.
    fn width(self) -> usize {
        match self {
            Year => 4,
            _ => 2,
        }
    }

    /// The field of `at`, zero padded to its width.
    fn text(self, at: NaiveDateTime) -> String {
        match self {
            Year => format!("{:04}", at.year()),
            Month => format!("{:02}", at.month()),
            Day => format!("{:02}", at.day()),
            Hour => format!("{:02}", at.hour()),
            Minute => format!("{:02}", at.minute()),
            Second => format!("{:02}", at.second()),
        }
    }
}

pub(super) struct Moment {
    shape: Shape,
    /// A standalone time's date is never read.
    at: NaiveDateTime,
}

/// The date or time starting at `syms[i]`, and the index just past it.
pub(super) fn scan(syms: &[Sym], i: usize) -> Option<(Moment, usize)> {
    date_at(syms, i).or_else(|| time_at(syms, i))
}

/// Whether `syms[k]` is a digit or a `:`, which would make a time beside it
/// part of a longer run (`1:23:45`).
fn extends_run(syms: &[Sym], k: usize) -> bool {
    is_digit(syms, k) || ascii_at(syms, k) == Some(':')
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

/// A standalone time.
fn time_at(syms: &[Sym], i: usize) -> Option<(Moment, usize)> {
    if i > 0 && extends_run(syms, i - 1) {
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
    if extends_run(syms, end) {
        return None;
    }
    let time = NaiveTime::from_hms_opt(hour, minute, second.unwrap_or(0))?;
    Some((time, second.is_some(), end))
}

impl Moment {
    /// The literal's fields and the index each starts at, largest first.
    fn layout(&self) -> &'static [(Field, usize)] {
        match self.shape {
            Shape::Date => &DATE_TIME[..3],
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
            Shape::Time { .. } => Minute,
            Shape::Date | Shape::DateTime { .. } => Day,
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
        let n = i64::try_from(amount).ok();
        let delta = match field {
            Year => return add_months(at, amount.checked_mul(12)?),
            Month => return add_months(at, amount),
            Day => TimeDelta::try_days(n?),
            Hour => TimeDelta::try_hours(n?),
            Minute => TimeDelta::try_minutes(n?),
            Second => TimeDelta::try_seconds(n?),
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

/// `at` moved by `months`, the day clamped to the new month.
fn add_months(at: NaiveDateTime, months: i128) -> Option<NaiveDateTime> {
    let count = Months::new(u32::try_from(months.unsigned_abs()).ok()?);
    if months >= 0 {
        at.checked_add_months(count)
    } else {
        at.checked_sub_months(count)
    }
}
