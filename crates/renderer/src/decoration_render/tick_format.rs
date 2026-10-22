//! Numeric tick formatting: decimals, the decimal, scientific, radian and
//! timestamp forms, and the power form with its superscript exponent.

use super::*;

/// Minimum shared decimal count that keeps adjacent ticks distinguishable.
/// Only the step's magnitude matters: an auto-fit step such as 1.312815 must
/// not force six decimals when the label style requests three significant
/// digits. A decimal rounding quantum no larger than the step prevents two
/// adjacent ticks from collapsing to the same label.
pub(crate) fn decimals_from_spacing(spacing: f64) -> usize {
    if !spacing.is_finite() || spacing <= 0.0 {
        return 0;
    }
    (-spacing.log10().floor()).clamp(0.0, 15.0) as usize
}

/// Decimal places needed to make `significant_digits` observable without
/// mixing a different precision at every tick. The largest magnitude in the
/// label set chooses one shared decimal count; spacing may raise it further so
/// adjacent values can never collapse to the same string.
pub(super) fn decimals_from_significant_digits(
    sig_digits: u8,
    range_min: f64,
    range_max: f64,
) -> usize {
    let sig = sig_digits.clamp(1, 15) as i32;
    let max_abs = range_min.abs().max(range_max.abs());
    if !max_abs.is_finite() || max_abs == 0.0 {
        return sig.saturating_sub(1) as usize;
    }
    let order = max_abs.log10().floor() as i32;
    (sig - 1 - order).clamp(0, 15) as usize
}

pub(crate) fn format_tick_value(
    value: f64,
    format: &LabelFormat,
    sig_digits: u8,
    scale: &AxisScale,
    major_spacing: f64,
    range_min: f64,
    range_max: f64,
) -> String {
    let sig = sig_digits.clamp(1, 15) as usize;
    if matches!(scale, AxisScale::Logarithmic) {
        if value == 0.0 {
            return "0".into();
        }
        return match format {
            // Logarithmic ticks span orders of magnitude. Round each value to
            // the requested precision, then omit padding on exact decades.
            // Scientific rounding also preserves tiny positive values that a
            // fixed decimal-place cap would turn into zero.
            LabelFormat::Decimal | LabelFormat::Power => format!("{:.*e}", sig - 1, value)
                .parse::<f64>()
                .ok()
                .filter(|rounded| rounded.is_finite())
                .unwrap_or(value)
                .to_string(),
            LabelFormat::Scientific => format!("{:.*e}", sig - 1, value),
            LabelFormat::Timestamp(_) => format!("{}", value),
            LabelFormat::Degrees => format!("{}°", value),
            LabelFormat::Radians => format_radians(value, sig),
        };
    }

    match format {
        LabelFormat::Scientific => format!("{:.*e}", sig.saturating_sub(1), value),
        LabelFormat::Timestamp(cfg) => crate::time_axis::format_timestamp_numeric_tick(
            value,
            cfg,
            value - major_spacing,
            value + major_spacing,
            major_spacing,
        )
        .unwrap_or_else(|| format!("{}", value)),
        // Power uses the RichText path; if we end up here, treat as Decimal.
        // One uniform form for the whole axis/level set. `significant_digits`
        // controls requested precision while spacing is the correctness floor:
        // a 0.5 level interval must never format both neighbours as integers.
        LabelFormat::Decimal | LabelFormat::Power | LabelFormat::Degrees => {
            let text = if value == 0.0 {
                "0".to_string()
            } else {
                let decimals = decimals_from_spacing(major_spacing).max(
                    decimals_from_significant_digits(sig_digits, range_min, range_max),
                );
                format!("{:.*}", decimals, value)
            };
            if matches!(format, LabelFormat::Degrees) {
                format!("{text}°")
            } else {
                text
            }
        }
        LabelFormat::Radians => format_radians(value, sig),
    }
}

/// `value` as a multiple of π: `0`, `π`, `-π`, `π/2`, `3π/2`, `0.4π`. Halves
/// and quarters use the fraction form; other multiples a decimal factor
/// rounded to `sig` significant digits.
pub(super) fn format_radians(value: f64, sig: usize) -> String {
    let factor = value / std::f64::consts::PI;
    if factor == 0.0 {
        return "0".into();
    }
    let quarters = factor * 4.0;
    if (quarters - quarters.round()).abs() < 1e-9 {
        let quarters = quarters.round() as i64;
        let sign = if quarters < 0 { "-" } else { "" };
        let quarters = quarters.abs();
        let (numerator, denominator) = if quarters % 4 == 0 {
            (quarters / 4, 1)
        } else if quarters % 2 == 0 {
            (quarters / 2, 2)
        } else {
            (quarters, 4)
        };
        let numerator = if numerator == 1 {
            String::new()
        } else {
            numerator.to_string()
        };
        return if denominator == 1 {
            format!("{sign}{numerator}π")
        } else {
            format!("{sign}{numerator}π/{denominator}")
        };
    }
    let rounded = format!("{:.*e}", sig.saturating_sub(1), factor)
        .parse::<f64>()
        .unwrap_or(factor);
    format!("{rounded}π")
}

// Power format (RichText with superscript exponent).

pub(super) fn trim_trailing_fraction_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let trimmed = s.trim_end_matches('0');
    trimmed.trim_end_matches('.').to_string()
}

/// Decompose `value` into mantissa × 10^exp and produce a RichText with the
/// exponent as superscript. If the mantissa is very close to 1, only "10^exp"
/// is shown (the conventional log major-tick form).
pub(crate) fn format_tick_power(value: f64, sig_digits: u8, ls: &LabelStyle) -> RichText {
    let style_from = |segments: Vec<RichSegment>| RichText {
        segments,
        color: ls.color,
        font_size: ls.font_size,
        font: ls.label_font.clone(),
        align: TextAlign::Left,
    };

    if value == 0.0 {
        return style_from(vec![plain_seg('0')]);
    }

    let abs = value.abs();
    let exp = abs.log10().floor() as i32;
    let mantissa = value / 10f64.powi(exp);
    let mantissa_close_to_one = (mantissa.abs() - 1.0).abs() < 1e-9;

    let mut segs: Vec<RichSegment> = Vec::new();

    if mantissa_close_to_one {
        if mantissa < 0.0 {
            segs.push(plain_seg('-'));
        }
    } else {
        let sig = sig_digits.max(1) as usize;
        let raw = format!("{:.*}", sig.saturating_sub(1), mantissa);
        let trimmed = trim_trailing_fraction_zeros(&raw);
        for c in trimmed.chars() {
            segs.push(plain_seg(c));
        }
        segs.push(plain_seg('×'));
    }

    // The "10".
    segs.push(plain_seg('1'));
    segs.push(plain_seg('0'));

    // Exponent (every char is superscript).
    let exp_str = format!("{}", exp);
    for c in exp_str.chars() {
        segs.push(sup_seg(c));
    }

    style_from(segs)
}
