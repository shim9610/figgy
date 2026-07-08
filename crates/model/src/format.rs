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
#[cfg_attr(feature = "serde", serde(default))]
pub struct TimestampLabelFormat {
    pub unit: TimestampUnit,
    pub timezone: TimestampZone,
    pub label: TimestampLabelMode,
    pub fractional: FractionalSecondDigits,
    pub tick_policy: TimestampTickPolicy,
}

impl Default for TimestampUnit {
    fn default() -> Self {
        Self::Seconds
    }
}

impl Default for TimestampZone {
    fn default() -> Self {
        Self::Utc
    }
}

impl Default for TimestampLabelMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl Default for FractionalSecondDigits {
    fn default() -> Self {
        Self::Auto
    }
}

impl Default for TimestampTickPolicy {
    fn default() -> Self {
        Self::AutoCalendar
    }
}

impl Default for TimestampLabelFormat {
    fn default() -> Self {
        Self {
            unit: TimestampUnit::default(),
            timezone: TimestampZone::default(),
            label: TimestampLabelMode::default(),
            fractional: FractionalSecondDigits::default(),
            tick_policy: TimestampTickPolicy::default(),
        }
    }
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
