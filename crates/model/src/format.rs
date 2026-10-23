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
    /// A plain number that depends on the value alone, not on the axis
    /// scale, spacing or significant digits: `2.5`, `1000`, `1.23e6`, or a
    /// fixed number of decimals. See [`PlainNumberFormat`].
    Plain(PlainNumberFormat),
}

/// How [`LabelFormat::Plain`] writes a number.
///
/// The automatic form ([`Self::AUTO`], `decimals: None`) is the label form of
/// the bounded chart families' value axes: `0` for zero, `{:.2e}` (`1.23e6`,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampUnit {
    #[default]
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampZone {
    #[default]
    Utc,
    /// Fixed offset from UTC in minutes. Korea Standard Time is 540.
    FixedOffsetMinutes(i32),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampLabelMode {
    #[default]
    Auto,
    /// Minimal strftime-like pattern: %Y %m %d %H %M %S %f and %%.
    Pattern(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FractionalSecondDigits {
    #[default]
    Auto,
    Fixed(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampTickPolicy {
    /// Calendar-aware ticks such as minutes, hours, days, months, and years.
    #[default]
    AutoCalendar,
    /// Reuse the numeric `major_spacing` / `minor_count` tick walkers.
    NumericSpacing,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
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
        }
    }
}

impl AxisOptions {
    pub fn auto_significant_digits(&mut self) {
        let sig = LabelStyle::compute_auto_significant_digits(
            self.scale.clone(),
            self.min,
            self.max,
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
    use crate::default::default_config;

    // Result must be in 1..=15.
    #[test]
    fn sig_digits_bounds() {
        for (min, max, step) in [
            (0.0, 1.0, 0.2),
            (0.0, 1000.0, 200.0),
            (0.0, 0.01, 0.002),
            (-5.0, 5.0, 1.0),
            (100.0, 110.0, 2.0),
            (0.0, 0.0, 0.0),
        ] {
            let s = LabelStyle::compute_auto_significant_digits(AxisScale::Linear, min, max, step);
            assert!((1..=15).contains(&s), "sig out of bounds: {}", s);
        }
    }

    #[test]
    fn sig_digits_spec_examples() {
        let cases: &[(f64, f64, f64, u8)] = &[
            (0.0, 1.0, 0.2, 2),
            (0.0, 1000.0, 200.0, 4),
            (0.0, 0.01, 0.002, 2),
            (-5.0, 5.0, 1.0, 1),
            (100.0, 110.0, 2.0, 3),
        ];
        for &(min, max, step, expected) in cases {
            let got =
                LabelStyle::compute_auto_significant_digits(AxisScale::Linear, min, max, step);
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
        assert_eq!(PlainNumberFormat::default(), PlainNumberFormat::AUTO);
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

    // auto_significant_digits should mutate only the sig field.
    #[test]
    fn auto_sig_changes_only_sig_field() {
        let mut cfg = default_config();
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
    }

    // auto_significant_digits_all should also mutate only sig fields.
    #[test]
    fn auto_sig_all_changes_only_sig_fields() {
        let mut cfg = default_config();
        let before = cfg.clone();
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
