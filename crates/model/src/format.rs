use crate::config::{AxisOptions, AxisScale, Config, LabelStyle};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LabelFormat {
    /// Plain decimal: `1234.56`, `0.003`, `1000`, `10`.
    Decimal,
    /// ASCII scientific: `1.23e3`, `1e-1`.
    Scientific,
    /// Typographic superscript exponent (RichText): `10³`, `1.5×10⁻³`, `10⁻¹`.
    Power,
    /// Unix-epoch timestamp labels. Data values stay numeric; this only
    /// changes tick planning and label text on linear axes.
    Timestamp(TimestampLabelFormat),
    /// Decimal with a degree sign: `90°`, `22.5°`. For an angle axis in
    /// degrees.
    Degrees,
    /// Multiples of π: `π/2`, `π`, `3π/2`, `0.75π`. For an angle axis in
    /// radians.
    Radians,
    /// The value times 100 with a percent sign: `25%`, `0.1%`, `99.9%`
    /// (#209). On a probability axis the decimals follow each value's tail
    /// `min(p, 1 − p)`, so `0.1%`, `50%` and `99.99%` keep the digits that
    /// tell the tail apart; elsewhere they follow the spacing, as `Decimal`.
    Percent,
    /// A plain number that depends on the value alone, not on the axis
    /// scale, spacing or significant digits: `2.5`, `1000`, `1.23e6`, or a
    /// fixed number of decimals. See [`PlainNumberFormat`].
    Plain(PlainNumberFormat),
    /// Clean axis numbers chosen from the axis' tick step (#357): `0.8`, not
    /// `0.800`; `-3`, not `-3.00`; `10`, not `10.0`. Every tick of one axis
    /// gets the decimals its step needs and no more (`0.0`, `0.2`, `0.4`;
    /// `0.00`, `0.25`, `0.50`), and an axis whose range reaches very large
    /// or very small magnitudes switches to an exponent form (`2×10⁹`,
    /// `5×10⁻⁶`). See [`AutoNumberFormat`]. The format the helpers give a new
    /// chart's linear axes; a document never gets it by omission.
    Auto(AutoNumberFormat),
}

/// How [`LabelFormat::Auto`] writes the ticks of one axis (#357).
///
/// On a linear axis the decimals come from the major spacing alone: the
/// fewest decimals that write the step exactly (`0.2` → 1, `0.25` → 2,
/// `2000` → 0), at most two more than the step's first digit needs, so a
/// fitted step such as `1.312815` prints `1.31`. Ticks are multiples of the
/// step, so every tick of the axis is told apart. With
/// [`Self::uniform_decimals`] every tick keeps those decimals (`0.0`,
/// `0.5`, `1.0`, as matplotlib writes them); without it each tick drops its
/// own trailing zeros (`0`, `0.5`, `1`, as Plotly writes them).
///
/// The axis switches to an exponent form when the order of magnitude of its
/// largest end, `floor(log10(max(|min|, |max|)))`, is at most
/// [`Self::exponent_low`] or at least [`Self::exponent_high`]: the limits of
/// matplotlib's `ScalarFormatter` (`axes.formatter.limits`, -5 and 6 by
/// default). Each tick is then written with its own exponent
/// ([`ExponentNotation`], `2×10⁹` by default) and a mantissa with the
/// decimals the step needs at that exponent.
///
/// The ASCII form a string-only caller gets ([`AutoTickText::to_ascii`])
/// writes the power notation as scientific (`2e9`).
///
/// A logarithmic axis writes each tick at the label style's significant
/// digits, plain inside the same window and in the exponent form outside
/// it; a probability axis writes them as [`LabelFormat::Decimal`] does,
/// with the digits each tail needs (#209).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::auto_number_format")
)]
pub struct AutoNumberFormat {
    /// Every tick of the axis keeps the decimals the step needs (`0.0`,
    /// `0.5`, `1.0`); `false` trims each tick's trailing zeros (`0`, `0.5`,
    /// `1`). Default `true`; omitted from JSON when `true`.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "is_true"))]
    pub uniform_decimals: bool,
    /// How a tick is written once the axis switches to an exponent form.
    /// Default [`ExponentNotation::Power`] (`2×10⁹`); omitted from JSON then.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "ExponentNotation::is_power")
    )]
    pub notation: ExponentNotation,
    /// The exponent form is used when the axis' order of magnitude is at
    /// most this. Default `-5`; omitted from JSON then.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "is_default_low"))]
    pub exponent_low: i8,
    /// The exponent form is used when the axis' order of magnitude is at
    /// least this. Default `6`; omitted from JSON then.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "is_default_high"))]
    pub exponent_high: i8,
}

/// The exponent form of [`AutoNumberFormat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExponentNotation {
    /// ASCII scientific with a normalized mantissa: `2e9`, `1.5e-6`.
    Scientific,
    /// ASCII with the exponent a multiple of three: `20e-6`, `1.5e9`.
    Engineering,
    /// Typographic superscript (RichText) with a normalized mantissa:
    /// `2×10⁹`, `10⁻⁶`. A caller that writes plain strings only (contour
    /// level labels) gets the scientific form.
    Power,
}

impl ExponentNotation {
    #[cfg(feature = "serde")]
    fn is_power(&self) -> bool {
        *self == Self::Power
    }
}

#[cfg(feature = "serde")]
fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(feature = "serde")]
fn is_default_low(value: &i8) -> bool {
    *value == AutoNumberFormat::DEFAULT.exponent_low
}

#[cfg(feature = "serde")]
fn is_default_high(value: &i8) -> bool {
    *value == AutoNumberFormat::DEFAULT.exponent_high
}

/// One tick written by [`AutoNumberFormat`]: the number, or its mantissa
/// when `exponent` is set (`mantissa × 10^exponent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoTickText {
    /// The plain number, or the mantissa of the exponent form.
    pub mantissa: String,
    /// The power of ten of the exponent form; `None` for a plain number.
    pub exponent: Option<i32>,
}

impl AutoTickText {
    /// The text in ASCII: `0.8`, `2e9`, `1.5e-6`.
    pub fn to_ascii(&self) -> String {
        match self.exponent {
            None => self.mantissa.clone(),
            Some(exponent) => format!("{}e{exponent}", self.mantissa),
        }
    }
}

/// The largest decimal count [`AutoNumberFormat`] writes.
const AUTO_MAX_DECIMALS: usize = 15;

/// The fewest decimals that write `step` exactly, at most two more than its
/// first significant digit needs, and 0 for a step that is not finite and
/// positive.
fn step_decimals(step: f64) -> usize {
    if !(step.is_finite() && step > 0.0) {
        return 0;
    }
    let first = (-step.log10().floor()).clamp(0.0, AUTO_MAX_DECIMALS as f64) as usize;
    let last = (first + 2).min(AUTO_MAX_DECIMALS);
    (first..=last)
        .find(|&decimals| {
            let scaled = step * 10f64.powi(decimals as i32);
            (scaled - scaled.round()).abs() <= 1e-6 * scaled.max(1.0)
        })
        .unwrap_or(last)
}

/// `value` with `decimals` decimals, trailing zeros trimmed unless
/// `uniform`, and never a negative zero.
fn fixed_text(value: f64, decimals: usize, uniform: bool) -> String {
    let text = format!("{value:.decimals$}");
    let text = if uniform {
        text
    } else {
        trim_fraction_zeros(text)
    };
    match text.strip_prefix('-') {
        Some(rest) if rest.bytes().all(|b| b == b'0' || b == b'.') => rest.to_owned(),
        _ => text,
    }
}

impl AutoNumberFormat {
    /// The default: uniform decimals, typographic exponents (`2×10⁹`), and
    /// the limits -5 and 6.
    pub const DEFAULT: Self = Self {
        uniform_decimals: true,
        notation: ExponentNotation::Power,
        exponent_low: -5,
        exponent_high: 6,
    };

    /// Whether a magnitude of order `oom` is written in the exponent form.
    fn uses_exponent(&self, oom: i32) -> bool {
        oom <= i32::from(self.exponent_low) || oom >= i32::from(self.exponent_high)
    }

    /// The exponent a nonzero finite `value` is written with: its order of
    /// magnitude, rounded down to a multiple of three for engineering.
    fn exponent_of(&self, value: f64) -> i32 {
        let oom = value.abs().log10().floor() as i32;
        match self.notation {
            ExponentNotation::Engineering => oom.div_euclid(3) * 3,
            ExponentNotation::Scientific | ExponentNotation::Power => oom,
        }
    }

    /// `value` as a mantissa over `10^exponent`, with the decimals
    /// `decimals(exponent)` gives; a mantissa that rounds up to the next
    /// exponent moves there.
    fn exponent_text(&self, value: f64, decimals: impl Fn(i32) -> usize) -> AutoTickText {
        let mut exponent = self.exponent_of(value);
        let limit = match self.notation {
            ExponentNotation::Engineering => 1000.0,
            ExponentNotation::Scientific | ExponentNotation::Power => 10.0,
        };
        let step = if limit > 10.0 { 3 } else { 1 };
        let mut mantissa = value / 10f64.powi(exponent);
        let rounded: f64 = format!("{mantissa:.*}", decimals(exponent))
            .parse()
            .unwrap_or(mantissa);
        if rounded.abs() >= limit {
            exponent += step;
            mantissa = value / 10f64.powi(exponent);
        }
        AutoTickText {
            mantissa: fixed_text(mantissa, decimals(exponent), self.uniform_decimals),
            exponent: Some(exponent),
        }
    }

    /// A linear axis' tick `value`, for the axis' major spacing `step` and
    /// its (shown) range `min..max`.
    pub fn linear_tick(&self, value: f64, step: f64, min: f64, max: f64) -> AutoTickText {
        let plain = |text: String| AutoTickText {
            mantissa: text,
            exponent: None,
        };
        if !value.is_finite() {
            return plain(format!("{value}"));
        }
        // A tick walked to a residue of zero (`k · step` rounding) is zero.
        let value = if step.is_finite() && value.abs() < step * 1e-9 {
            0.0
        } else {
            value
        };
        let reach = min.abs().max(max.abs());
        let exponent_form =
            reach.is_finite() && reach > 0.0 && self.uses_exponent(reach.log10().floor() as i32);
        if !exponent_form {
            return plain(fixed_text(
                value,
                step_decimals(step),
                self.uniform_decimals,
            ));
        }
        if value == 0.0 {
            return plain("0".into());
        }
        self.exponent_text(value, |exponent| step_decimals(step / 10f64.powi(exponent)))
    }

    /// A logarithmic axis' tick `value`, rounded to `significant_digits`.
    pub fn log_tick(&self, value: f64, significant_digits: u8) -> AutoTickText {
        let sig = usize::from(significant_digits.clamp(1, 15));
        if !value.is_finite() || value == 0.0 {
            return AutoTickText {
                mantissa: if value == 0.0 {
                    "0".into()
                } else {
                    format!("{value}")
                },
                exponent: None,
            };
        }
        let rounded: f64 = format!("{:.*e}", sig - 1, value).parse().unwrap_or(value);
        let oom = rounded.abs().log10().floor() as i32;
        if !self.uses_exponent(oom) {
            let decimals = (sig as i32 - 1 - oom).clamp(0, AUTO_MAX_DECIMALS as i32) as usize;
            return AutoTickText {
                mantissa: fixed_text(rounded, decimals, false),
                exponent: None,
            };
        }
        let trimmed = Self {
            uniform_decimals: false,
            ..*self
        };
        // The mantissa keeps `sig` digits: fewer decimals the more digits
        // an engineering exponent moves before the point.
        trimmed.exponent_text(rounded, |exponent| {
            (sig as i32 - 1 - (oom - exponent)).clamp(0, AUTO_MAX_DECIMALS as i32) as usize
        })
    }
}

/// How [`LabelFormat::Plain`] writes a number.
///
/// The automatic form ([`Self::AUTO`], `decimals: None`) is the label form of
/// the legacy documents' value axes: `0` for zero, `{:.2e}` (`1.23e6`,
/// `-5.00e-4`, `inf`) when `|v| >= 1e6` or `|v| < 0.001`, and otherwise six
/// decimals with trailing zeros and then the point trimmed (`2.5`, `-0.125`,
/// `1000`). It never prints thousands separators, and the exponent switch
/// tests the value before rounding (`999999.9999996` prints `1000000`).
///
/// `decimals: Some(d)` prints exactly `d` decimals (`{:.d}`, no exponent, so
/// `-0.0` prints `-0` at 0 decimals) and, with `trim_zeros`, drops the
/// trailing fractional zeros and then a bare point. `trim_zeros` has no
/// effect on the automatic form, which always trims.
///
/// `Plain` ignores the axis scale, spacing and significant digits: the text
/// depends on the value alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::plain_number_format")
)]
pub struct PlainNumberFormat {
    /// Fixed decimal places, or `None` for the automatic form.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub decimals: Option<u8>,
    /// With fixed decimals, drop trailing fractional zeros and a bare point.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "std::ops::Not::not"))]
    pub trim_zeros: bool,
}

impl PlainNumberFormat {
    /// The automatic form; also the `Default`.
    pub const AUTO: Self = Self {
        decimals: None,
        trim_zeros: false,
    };

    /// Exactly `decimals` decimals, saturated to the `u8` field: the label
    /// form of the value labels' `decimals`.
    pub(crate) fn fixed(decimals: usize, trim_zeros: bool) -> Self {
        Self {
            decimals: Some(u8::try_from(decimals).unwrap_or(u8::MAX)),
            trim_zeros,
        }
    }

    /// `value` written in this form.
    pub fn format(&self, value: f64) -> String {
        match self.decimals {
            None => {
                if value == 0.0 {
                    return "0".into();
                }
                if value.abs() >= 1e6 || value.abs() < 0.001 {
                    format!("{value:.2e}")
                } else {
                    trim_fraction_zeros(format!("{value:.6}"))
                }
            }
            Some(decimals) => {
                let text = format!("{:.*}", usize::from(decimals), value);
                if self.trim_zeros {
                    trim_fraction_zeros(text)
                } else {
                    text
                }
            }
        }
    }
}

/// `text` without the trailing zeros of its fraction and then a bare point;
/// unchanged when it has no point.
fn trim_fraction_zeros(text: String) -> String {
    if !text.contains('.') {
        return text;
    }
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampUnit {
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampZone {
    Utc,
    /// Fixed offset from UTC in minutes. Korea Standard Time is 540.
    FixedOffsetMinutes(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampLabelMode {
    Auto,
    /// Minimal strftime-like pattern: %Y %m %d %H %M %S %f and %%.
    Pattern(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FractionalSecondDigits {
    Auto,
    Fixed(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampTickPolicy {
    /// Calendar-aware ticks such as minutes, hours, days, months, and years.
    AutoCalendar,
    /// Reuse the numeric `major_spacing` / `minor_count` tick walkers.
    NumericSpacing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::timestamp_label_format")
)]
pub struct TimestampLabelFormat {
    pub unit: TimestampUnit,
    pub timezone: TimestampZone,
    pub label: TimestampLabelMode,
    pub fractional: FractionalSecondDigits,
    pub tick_policy: TimestampTickPolicy,
}

fn auto_sig_linear(min: f64, max: f64, step: f64) -> u8 {
    let max_abs = f64::max(min.abs(), max.abs());
    if max_abs == 0.0 || step <= 0.0 {
        return 1;
    }
    let order_max = max_abs.log10().floor() as i32;
    let order_step = step.log10().floor() as i32;
    // step >= 1: integer-range ticks → total integer digits.
    // step <  1: fractional range → integer digits + fractional digits.
    let sig = if step >= 1.0 {
        order_max + 1
    } else {
        order_max - order_step + 1
    };
    sig.clamp(1, 15) as u8
}

fn auto_sig_log(_min: f64, _max: f64, _major_spacing: f64) -> u8 {
    2
}

impl LabelStyle {
    pub fn compute_auto_significant_digits(
        scale: AxisScale,
        min: f64,
        max: f64,
        major_spacing: f64,
    ) -> u8 {
        match scale {
            AxisScale::Linear => auto_sig_linear(min, max, major_spacing),
            AxisScale::Logarithmic => auto_sig_log(min, max, major_spacing),
            // The decimals follow each value's tail (#209); one digit of it.
            AxisScale::Probability(_) => 1,
        }
    }
}

impl AxisOptions {
    /// Set `label_style.significant_digits` for the range and spacing the
    /// labels print: the shown range with a [`AxisOptions::unit`] (#273).
    pub fn auto_significant_digits(&mut self) {
        let (min, max) = self.shown_range();
        let sig = LabelStyle::compute_auto_significant_digits(
            self.scale.clone(),
            min,
            max,
            self.major_spacing,
        );
        self.label_style.significant_digits = sig.max(1);
    }
}

impl Config {
    pub fn auto_significant_digits_all(&mut self) {
        self.top_x.auto_significant_digits();
        self.bottom_x.auto_significant_digits();
        self.left_y.auto_significant_digits();
        self.right_y.auto_significant_digits();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The spec's examples, and an all-zero range, which falls back to one
    // digit. Every result is in 1..=15.
    #[test]
    fn sig_digits_spec_examples() {
        let cases: &[(f64, f64, f64, u8)] = &[
            (0.0, 1.0, 0.2, 2),
            (0.0, 1000.0, 200.0, 4),
            (0.0, 0.01, 0.002, 2),
            (-5.0, 5.0, 1.0, 1),
            (100.0, 110.0, 2.0, 3),
            (0.0, 0.0, 0.0, 1),
        ];
        for &(min, max, step, expected) in cases {
            let got =
                LabelStyle::compute_auto_significant_digits(AxisScale::Linear, min, max, step);
            assert!((1..=15).contains(&got), "sig out of bounds: {}", got);
            assert_eq!(got, expected, "min={}, max={}, step={}", min, max, step);
        }
    }

    #[test]
    fn log_sig_digits_default_2() {
        let s =
            LabelStyle::compute_auto_significant_digits(AxisScale::Logarithmic, 0.001, 1000.0, 1.0);
        assert_eq!(s, 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn timestamp_format_serde_roundtrip() {
        let fmt = LabelFormat::Timestamp(TimestampLabelFormat {
            unit: TimestampUnit::Milliseconds,
            timezone: TimestampZone::FixedOffsetMinutes(540),
            label: TimestampLabelMode::Pattern("%Y-%m-%d %H:%M:%S.%f".into()),
            fractional: FractionalSecondDigits::Fixed(3),
            tick_policy: TimestampTickPolicy::NumericSpacing,
        });
        let json = serde_json::to_string(&fmt).unwrap();
        let got: LabelFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(got, fmt);
    }

    #[test]
    fn plain_auto_writes_the_bounded_axis_form() {
        for (value, text) in [
            (0.0, "0"),
            (-0.0, "0"),
            (2.5, "2.5"),
            (-0.125, "-0.125"),
            (1000.0, "1000"),
            (0.001, "0.001"),
            (0.000_999_9, "1.00e-3"),
            (0.000_999_999_6, "1.00e-3"),
            (999_999.999_999_6, "1000000"),
            (1e6, "1.00e6"),
            (-5e-4, "-5.00e-4"),
            (1_234_567.0, "1.23e6"),
            (0.123_456_789, "0.123457"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(PlainNumberFormat::AUTO.format(value), text, "{value:?}");
        }
        assert_eq!(
            crate::defaults::plain_number_format(),
            PlainNumberFormat::AUTO
        );
        // `trim_zeros` does not change the automatic form.
        let trimmed = PlainNumberFormat {
            decimals: None,
            trim_zeros: true,
        };
        assert_eq!(trimmed.format(2.5), "2.5");
        assert_eq!(trimmed.format(1e7), "1.00e7");
    }

    #[test]
    fn plain_fixed_decimals_trim_only_when_asked() {
        let fixed = |decimals, trim_zeros| PlainNumberFormat {
            decimals: Some(decimals),
            trim_zeros,
        };
        for (format, value, text) in [
            (fixed(2, false), 2.5, "2.50"),
            (fixed(2, true), 2.5, "2.5"),
            (fixed(2, true), 3.0, "3"),
            (fixed(0, false), 100.0, "100"),
            (fixed(0, true), 100.0, "100"),
            (fixed(1, false), -0.0, "-0.0"),
            (fixed(0, true), -0.0, "-0"),
            (fixed(1, true), -0.04, "-0"),
            (fixed(3, false), 1e7, "10000000.000"),
            (fixed(1, true), f64::NAN, "NaN"),
            (fixed(1, true), f64::INFINITY, "inf"),
        ] {
            assert_eq!(format.format(value), text, "{format:?} {value:?}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn plain_format_serde_writes_only_what_is_set() {
        let auto = LabelFormat::Plain(PlainNumberFormat::AUTO);
        assert_eq!(serde_json::to_string(&auto).unwrap(), r#"{"Plain":{}}"#);
        assert_eq!(
            serde_json::from_str::<LabelFormat>(r#"{"Plain":{}}"#).unwrap(),
            auto
        );
        let fixed = LabelFormat::Plain(PlainNumberFormat {
            decimals: Some(1),
            trim_zeros: true,
        });
        let json = serde_json::to_string(&fixed).unwrap();
        assert_eq!(json, r#"{"Plain":{"decimals":1,"trim_zeros":true}}"#);
        assert_eq!(serde_json::from_str::<LabelFormat>(&json).unwrap(), fixed);
        assert_eq!(
            serde_json::from_str::<LabelFormat>(r#"{"Plain":{"decimals":2}}"#).unwrap(),
            LabelFormat::Plain(PlainNumberFormat {
                decimals: Some(2),
                trim_zeros: false,
            })
        );
    }

    /// Every label of the linear ticks `k · step` for `k` in `ks`, over the
    /// range `min..max`, in `format`.
    fn auto_labels(
        format: AutoNumberFormat,
        step: f64,
        ks: std::ops::RangeInclusive<i32>,
    ) -> Vec<String> {
        let (lo, hi) = (f64::from(*ks.start()) * step, f64::from(*ks.end()) * step);
        ks.map(|k| {
            format
                .linear_tick(f64::from(k) * step, step, lo, hi)
                .to_ascii()
        })
        .collect()
    }

    // #357: the typical tick sets print the decimals their step needs and
    // no more, the same count on every tick of the axis.
    #[test]
    fn auto_linear_ticks_print_the_decimals_the_step_needs() {
        let auto = AutoNumberFormat::DEFAULT;
        assert_eq!(
            auto_labels(auto, 0.2, 0..=5),
            ["0.0", "0.2", "0.4", "0.6", "0.8", "1.0"]
        );
        assert_eq!(
            auto_labels(auto, 0.25, 0..=4),
            ["0.00", "0.25", "0.50", "0.75", "1.00"]
        );
        assert_eq!(
            auto_labels(auto, 1.0, -3..=3),
            ["-3", "-2", "-1", "0", "1", "2", "3"]
        );
        assert_eq!(
            auto_labels(auto, 2000.0, 0..=5),
            ["0", "2000", "4000", "6000", "8000", "10000"]
        );
        assert_eq!(
            auto_labels(auto, 2.0, 0..=5),
            ["0", "2", "4", "6", "8", "10"]
        );
        assert_eq!(
            auto_labels(auto, 0.5, -2..=2),
            ["-1.0", "-0.5", "0.0", "0.5", "1.0"]
        );
        assert_eq!(
            auto_labels(auto, 0.002, -1..=1),
            ["-0.002", "0.000", "0.002"]
        );
        // A step of 1e-4 still prints plain: the window is matplotlib's.
        assert_eq!(
            auto_labels(auto, 1e-4, 0..=2),
            ["0.0000", "0.0001", "0.0002"]
        );
    }

    #[test]
    fn auto_linear_trimmed_form_drops_each_ticks_trailing_zeros() {
        let trimmed = AutoNumberFormat {
            uniform_decimals: false,
            ..AutoNumberFormat::DEFAULT
        };
        assert_eq!(
            auto_labels(trimmed, 0.2, 0..=5),
            ["0", "0.2", "0.4", "0.6", "0.8", "1"]
        );
        assert_eq!(
            auto_labels(trimmed, 0.25, 0..=4),
            ["0", "0.25", "0.5", "0.75", "1"]
        );
    }

    // Walk residues (`3 · 0.1 = 0.30000000000000004`, a zero walked to
    // `-1e-17`) print as the round value, and never as `-0`.
    #[test]
    fn auto_linear_ticks_absorb_walk_residues() {
        let auto = AutoNumberFormat::DEFAULT;
        assert_eq!(auto.linear_tick(3.0 * 0.1, 0.1, 0.0, 1.0).to_ascii(), "0.3");
        assert_eq!(auto.linear_tick(-1e-17, 0.1, -1.0, 1.0).to_ascii(), "0.0");
        assert_eq!(auto.linear_tick(-0.0, 1.0, -1.0, 1.0).to_ascii(), "0");
        // A fitted step that writes no round number keeps two decimals past
        // its first digit.
        assert_eq!(
            auto.linear_tick(2.0 * 1.312815, 1.312815, 0.0, 5.0)
                .to_ascii(),
            "2.63"
        );
        assert_eq!(auto.linear_tick(f64::NAN, 1.0, 0.0, 5.0).to_ascii(), "NaN");
    }

    #[test]
    fn auto_linear_switches_to_an_exponent_outside_the_window() {
        let auto = AutoNumberFormat::DEFAULT;
        // 1e-6 steps: order of magnitude -5 at the range's end.
        assert_eq!(
            auto_labels(auto, 2e-6, 0..=5),
            ["0", "2e-6", "4e-6", "6e-6", "8e-6", "1.0e-5"]
        );
        assert_eq!(
            auto_labels(auto, 1e-6, -2..=2),
            ["-2e-6", "-1e-6", "0", "1e-6", "2e-6"]
        );
        // 1e9 values.
        assert_eq!(auto_labels(auto, 1e9, 0..=3), ["0", "1e9", "2e9", "3e9"]);
        assert_eq!(auto_labels(auto, 5e8, 2..=4), ["1.0e9", "1.5e9", "2.0e9"]);
        assert_eq!(
            auto_labels(auto, 5e8, -4..=-2),
            ["-2.0e9", "-1.5e9", "-1.0e9"]
        );
        // Order 5 stays plain; order 6 switches.
        assert_eq!(auto_labels(auto, 2e5, 0..=2), ["0", "200000", "400000"]);
        assert_eq!(auto_labels(auto, 5e5, 0..=2), ["0", "5e5", "1.0e6"]);
        let engineering = AutoNumberFormat {
            notation: ExponentNotation::Engineering,
            ..AutoNumberFormat::DEFAULT
        };
        assert_eq!(
            auto_labels(engineering, 2e-6, 0..=5),
            ["0", "2e-6", "4e-6", "6e-6", "8e-6", "10e-6"]
        );
        assert_eq!(auto_labels(engineering, 5e9, 0..=2), ["0", "5e9", "10e9"]);
        // Wider limits keep the plain form.
        let wide = AutoNumberFormat {
            exponent_low: -8,
            exponent_high: 12,
            ..AutoNumberFormat::DEFAULT
        };
        assert_eq!(
            auto_labels(wide, 1e9, 0..=2),
            ["0", "1000000000", "2000000000"]
        );
        assert_eq!(auto_labels(wide, 2e-6, 0..=1), ["0.000000", "0.000002"]);
    }

    // A mantissa that rounds to the next power moves to that exponent.
    #[test]
    fn auto_exponent_mantissa_rounds_into_the_next_power() {
        let auto = AutoNumberFormat::DEFAULT;
        assert_eq!(
            auto.linear_tick(9.999_999e8, 1e9, 0.0, 2e9).to_ascii(),
            "1e9"
        );
    }

    #[test]
    fn auto_log_ticks_use_significant_digits_and_the_window() {
        let auto = AutoNumberFormat::DEFAULT;
        let log = |value| auto.log_tick(value, 2).to_ascii();
        assert_eq!(log(1.0), "1");
        assert_eq!(log(0.01), "0.01");
        assert_eq!(log(1000.0), "1000");
        assert_eq!(log(2.5e-3), "0.0025");
        assert_eq!(log(1e-6), "1e-6");
        assert_eq!(log(1e9), "1e9");
        assert_eq!(log(2.5e9), "2.5e9");
        assert_eq!(log(0.0), "0");
        let engineering = AutoNumberFormat {
            notation: ExponentNotation::Engineering,
            ..AutoNumberFormat::DEFAULT
        };
        assert_eq!(engineering.log_tick(2.5e10, 2).to_ascii(), "25e9");
    }

    // The default typesets an exponent tick as `Power` does (`2×10⁹`,
    // `10⁹` for a mantissa of one); a plain tick stays plain text.
    #[test]
    fn auto_power_ticks_typeset_the_exponent() {
        let ls = crate::defaults::label_style_x();
        let typeset = |value: f64, step: f64, max: f64| {
            let text = AutoNumberFormat::DEFAULT.linear_tick(value, step, 0.0, max);
            let rich = crate::chrome::format_tick_auto_power(&text, &ls);
            let shown: String = rich
                .segments
                .iter()
                .filter(|s| s.text != '\u{200B}')
                .map(|s| s.text)
                .collect();
            (shown, rich.segments.iter().any(|s| s.math_parent.is_some()))
        };
        assert_eq!(typeset(2e9, 1e9, 3e9), ("2×109".into(), true));
        assert_eq!(typeset(1e9, 1e9, 3e9), ("109".into(), true));
        assert_eq!(typeset(1.5e9, 5e8, 3e9), ("1.5×109".into(), true));
        assert_eq!(typeset(8e-6, 2e-6, 1e-5), ("8×10-6".into(), true));
        assert_eq!(typeset(0.8, 0.2, 1.0), ("0.8".into(), false));
        // The power of `format_tick_power` for the same value is the same.
        let power = crate::chrome::format_tick_power(2e9, 1, &ls);
        let auto = crate::chrome::format_tick_auto_power(
            &AutoNumberFormat::DEFAULT.linear_tick(2e9, 1e9, 0.0, 3e9),
            &ls,
        );
        assert_eq!(power, auto);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn auto_format_serde_writes_only_what_differs_from_the_default() {
        let auto = LabelFormat::Auto(AutoNumberFormat::DEFAULT);
        assert_eq!(serde_json::to_string(&auto).unwrap(), r#"{"Auto":{}}"#);
        assert_eq!(
            serde_json::from_str::<LabelFormat>(r#"{"Auto":{}}"#).unwrap(),
            auto
        );
        let custom = LabelFormat::Auto(AutoNumberFormat {
            uniform_decimals: false,
            notation: ExponentNotation::Scientific,
            exponent_low: -3,
            exponent_high: 4,
        });
        let json = serde_json::to_string(&custom).unwrap();
        assert_eq!(
            json,
            r#"{"Auto":{"uniform_decimals":false,"notation":"Scientific","exponent_low":-3,"exponent_high":4}}"#
        );
        assert_eq!(serde_json::from_str::<LabelFormat>(&json).unwrap(), custom);
        // The documented default of a document stays `Decimal` (#357).
        assert_eq!(
            crate::defaults::config().bottom_x.label_style.format,
            LabelFormat::Decimal
        );
    }

    // auto_significant_digits mutates only the sig field, and
    // auto_significant_digits_all only the sig fields.
    #[test]
    fn auto_sig_changes_only_sig_fields() {
        let mut cfg = crate::defaults::config();
        let before = cfg.clone();
        cfg.top_x.auto_significant_digits();
        let mut expected = before.clone();
        expected.top_x.label_style.significant_digits = LabelStyle::compute_auto_significant_digits(
            AxisScale::Linear,
            expected.top_x.min,
            expected.top_x.max,
            expected.top_x.major_spacing,
        );
        assert_eq!(cfg, expected);

        let mut cfg = before.clone();
        cfg.auto_significant_digits_all();
        assert!(cfg.top_x.label_style.significant_digits >= 1);
        assert!(cfg.bottom_x.label_style.significant_digits >= 1);
        assert!(cfg.left_y.label_style.significant_digits >= 1);
        assert!(cfg.right_y.label_style.significant_digits >= 1);
        assert_eq!(cfg.chart_area, before.chart_area);
        assert_eq!(cfg.grid, before.grid);
        assert_eq!(cfg.chart_title, before.chart_title);
    }
}
