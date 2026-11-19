use crate::config::{AxisOptions, AxisScale};
use crate::format::{
    CalendarStep, FractionalSecondDigits, LabelFormat, TimestampLabelFormat, TimestampLabelMode,
    TimestampTickPolicy, TimestampUnit, TimestampZone,
};

const NS_PER_SECOND: i128 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const MAX_ABS_SECONDS: f64 = 31_556_952_000_000.0; // about one million years
const MAX_TIME_TICKS: usize = 1_000;

#[derive(Debug, Clone)]
pub struct TimestampTick {
    pub value: f64,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct TimestampTickPlan {
    pub majors: Vec<TimestampTick>,
    pub minor_values: Vec<f64>,
    /// The labels, each at the value it is drawn at: a major's own value,
    /// or with `period_labels` the visible centre of its period (#357).
    /// An empty string draws nothing (a custom label's period with no
    /// string).
    pub labels: Vec<TimestampTick>,
}

#[derive(Debug, Clone, Copy)]
enum TimeStep {
    FixedNs(i128),
    Months(i32),
    Years(i32),
}

#[derive(Debug, Clone, Copy)]
enum AutoLabelKind {
    Fraction(u8),
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    step: TimeStep,
    kind: AutoLabelKind,
}

#[derive(Debug, Clone, Copy)]
struct DateTimeParts {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

pub fn timestamp_format(axis: &AxisOptions) -> Option<&TimestampLabelFormat> {
    match &axis.label_style.format {
        LabelFormat::Timestamp(cfg) => Some(cfg),
        _ => None,
    }
}

pub fn uses_calendar_ticks(axis: &AxisOptions, cfg: &TimestampLabelFormat) -> bool {
    matches!(axis.scale, AxisScale::Linear)
        && matches!(cfg.tick_policy, TimestampTickPolicy::AutoCalendar)
}

pub fn build_timestamp_tick_plan<F>(
    axis: &AxisOptions,
    axis_len_px: f32,
    min_gap_px: f32,
    mut measure_extent_px: F,
) -> Option<TimestampTickPlan>
where
    F: FnMut(&str) -> f32,
{
    let cfg = timestamp_format(axis)?;
    if let Some(step) = cfg.step {
        if !matches!(axis.scale, AxisScale::Linear) || !crate::tick::walkable_range(axis) {
            return None;
        }
        return forced_step_plan(axis, cfg, step);
    }
    if !uses_calendar_ticks(axis, cfg) || !crate::tick::walkable_range(axis) {
        return None;
    }

    let start = parts_from_value(axis.min, cfg)?;
    let end = parts_from_value(axis.max, cfg)?;
    let mut last_valid = None;

    for candidate in candidates() {
        let ticks = ticks_for_candidate(axis, cfg, candidate, start, end)?;
        if ticks.is_empty() {
            continue;
        }
        let count = ticks.len();
        if count > MAX_TIME_TICKS {
            continue;
        }

        let max_extent = ticks
            .iter()
            .map(|tick| measure_extent_px(&tick.label))
            .fold(0.0_f32, f32::max);
        let required = max_extent * count as f32 + min_gap_px * count.saturating_sub(1) as f32;
        last_valid = Some(TimestampTickPlan {
            labels: ticks.clone(),
            majors: ticks,
            minor_values: Vec::new(),
        });
        if count <= 1 || required <= axis_len_px.max(1.0) {
            return last_valid;
        }
    }

    last_valid
}

pub fn format_timestamp_numeric_tick(
    value: f64,
    cfg: &TimestampLabelFormat,
    axis_min: f64,
    axis_max: f64,
    major_spacing: f64,
) -> Option<String> {
    let start = parts_from_value(axis_min, cfg)?;
    let end = parts_from_value(axis_max, cfg)?;
    let seconds = (major_spacing.abs() * seconds_per_unit(cfg.unit)).max(0.0);
    let kind = label_kind_from_seconds(seconds, cfg);
    format_timestamp_with_kind(value, cfg, kind, start, end)
}

fn ticks_for_candidate(
    axis: &AxisOptions,
    cfg: &TimestampLabelFormat,
    candidate: Candidate,
    start: DateTimeParts,
    end: DateTimeParts,
) -> Option<Vec<TimestampTick>> {
    let values = match candidate.step {
        TimeStep::FixedNs(step) => fixed_step_values(axis.min, axis.max, cfg, step)?,
        TimeStep::Months(step) => calendar_month_values(axis.min, axis.max, cfg, step)?,
        TimeStep::Years(step) => calendar_year_values(axis.min, axis.max, cfg, step)?,
    };

    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let label = format_timestamp_with_kind(value, cfg, candidate.kind, start, end)?;
        out.push(TimestampTick { value, label });
    }
    Some(out)
}

fn candidates() -> Vec<Candidate> {
    use AutoLabelKind::*;
    use TimeStep::*;

    let fixed = |ns, kind| Candidate {
        step: FixedNs(ns),
        kind,
    };
    let seconds = |s: i128, kind| fixed(s * NS_PER_SECOND, kind);
    let minutes = |m: i128, kind| seconds(m * 60, kind);
    let hours = |h: i128, kind| minutes(h * 60, kind);
    let days = |d: i128, kind| hours(d * 24, kind);

    vec![
        fixed(1_000, Fraction(6)),
        fixed(10_000, Fraction(5)),
        fixed(100_000, Fraction(4)),
        fixed(1_000_000, Fraction(3)),
        fixed(2_000_000, Fraction(3)),
        fixed(5_000_000, Fraction(3)),
        fixed(10_000_000, Fraction(2)),
        fixed(20_000_000, Fraction(2)),
        fixed(50_000_000, Fraction(2)),
        fixed(100_000_000, Fraction(1)),
        fixed(200_000_000, Fraction(1)),
        fixed(500_000_000, Fraction(1)),
        seconds(1, Second),
        seconds(2, Second),
        seconds(5, Second),
        seconds(10, Second),
        seconds(15, Second),
        seconds(30, Second),
        minutes(1, Minute),
        minutes(2, Minute),
        minutes(5, Minute),
        minutes(10, Minute),
        minutes(15, Minute),
        minutes(30, Minute),
        hours(1, Hour),
        hours(2, Hour),
        hours(3, Hour),
        hours(6, Hour),
        hours(12, Hour),
        days(1, Day),
        days(2, Day),
        days(7, Day),
        days(14, Day),
        Candidate {
            step: Months(1),
            kind: Month,
        },
        Candidate {
            step: Months(3),
            kind: Month,
        },
        Candidate {
            step: Months(6),
            kind: Month,
        },
        Candidate {
            step: Years(1),
            kind: Year,
        },
        Candidate {
            step: Years(2),
            kind: Year,
        },
        Candidate {
            step: Years(5),
            kind: Year,
        },
        Candidate {
            step: Years(10),
            kind: Year,
        },
        Candidate {
            step: Years(20),
            kind: Year,
        },
        Candidate {
            step: Years(50),
            kind: Year,
        },
        Candidate {
            step: Years(100),
            kind: Year,
        },
        Candidate {
            step: Years(200),
            kind: Year,
        },
        Candidate {
            step: Years(500),
            kind: Year,
        },
        Candidate {
            step: Years(1000),
            kind: Year,
        },
    ]
}

fn fixed_step_values(
    min: f64,
    max: f64,
    cfg: &TimestampLabelFormat,
    step_ns: i128,
) -> Option<Vec<f64>> {
    if step_ns <= 0 {
        return None;
    }
    let min_ns = epoch_ns_from_value(min, cfg.unit)?;
    let max_ns = epoch_ns_from_value(max, cfg.unit)?;
    let first = div_ceil_i128(min_ns, step_ns) * step_ns;
    let last = div_floor_i128(max_ns, step_ns) * step_ns;
    if last < first {
        return Some(Vec::new());
    }
    let count_i128 = (last - first) / step_ns + 1;
    if count_i128 > MAX_TIME_TICKS as i128 {
        return Some(Vec::new());
    }
    let count = count_i128 as usize;

    let mut out = Vec::with_capacity(count);
    let mut cur = first;
    while cur <= last {
        out.push(value_from_epoch_ns(cur, cfg.unit));
        cur += step_ns;
    }
    Some(out)
}

fn calendar_month_values(
    min: f64,
    max: f64,
    cfg: &TimestampLabelFormat,
    step_months: i32,
) -> Option<Vec<f64>> {
    if step_months <= 0 {
        return None;
    }
    let min_parts = parts_from_value(min, cfg)?;
    let first_month = aligned_total_month(min_parts.year, min_parts.month, step_months);
    let mut total_month = first_month;
    let mut out = Vec::new();
    loop {
        let (year, month) = year_month_from_total(total_month)?;
        let value = value_from_local_parts(
            DateTimeParts {
                year,
                month,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                nanosecond: 0,
            },
            cfg,
        )?;
        if value > max + range_epsilon(min, max) {
            break;
        }
        if value >= min - range_epsilon(min, max) {
            out.push(value);
            if out.len() > MAX_TIME_TICKS {
                break;
            }
        }
        total_month = total_month.checked_add(step_months)?;
    }
    Some(out)
}

fn calendar_year_values(
    min: f64,
    max: f64,
    cfg: &TimestampLabelFormat,
    step_years: i32,
) -> Option<Vec<f64>> {
    if step_years <= 0 {
        return None;
    }
    let min_parts = parts_from_value(min, cfg)?;
    let mut year = div_ceil_i32(min_parts.year, step_years) * step_years;
    let mut out = Vec::new();
    loop {
        let value = value_from_local_parts(
            DateTimeParts {
                year,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                nanosecond: 0,
            },
            cfg,
        )?;
        if value > max + range_epsilon(min, max) {
            break;
        }
        if value >= min - range_epsilon(min, max) {
            out.push(value);
            if out.len() > MAX_TIME_TICKS {
                break;
            }
        }
        year = year.checked_add(step_years)?;
    }
    Some(out)
}

fn format_timestamp_with_kind(
    value: f64,
    cfg: &TimestampLabelFormat,
    kind: AutoLabelKind,
    start: DateTimeParts,
    end: DateTimeParts,
) -> Option<String> {
    let parts = parts_from_value(value, cfg)?;
    let digits = fractional_digits(cfg, kind);
    match &cfg.label {
        TimestampLabelMode::Pattern(pattern) => Some(format_custom_pattern(pattern, parts, digits)),
        TimestampLabelMode::Auto => Some(format_auto(parts, kind, start, end, digits)),
        // Counted in the format's step (#357): a format without one has
        // no period to count and prints nothing (validation refuses it).
        TimestampLabelMode::Relative { .. } | TimestampLabelMode::Custom { .. } => {
            period_text(value, cfg)
        }
    }
}

fn label_kind_from_seconds(seconds: f64, cfg: &TimestampLabelFormat) -> AutoLabelKind {
    if seconds < 1.0 {
        let ns = (seconds * NS_PER_SECOND as f64).max(1.0).round() as i128;
        return AutoLabelKind::Fraction(fractional_digits_from_step_ns(ns));
    }
    if seconds < 60.0 {
        return AutoLabelKind::Second;
    }
    if seconds < 3_600.0 {
        return AutoLabelKind::Minute;
    }
    if seconds < 86_400.0 {
        return AutoLabelKind::Hour;
    }
    if seconds < 28.0 * 86_400.0 {
        return AutoLabelKind::Day;
    }
    if seconds < 365.0 * 86_400.0 {
        return AutoLabelKind::Month;
    }
    match cfg.fractional {
        FractionalSecondDigits::Fixed(d) if d > 0 => AutoLabelKind::Fraction(d.min(9)),
        _ => AutoLabelKind::Year,
    }
}

fn fractional_digits(cfg: &TimestampLabelFormat, kind: AutoLabelKind) -> u8 {
    match cfg.fractional {
        FractionalSecondDigits::Fixed(d) => d.min(9),
        FractionalSecondDigits::Auto => match kind {
            AutoLabelKind::Fraction(d) => d.min(9),
            _ => 0,
        },
    }
}

fn fractional_digits_from_step_ns(step_ns: i128) -> u8 {
    for digits in 0..=9 {
        let scale = 10_i128.pow(9 - digits as u32);
        if step_ns % scale == 0 {
            return digits;
        }
    }
    9
}

fn format_auto(
    parts: DateTimeParts,
    kind: AutoLabelKind,
    start: DateTimeParts,
    end: DateTimeParts,
    digits: u8,
) -> String {
    let same_year = start.year == end.year;
    let same_day = same_year && start.month == end.month && start.day == end.day;
    match kind {
        AutoLabelKind::Year => format!("{:04}", parts.year),
        AutoLabelKind::Month => format!("{:04}-{:02}", parts.year, parts.month),
        AutoLabelKind::Day => {
            if same_year {
                format!("{:02}-{:02}", parts.month, parts.day)
            } else {
                format!("{:04}-{:02}-{:02}", parts.year, parts.month, parts.day)
            }
        }
        AutoLabelKind::Hour | AutoLabelKind::Minute => {
            if same_day {
                format!("{:02}:{:02}", parts.hour, parts.minute)
            } else if same_year {
                format!(
                    "{:02}-{:02} {:02}:{:02}",
                    parts.month, parts.day, parts.hour, parts.minute
                )
            } else {
                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}",
                    parts.year, parts.month, parts.day, parts.hour, parts.minute
                )
            }
        }
        AutoLabelKind::Second => format_time_with_seconds(parts, start, end, None),
        AutoLabelKind::Fraction(_) => format_time_with_seconds(parts, start, end, Some(digits)),
    }
}

fn format_time_with_seconds(
    parts: DateTimeParts,
    start: DateTimeParts,
    end: DateTimeParts,
    digits: Option<u8>,
) -> String {
    let same_year = start.year == end.year;
    let same_day = same_year && start.month == end.month && start.day == end.day;
    let tail = match digits {
        Some(d) if d > 0 => format!(
            "{:02}:{:02}:{:02}.{}",
            parts.hour,
            parts.minute,
            parts.second,
            fraction_text(parts.nanosecond, d)
        ),
        _ => format!("{:02}:{:02}:{:02}", parts.hour, parts.minute, parts.second),
    };
    if same_day {
        tail
    } else if same_year {
        format!("{:02}-{:02} {tail}", parts.month, parts.day)
    } else {
        format!(
            "{:04}-{:02}-{:02} {tail}",
            parts.year, parts.month, parts.day
        )
    }
}

fn format_custom_pattern(pattern: &str, parts: DateTimeParts, digits: u8) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('Y') => out.push_str(&format!("{:04}", parts.year)),
            Some('m') => out.push_str(&format!("{:02}", parts.month)),
            Some('d') => out.push_str(&format!("{:02}", parts.day)),
            Some('H') => out.push_str(&format!("{:02}", parts.hour)),
            Some('M') => out.push_str(&format!("{:02}", parts.minute)),
            Some('S') => out.push_str(&format!("{:02}", parts.second)),
            Some('f') => out.push_str(&fraction_text(parts.nanosecond, digits)),
            Some('b') => out.push_str(MONTH_NAMES[(parts.month as usize).clamp(1, 12) - 1]),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// The English three-letter month names `%b` prints (#357).
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ── Calendar steps (#357) ───────────────────────────────────────────────
//
// A forced step numbers its periods with one integer each, in the format's
// time zone: a day by its local date (days since 1970-01-01), a week by its
// week-start date, a month by `12 · year + month0`, a quarter by its first
// month over three, a year by itself. A period's start is local midnight on
// its first day; the ticks are the starts inside the range.

/// The weekday of a local date (days since 1970-01-01, a Thursday), Monday
/// 0 to Sunday 6.
fn weekday_of(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

/// The index of the period of `step` holding the local date `days`.
fn period_of_days(step: CalendarStep, days: i64) -> i64 {
    let (year, month, _) = civil_from_days(days);
    let total_month = year as i64 * 12 + (month as i64 - 1);
    match step {
        CalendarStep::Day => days,
        CalendarStep::Week { start } => {
            // The date of the week's start, counted in weeks.
            div_floor_i64(
                days - (weekday_of(days) - start.index()).rem_euclid(7) + 3,
                7,
            )
        }
        CalendarStep::Month => total_month,
        CalendarStep::Quarter => div_floor_i64(total_month, 3),
        CalendarStep::Year => year as i64,
    }
}

/// The local date (days since 1970-01-01) the period `p` of `step` starts
/// on.
fn period_start_days(step: CalendarStep, p: i64) -> Option<i64> {
    let month_start = |total_month: i64| {
        let year = i32::try_from(div_floor_i64(total_month, 12)).ok()?;
        let month0 = total_month.rem_euclid(12);
        days_from_civil(year, month0 as u8 + 1, 1)
    };
    match step {
        CalendarStep::Day => Some(p),
        // Week `p` starts on the date `7p - 3`, moved forward to `start`.
        CalendarStep::Week { start } => {
            let anchor = p.checked_mul(7)?.checked_sub(3)?;
            Some(anchor + (start.index() - weekday_of(anchor)).rem_euclid(7))
        }
        CalendarStep::Month => month_start(p),
        CalendarStep::Quarter => month_start(p.checked_mul(3)?),
        CalendarStep::Year => days_from_civil(i32::try_from(p).ok()?, 1, 1),
    }
}

/// The local date of `value` in `cfg`'s time zone.
fn local_days(value: f64, cfg: &TimestampLabelFormat) -> Option<i64> {
    let parts = parts_from_value(value, cfg)?;
    days_from_civil(parts.year, parts.month, parts.day)
}

/// The value at which period `p` of `step` starts: local midnight.
fn period_start_value(step: CalendarStep, p: i64, cfg: &TimestampLabelFormat) -> Option<f64> {
    let (year, month, day) = civil_from_days(period_start_days(step, p)?);
    value_from_local_parts(
        DateTimeParts {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
            second: 0,
            nanosecond: 0,
        },
        cfg,
    )
}

/// The period of `step` holding `value`.
fn period_of_value(step: CalendarStep, value: f64, cfg: &TimestampLabelFormat) -> Option<i64> {
    Some(period_of_days(step, local_days(value, cfg)?))
}

/// The label kind of an automatic label at a step's boundaries.
fn step_label_kind(step: CalendarStep) -> AutoLabelKind {
    match step {
        CalendarStep::Day | CalendarStep::Week { .. } => AutoLabelKind::Day,
        CalendarStep::Month | CalendarStep::Quarter => AutoLabelKind::Month,
        CalendarStep::Year => AutoLabelKind::Year,
    }
}

/// The text of a relative or custom label at the period starting at
/// `value` (#357): `n` whole periods of the format's step from the period
/// holding the origin. `None` without a step, or for a custom label whose
/// period has no string.
fn period_text(value: f64, cfg: &TimestampLabelFormat) -> Option<String> {
    let step = cfg.step?;
    let origin = |origin: i64| period_of_value(step, origin as f64, cfg);
    let n = |o: i64| Some(period_of_value(step, value, cfg)? - origin(o)?);
    match &cfg.label {
        TimestampLabelMode::Relative { origin, prefix } => Some(format!("{prefix}{}", n(*origin)?)),
        TimestampLabelMode::Custom { origin, labels } => {
            let n = usize::try_from(n(*origin)?).ok()?;
            labels.get(n).cloned()
        }
        TimestampLabelMode::Auto | TimestampLabelMode::Pattern(_) => None,
    }
}

/// The plan of a forced calendar step (#357): a major at every period
/// start inside the axis range and a label per period, on its start or
/// centred on its part inside the range (`period_labels`). Like the
/// planner, a range holding more than [`MAX_TIME_TICKS`] periods draws no
/// ticks: a tick plan is not a data count.
fn forced_step_plan(
    axis: &AxisOptions,
    cfg: &TimestampLabelFormat,
    step: CalendarStep,
) -> Option<TimestampTickPlan> {
    let (min, max) = (axis.min, axis.max);
    let eps = range_epsilon(min, max);
    let first = period_of_value(step, min, cfg)?;
    let last = period_of_value(step, max, cfg)?;
    let empty = TimestampTickPlan {
        majors: Vec::new(),
        minor_values: Vec::new(),
        labels: Vec::new(),
    };
    let periods = last.checked_sub(first)?.checked_add(1)?;
    if periods > MAX_TIME_TICKS as i64 {
        return Some(empty);
    }
    let start = parts_from_value(min, cfg)?;
    let end = parts_from_value(max, cfg)?;
    let kind = step_label_kind(step);
    let mut plan = empty;
    for p in first..=last {
        let at = period_start_value(step, p, cfg)?;
        let next = period_start_value(step, p.checked_add(1)?, cfg)?;
        let text = format_timestamp_with_kind(at, cfg, kind, start, end).unwrap_or_default();
        let on_tick = at >= min - eps && at <= max + eps;
        if on_tick {
            plan.majors.push(TimestampTick {
                value: at,
                label: text.clone(),
            });
        }
        let label_at = if cfg.period_labels {
            let (lo, hi) = (at.max(min), next.min(max));
            (hi > lo + eps).then_some(0.5 * (lo + hi))
        } else {
            on_tick.then_some(at)
        };
        if let Some(value) = label_at {
            plan.labels.push(TimestampTick { value, label: text });
        }
    }
    Some(plan)
}

fn fraction_text(nanosecond: u32, digits: u8) -> String {
    if digits == 0 {
        return String::new();
    }
    let full = format!("{:09}", nanosecond);
    full[..digits as usize].to_string()
}

fn parts_from_value(value: f64, cfg: &TimestampLabelFormat) -> Option<DateTimeParts> {
    let epoch_ns = epoch_ns_from_value(value, cfg.unit)?;
    parts_from_epoch_ns(epoch_ns, cfg.timezone)
}

fn parts_from_epoch_ns(epoch_ns: i128, zone: TimestampZone) -> Option<DateTimeParts> {
    let offset_ns = zone_offset_seconds(zone) as i128 * NS_PER_SECOND;
    let local_ns = epoch_ns.checked_add(offset_ns)?;
    let local_seconds_i128 = div_floor_i128(local_ns, NS_PER_SECOND);
    let nanosecond = (local_ns - local_seconds_i128 * NS_PER_SECOND) as u32;
    if local_seconds_i128 < i64::MIN as i128 || local_seconds_i128 > i64::MAX as i128 {
        return None;
    }
    let local_seconds = local_seconds_i128 as i64;
    let days = div_floor_i64(local_seconds, SECONDS_PER_DAY);
    let sec_of_day = local_seconds - days * SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    Some(DateTimeParts {
        year,
        month,
        day,
        hour: (sec_of_day / 3_600) as u8,
        minute: ((sec_of_day % 3_600) / 60) as u8,
        second: (sec_of_day % 60) as u8,
        nanosecond,
    })
}

fn value_from_local_parts(parts: DateTimeParts, cfg: &TimestampLabelFormat) -> Option<f64> {
    let days = days_from_civil(parts.year, parts.month, parts.day)?;
    let local_seconds = days as i128 * SECONDS_PER_DAY as i128
        + parts.hour as i128 * 3_600
        + parts.minute as i128 * 60
        + parts.second as i128;
    let utc_seconds = local_seconds.checked_sub(zone_offset_seconds(cfg.timezone) as i128)?;
    let epoch_ns = utc_seconds
        .checked_mul(NS_PER_SECOND)?
        .checked_add(parts.nanosecond as i128)?;
    Some(value_from_epoch_ns(epoch_ns, cfg.unit))
}

fn epoch_ns_from_value(value: f64, unit: TimestampUnit) -> Option<i128> {
    if !value.is_finite() {
        return None;
    }
    let seconds = value * seconds_per_unit(unit);
    if !seconds.is_finite() || seconds.abs() > MAX_ABS_SECONDS {
        return None;
    }
    Some((seconds * NS_PER_SECOND as f64).round() as i128)
}

fn value_from_epoch_ns(epoch_ns: i128, unit: TimestampUnit) -> f64 {
    (epoch_ns as f64 / NS_PER_SECOND as f64) / seconds_per_unit(unit)
}

fn seconds_per_unit(unit: TimestampUnit) -> f64 {
    match unit {
        TimestampUnit::Seconds => 1.0,
        TimestampUnit::Milliseconds => 1.0e-3,
        TimestampUnit::Microseconds => 1.0e-6,
        TimestampUnit::Nanoseconds => 1.0e-9,
    }
}

fn zone_offset_seconds(zone: TimestampZone) -> i64 {
    match zone {
        TimestampZone::Utc => 0,
        TimestampZone::FixedOffsetMinutes(minutes) => minutes as i64 * 60,
    }
}

fn range_epsilon(min: f64, max: f64) -> f64 {
    ((max - min).abs() * 1.0e-12).max(1.0e-9)
}

fn aligned_total_month(year: i32, month: u8, step_months: i32) -> i32 {
    let total = year * 12 + (month as i32 - 1);
    div_ceil_i32(total, step_months) * step_months
}

fn year_month_from_total(total_month: i32) -> Option<(i32, u8)> {
    let year = div_floor_i32(total_month, 12);
    let month0 = total_month - year * 12;
    Some((year, (month0 + 1) as u8))
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u8, u8) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u8, d as u8)
}

fn days_from_civil(year: i32, month: u8, day: u8) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month as i64 + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn div_floor_i64(a: i64, b: i64) -> i64 {
    let q = a / b;
    let r = a % b;
    if r != 0 && ((r > 0) != (b > 0)) {
        q - 1
    } else {
        q
    }
}

fn div_floor_i128(a: i128, b: i128) -> i128 {
    let q = a / b;
    let r = a % b;
    if r != 0 && ((r > 0) != (b > 0)) {
        q - 1
    } else {
        q
    }
}

fn div_ceil_i128(a: i128, b: i128) -> i128 {
    -div_floor_i128(-a, b)
}

fn div_floor_i32(a: i32, b: i32) -> i32 {
    let q = a / b;
    let r = a % b;
    if r != 0 && ((r > 0) != (b > 0)) {
        q - 1
    } else {
        q
    }
}

fn div_ceil_i32(a: i32, b: i32) -> i32 {
    -div_floor_i32(-a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{TimestampLabelFormat, TimestampLabelMode};

    #[test]
    fn unix_seconds_format_utc_and_offset() {
        let utc = crate::defaults::timestamp_label_format();
        let text = format_timestamp_numeric_tick(0.0, &utc, 0.0, 10.0, 1.0).unwrap();
        assert_eq!(text, "00:00:00");

        let kst = TimestampLabelFormat {
            timezone: TimestampZone::FixedOffsetMinutes(540),
            label: TimestampLabelMode::Pattern("%Y-%m-%d %H:%M:%S".into()),
            ..crate::defaults::timestamp_label_format()
        };
        let text = format_timestamp_numeric_tick(0.0, &kst, 0.0, 10.0, 1.0).unwrap();
        assert_eq!(text, "1970-01-01 09:00:00");
    }

    #[test]
    fn subsecond_labels_choose_fraction_digits() {
        let cfg = crate::defaults::timestamp_label_format();
        let text = format_timestamp_numeric_tick(1.25, &cfg, 1.0, 2.0, 0.05).unwrap();
        assert_eq!(text, "00:00:01.25");
    }

    #[test]
    fn calendar_plan_respects_measured_extent() {
        let mut axis = crate::defaults::config().bottom_x;
        axis.min = 0.0;
        axis.max = 3_600.0;
        axis.label_style.format = LabelFormat::Timestamp(crate::defaults::timestamp_label_format());
        let plan = build_timestamp_tick_plan(&axis, 200.0, 8.0, |s| s.len() as f32 * 8.0)
            .expect("timestamp plan");
        assert!(
            plan.majors.len() <= 4,
            "planner should coarsen dense labels: {:?}",
            plan.majors
        );
        assert!(plan.majors.iter().any(|tick| tick.label.contains(':')));
    }

    #[test]
    fn month_ticks_follow_calendar_boundaries() {
        let cfg = TimestampLabelFormat {
            label: TimestampLabelMode::Pattern("%Y-%m-%d".into()),
            ..crate::defaults::timestamp_label_format()
        };
        let jan_15 = value_from_local_parts(
            DateTimeParts {
                year: 2026,
                month: 1,
                day: 15,
                hour: 0,
                minute: 0,
                second: 0,
                nanosecond: 0,
            },
            &cfg,
        )
        .unwrap();
        let may_15 = value_from_local_parts(
            DateTimeParts {
                year: 2026,
                month: 5,
                day: 15,
                hour: 0,
                minute: 0,
                second: 0,
                nanosecond: 0,
            },
            &cfg,
        )
        .unwrap();
        let mut axis = crate::defaults::config().bottom_x;
        axis.min = jan_15;
        axis.max = may_15;
        axis.label_style.format = LabelFormat::Timestamp(cfg);
        let plan = build_timestamp_tick_plan(&axis, 300.0, 8.0, |s| s.len() as f32 * 6.0)
            .expect("timestamp plan");
        let labels: Vec<_> = plan.majors.iter().map(|tick| tick.label.as_str()).collect();
        assert!(labels.contains(&"2026-02-01"), "{labels:?}");
        assert!(labels.contains(&"2026-05-01"), "{labels:?}");
    }

    // ── Calendar steps (#357) ───────────────────────────────────────────

    use crate::format::{CalendarStep, Weekday};

    /// Unix seconds of a UTC date at midnight.
    fn utc(year: i32, month: u8, day: u8) -> f64 {
        (days_from_civil(year, month, day).unwrap() * SECONDS_PER_DAY) as f64
    }

    fn calendar(
        step: CalendarStep,
        label: TimestampLabelMode,
        period_labels: bool,
    ) -> TimestampLabelFormat {
        TimestampLabelFormat {
            label,
            step: Some(step),
            period_labels,
            ..crate::defaults::timestamp_label_format()
        }
    }

    fn axis(min: f64, max: f64, format: TimestampLabelFormat) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_x();
        axis.min = min;
        axis.max = max;
        axis.label_style.format = LabelFormat::Timestamp(format);
        axis
    }

    fn plan(axis: &AxisOptions) -> TimestampTickPlan {
        // Labels measure wide: a forced step keeps every boundary anyway.
        build_timestamp_tick_plan(axis, 100.0, 8.0, |_| 1000.0).unwrap()
    }

    fn texts(ticks: &[TimestampTick]) -> Vec<&str> {
        ticks.iter().map(|t| t.label.as_str()).collect()
    }

    /// A month header: a tick at each 1st inside the range, `%b %Y`, and
    /// with period labels one label per month centred on its part of the
    /// range, the partial months at both ends included.
    #[test]
    fn a_month_step_ticks_every_first_and_centres_its_periods() {
        let (min, max) = (utc(2026, 3, 11), utc(2026, 6, 21));
        let pattern = TimestampLabelMode::Pattern("%b %Y".into());
        let on_ticks = plan(&axis(
            min,
            max,
            calendar(CalendarStep::Month, pattern.clone(), false),
        ));
        let firsts = [utc(2026, 4, 1), utc(2026, 5, 1), utc(2026, 6, 1)];
        assert_eq!(
            on_ticks.majors.iter().map(|t| t.value).collect::<Vec<_>>(),
            firsts
        );
        assert_eq!(
            texts(&on_ticks.labels),
            ["Apr 2026", "May 2026", "Jun 2026"]
        );
        let periods = plan(&axis(
            min,
            max,
            calendar(CalendarStep::Month, pattern, true),
        ));
        assert_eq!(
            periods.majors.iter().map(|t| t.value).collect::<Vec<_>>(),
            firsts
        );
        assert_eq!(
            texts(&periods.labels),
            ["Mar 2026", "Apr 2026", "May 2026", "Jun 2026"]
        );
        let at: Vec<f64> = periods.labels.iter().map(|t| t.value).collect();
        assert_eq!(at[0], 0.5 * (min + utc(2026, 4, 1)));
        assert_eq!(at[1], 0.5 * (utc(2026, 4, 1) + utc(2026, 5, 1)));
        assert_eq!(at[3], 0.5 * (utc(2026, 6, 1) + max));
    }

    /// Weeks start on the chosen weekday at local midnight, in UTC and in a
    /// fixed offset; days, quarters and years walk their boundaries.
    #[test]
    fn every_step_walks_its_local_boundaries() {
        let auto = TimestampLabelMode::Auto;
        let (min, max) = (utc(2026, 3, 1), utc(2026, 3, 31));
        for start in [Weekday::Monday, Weekday::Thursday, Weekday::Sunday] {
            for zone in [TimestampZone::Utc, TimestampZone::FixedOffsetMinutes(540)] {
                let mut format = calendar(CalendarStep::Week { start }, auto.clone(), false);
                format.timezone = zone;
                let majors = plan(&axis(min, max, format.clone())).majors;
                assert!(majors.len() >= 4, "{start:?} {zone:?}");
                for tick in &majors {
                    let parts = parts_from_value(tick.value, &format).unwrap();
                    let days = days_from_civil(parts.year, parts.month, parts.day).unwrap();
                    assert_eq!(weekday_of(days), start.index(), "{start:?} {zone:?}");
                    assert_eq!((parts.hour, parts.minute, parts.second), (0, 0, 0));
                }
                let gaps = majors.windows(2).map(|w| w[1].value - w[0].value);
                assert!(gaps.into_iter().all(|g| g == 7.0 * 86_400.0));
            }
        }
        let majors = |step, min, max| {
            plan(&axis(min, max, calendar(step, auto.clone(), false)))
                .majors
                .iter()
                .map(|t| t.value)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            majors(CalendarStep::Day, utc(2026, 2, 27), utc(2026, 3, 2)),
            [
                utc(2026, 2, 27),
                utc(2026, 2, 28),
                utc(2026, 3, 1),
                utc(2026, 3, 2)
            ]
        );
        assert_eq!(
            majors(CalendarStep::Quarter, utc(2025, 12, 5), utc(2026, 10, 2)),
            [
                utc(2026, 1, 1),
                utc(2026, 4, 1),
                utc(2026, 7, 1),
                utc(2026, 10, 1)
            ]
        );
        assert_eq!(
            majors(CalendarStep::Year, utc(2024, 6, 1), utc(2027, 1, 1)),
            [utc(2025, 1, 1), utc(2026, 1, 1), utc(2027, 1, 1)]
        );
    }

    /// Relative labels count whole periods from the origin's period,
    /// negative before it; custom labels look each period up by the same
    /// count, and a period with no string has no label but keeps its tick.
    #[test]
    fn relative_and_custom_labels_count_periods_from_the_origin() {
        let origin = utc(2026, 3, 4) as i64; // a Wednesday: its week starts Mar 2
        let week = CalendarStep::Week {
            start: Weekday::Monday,
        };
        let (min, max) = (utc(2026, 2, 20), utc(2026, 3, 20));
        let relative = TimestampLabelMode::Relative {
            origin,
            prefix: "Week ".into(),
        };
        let p = plan(&axis(min, max, calendar(week, relative, false)));
        assert_eq!(texts(&p.labels), ["Week -1", "Week 0", "Week 1", "Week 2"]);
        let custom = TimestampLabelMode::Custom {
            origin,
            labels: vec!["Kickoff".into(), "Build".into()],
        };
        let p = plan(&axis(min, max, calendar(week, custom, false)));
        assert_eq!(p.majors.len(), 4, "every boundary keeps its tick");
        assert_eq!(texts(&p.labels), ["", "Kickoff", "Build", ""]);
        // Months and days count the same way.
        let months = TimestampLabelMode::Relative {
            origin,
            prefix: "Month ".into(),
        };
        let p = plan(&axis(
            utc(2026, 1, 15),
            utc(2026, 5, 2),
            calendar(CalendarStep::Month, months, true),
        ));
        assert_eq!(
            texts(&p.labels),
            ["Month -2", "Month -1", "Month 0", "Month 1", "Month 2"]
        );
        let days = TimestampLabelMode::Relative {
            origin,
            prefix: "Day ".into(),
        };
        let p = plan(&axis(
            utc(2026, 3, 3),
            utc(2026, 3, 6),
            calendar(CalendarStep::Day, days, false),
        ));
        assert_eq!(texts(&p.labels), ["Day -1", "Day 0", "Day 1", "Day 2"]);
    }

    /// Rule 10: custom labels have no count limit; a long axis reads deep
    /// into a long list.
    #[test]
    fn custom_labels_have_no_count_limit() {
        let origin = utc(2024, 1, 1);
        let labels: Vec<String> = (0..10_000).map(|i| format!("L{i}")).collect();
        let custom = TimestampLabelMode::Custom {
            origin: origin as i64,
            labels,
        };
        let (min, max) = (origin + 9_000.0 * 86_400.0, origin + 9_900.0 * 86_400.0);
        let p = plan(&axis(min, max, calendar(CalendarStep::Day, custom, false)));
        assert_eq!(p.labels.first().map(|t| t.label.as_str()), Some("L9000"));
        assert_eq!(p.labels.last().map(|t| t.label.as_str()), Some("L9900"));
    }

    #[test]
    fn b_prints_the_month_name() {
        let format = TimestampLabelFormat {
            label: TimestampLabelMode::Pattern("%b %d, %Y %%b".into()),
            ..crate::defaults::timestamp_label_format()
        };
        let text =
            format_timestamp_numeric_tick(utc(2026, 11, 19), &format, 0.0, 1.0, 1.0).unwrap();
        assert_eq!(text, "Nov 19, 2026 %b");
    }

    #[test]
    fn relative_custom_and_period_labels_need_a_step() {
        use crate::RuleKind;
        let mut format = crate::defaults::timestamp_label_format();
        assert_eq!(format.validate(), Ok(()));
        format.period_labels = true;
        assert_eq!(format.validate(), Err(RuleKind::TimestampStepRequired));
        format.period_labels = false;
        format.label = TimestampLabelMode::Relative {
            origin: 0,
            prefix: "Day ".into(),
        };
        assert_eq!(format.validate(), Err(RuleKind::TimestampStepRequired));
        format.step = Some(CalendarStep::Day);
        assert_eq!(format.validate(), Ok(()));
        format.label = TimestampLabelMode::Relative {
            origin: 0,
            prefix: "x".repeat(25),
        };
        assert_eq!(format.validate(), Err(RuleKind::TimestampRelativePrefix));
        format.label = TimestampLabelMode::Custom {
            origin: 0,
            labels: vec!["ok".into(), "two\nlines".into()],
        };
        assert_eq!(format.validate(), Err(RuleKind::TimestampCustomLabel));
        // The chart's validation reads it on every axis.
        let mut config = crate::defaults::config();
        config.top_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat {
            period_labels: true,
            ..crate::defaults::timestamp_label_format()
        });
        assert_eq!(
            config.validate_chart().unwrap_err().kind,
            RuleKind::TimestampStepRequired
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn calendar_formats_round_trip_and_older_ones_read_unchanged() {
        let format = calendar(
            CalendarStep::Week {
                start: Weekday::Sunday,
            },
            TimestampLabelMode::Custom {
                origin: 1_772_409_600,
                labels: vec!["Sprint A".into()],
            },
            true,
        );
        let json = serde_json::to_string(&format).unwrap();
        assert_eq!(
            serde_json::from_str::<TimestampLabelFormat>(&json).unwrap(),
            format
        );
        let plain = crate::defaults::timestamp_label_format();
        let json = serde_json::to_string(&plain).unwrap();
        assert!(
            !json.contains("step") && !json.contains("period_labels"),
            "{json}"
        );
        let old: TimestampLabelFormat =
            serde_json::from_str(r#"{"unit":"Seconds","label":{"Pattern":"%m/%d"}}"#).unwrap();
        assert_eq!((old.step, old.period_labels), (None, false));
    }
}
