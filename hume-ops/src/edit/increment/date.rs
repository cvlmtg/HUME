//! Date and time literals: `YYYY-MM-DD` or `YYYY/MM/DD`, optionally followed
//! by a space or `T` and `HH:MM[:SS]`, and a standalone `HH:MM[:SS]`. Every
//! field is zero padded to its fixed width.

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};

use super::{Sym, ascii_at};

#[derive(Clone, Copy)]
enum Shape {
    Date {
        sep: char,
    },
    DateTime {
        sep: char,
        joiner: char,
        seconds: bool,
    },
    Time {
        seconds: bool,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Field {
    Date(DateField),
    Clock(ClockField),
}

#[derive(Clone, Copy)]
pub(super) enum DateField {
    Year,
    Month,
    Day,
}

#[derive(Clone, Copy)]
pub(super) enum ClockField {
    Hour,
    Minute,
    Second,
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
    if let Some(joiner) = ascii_at(syms, end).filter(|c| matches!(c, ' ' | 'T'))
        && let Some((time, seconds, time_end)) = clock(syms, end + 1)
    {
        let shape = Shape::DateTime {
            sep,
            joiner,
            seconds,
        };
        return Some((
            Moment {
                shape,
                at: date.and_time(time),
            },
            time_end,
        ));
    }
    if is_digit(syms, end) {
        return None;
    }
    let moment = Moment {
        shape: Shape::Date { sep },
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
    /// The field holding index `touched` of the literal, a separator counting
    /// as the field before it. With no index, the day of a date and the
    /// minute of a time.
    pub(super) fn field_at(&self, touched: Option<usize>) -> Field {
        use ClockField::{Hour, Minute, Second};
        use DateField::{Day, Month, Year};
        let fields: &[(usize, Field)] = match self.shape {
            Shape::Date { .. } => &[
                (0, Field::Date(Year)),
                (5, Field::Date(Month)),
                (8, Field::Date(Day)),
            ],
            Shape::DateTime { .. } => &[
                (0, Field::Date(Year)),
                (5, Field::Date(Month)),
                (8, Field::Date(Day)),
                (11, Field::Clock(Hour)),
                (14, Field::Clock(Minute)),
                (17, Field::Clock(Second)),
            ],
            Shape::Time { .. } => &[
                (0, Field::Clock(Hour)),
                (3, Field::Clock(Minute)),
                (6, Field::Clock(Second)),
            ],
        };
        let default = match self.shape {
            Shape::Time { .. } => Field::Clock(Minute),
            Shape::Date { .. } | Shape::DateTime { .. } => Field::Date(Day),
        };
        touched
            .and_then(|at| fields.iter().rev().find(|(start, _)| *start <= at))
            .map_or(default, |&(_, field)| field)
    }

    /// The literal with `field` moved by `amount`. A day or clock step
    /// carries into the larger fields, a month or year step clamps the day
    /// to its month, and a standalone time wraps around midnight.
    pub(super) fn stepped(&self, field: Field, amount: i128) -> Option<String> {
        let at = match field {
            Field::Date(DateField::Year) => add_months(self.at, amount.checked_mul(12)?),
            Field::Date(DateField::Month) => add_months(self.at, amount),
            Field::Date(DateField::Day) => self
                .at
                .checked_add_signed(TimeDelta::try_days(i64::try_from(amount).ok()?)?),
            Field::Clock(clock) => {
                let n = i64::try_from(amount).ok()?;
                let delta = match clock {
                    ClockField::Hour => TimeDelta::try_hours(n),
                    ClockField::Minute => TimeDelta::try_minutes(n),
                    ClockField::Second => TimeDelta::try_seconds(n),
                }?;
                match self.shape {
                    Shape::Time { .. } => {
                        let (time, _) = self.at.time().overflowing_add_signed(delta);
                        Some(self.at.date().and_time(time))
                    }
                    Shape::Date { .. } | Shape::DateTime { .. } => {
                        self.at.checked_add_signed(delta)
                    }
                }
            }
        }?;
        (0..=9999).contains(&at.year()).then(|| self.format(at))
    }

    fn format(&self, at: NaiveDateTime) -> String {
        let date =
            |sep: char| format!("{:04}{sep}{:02}{sep}{:02}", at.year(), at.month(), at.day());
        let clock = |seconds: bool| {
            if seconds {
                format!("{:02}:{:02}:{:02}", at.hour(), at.minute(), at.second())
            } else {
                format!("{:02}:{:02}", at.hour(), at.minute())
            }
        };
        match self.shape {
            Shape::Date { sep } => date(sep),
            Shape::DateTime {
                sep,
                joiner,
                seconds,
            } => format!("{}{joiner}{}", date(sep), clock(seconds)),
            Shape::Time { seconds } => clock(seconds),
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
