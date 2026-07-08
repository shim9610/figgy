use crate::config::{AxisOptions, AxisScale};
use crate::format::{
    FractionalSecondDigits, LabelFormat, TimestampLabelFormat, TimestampLabelMode,
    TimestampTickPolicy, TimestampUnit, TimestampZone,
};

const NS_PER_SECOND: i128 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const MAX_ABS_SECONDS: f64 = 31_556_952_000_000.0; // about one million years
const MAX_TIME_TICKS: usize = 1_000;

#[derive(Debug, Clone)]
pub(crate) struct TimestampTick {
    pub value: f64,
    pub label: String,
}

#[derive(Debug, Clone)]
pub(crate) struct TimestampTickPlan {
    pub majors: Vec<TimestampTick>,
    pub minor_values: Vec<f64>,
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

pub(crate) fn timestamp_format(axis: &AxisOptions) -> Option<&TimestampLabelFormat> {
    match &axis.label_style.format {
        LabelFormat::Timestamp(cfg) => Some(cfg),
        _ => None,
    }
}

pub(crate) fn uses_calendar_ticks(axis: &AxisOptions, cfg: &TimestampLabelFormat) -> bool {
    matches!(axis.scale, AxisScale::Linear)
        && matches!(cfg.tick_policy, TimestampTickPolicy::AutoCalendar)
}

pub(crate) fn build_timestamp_tick_plan<F>(
    axis: &AxisOptions,
    axis_len_px: f32,
    min_gap_px: f32,
    mut measure_extent_px: F,
) -> Option<TimestampTickPlan>
where
    F: FnMut(&str) -> f32,
{
    let cfg = timestamp_format(axis)?;
    if !uses_calendar_ticks(axis, cfg) || !walkable_linear_range(axis) {
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
            majors: ticks,
            minor_values: Vec::new(),
        });
        if count <= 1 || required <= axis_len_px.max(1.0) {
            return last_valid;
        }
    }

    last_valid
}

pub(crate) fn format_timestamp_numeric_tick(
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
        let value = value_from_local_parts(year, month, 1, 0, 0, 0, 0, cfg)?;
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
        let value = value_from_local_parts(year, 1, 1, 0, 0, 0, 0, cfg)?;
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
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
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

fn value_from_local_parts(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
    cfg: &TimestampLabelFormat,
) -> Option<f64> {
    let days = days_from_civil(year, month, day)?;
    let local_seconds = days as i128 * SECONDS_PER_DAY as i128
        + hour as i128 * 3_600
        + minute as i128 * 60
        + second as i128;
    let utc_seconds = local_seconds.checked_sub(zone_offset_seconds(cfg.timezone) as i128)?;
    let epoch_ns = utc_seconds
        .checked_mul(NS_PER_SECOND)?
        .checked_add(nanosecond as i128)?;
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

fn walkable_linear_range(axis: &AxisOptions) -> bool {
    axis.min.is_finite() && axis.max.is_finite() && axis.max > axis.min
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
    use crate::default::default_config;
    use crate::format::{TimestampLabelFormat, TimestampLabelMode};

    #[test]
    fn unix_seconds_format_utc_and_offset() {
        let utc = TimestampLabelFormat::default();
        let text = format_timestamp_numeric_tick(0.0, &utc, 0.0, 10.0, 1.0).unwrap();
        assert_eq!(text, "00:00:00");

        let kst = TimestampLabelFormat {
            timezone: TimestampZone::FixedOffsetMinutes(540),
            label: TimestampLabelMode::Pattern("%Y-%m-%d %H:%M:%S".into()),
            ..TimestampLabelFormat::default()
        };
        let text = format_timestamp_numeric_tick(0.0, &kst, 0.0, 10.0, 1.0).unwrap();
        assert_eq!(text, "1970-01-01 09:00:00");
    }

    #[test]
    fn subsecond_labels_choose_fraction_digits() {
        let cfg = TimestampLabelFormat::default();
        let text = format_timestamp_numeric_tick(1.25, &cfg, 1.0, 2.0, 0.05).unwrap();
        assert_eq!(text, "00:00:01.25");
    }

    #[test]
    fn calendar_plan_respects_measured_extent() {
        let mut axis = default_config().bottom_x;
        axis.min = 0.0;
        axis.max = 3_600.0;
        axis.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat::default());
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
            ..TimestampLabelFormat::default()
        };
        let jan_15 = value_from_local_parts(2026, 1, 15, 0, 0, 0, 0, &cfg).unwrap();
        let may_15 = value_from_local_parts(2026, 5, 15, 0, 0, 0, 0, &cfg).unwrap();
        let mut axis = default_config().bottom_x;
        axis.min = jan_15;
        axis.max = may_15;
        axis.label_style.format = LabelFormat::Timestamp(cfg);
        let plan = build_timestamp_tick_plan(&axis, 300.0, 8.0, |s| s.len() as f32 * 6.0)
            .expect("timestamp plan");
        let labels: Vec<_> = plan.majors.iter().map(|tick| tick.label.as_str()).collect();
        assert!(labels.contains(&"2026-02-01"), "{labels:?}");
        assert!(labels.contains(&"2026-05-01"), "{labels:?}");
    }
}
