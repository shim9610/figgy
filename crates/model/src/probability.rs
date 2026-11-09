//! Probability axis scales (#209): the f64 maps of normal, Weibull and
//! logistic probability paper.
//!
//! A probability axis places a cumulative probability `p` in `(0, 1)` at
//! `F⁻¹(p)`, so a sample of the distribution plotted against its plotting
//! positions falls on a straight line. These are the one CPU scale map: the
//! geometry, the ticks, the annotation space and the renderer's range
//! helpers call [`crate::config::AxisScale::forward`] and
//! [`crate::config::AxisScale::inverse`], which delegate here for a
//! probability axis. The shader has its own f32 twin (`probability.wgsl`).
//!
//! - **Normal** (probit), `z = Φ⁻¹(p)`: Wichura's AS241 `PPND16` (relative
//!   error about 1e-16), and its inverse `Φ(z) = erfc(−z/√2) / 2` with Cody's
//!   rational `erfc` (about 1e-16). `std` has no `erf`, and no dependency is
//!   added.
//! - **Weibull**, `y = ln(−ln(1 − p))`, and its inverse `p = 1 − exp(−eʸ)`.
//! - **Logistic** (logit), `y = ln(p / (1 − p))`, and its inverse
//!   `p = 1 / (1 + e⁻ʸ)`.
//!
//! For `p ≥ 0.5` every forward map works on `q = 1 − p`, which is exact
//! there (Sterbenz), so the upper tail keeps its digits. An f64 `p` cannot be
//! closer to 1 than `2⁻⁵³`, so the upper tail ends at `q = 1.1e-16`; the
//! lower tail reaches the smallest positive f64.

/// Which probability paper an axis is drawn on
/// ([`crate::config::AxisScale::Probability`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProbabilityScale {
    /// Normal probability paper (probit): `p` at `Φ⁻¹(p)`. With a
    /// logarithmic x axis it is lognormal paper.
    Normal,
    /// Weibull paper: `p` at `ln(−ln(1 − p))`. With a logarithmic time axis a
    /// Weibull sample falls on a line whose slope is its shape parameter.
    Weibull,
    /// Logistic paper (logit): `p` at `ln(p / (1 − p))`.
    Logistic,
}

impl ProbabilityScale {
    /// The position of probability `p` on this paper, or `None` when `p` has
    /// none: not finite, or outside the open interval `(0, 1)`.
    pub fn forward(self, p: f64) -> Option<f64> {
        if !(p.is_finite() && p > 0.0 && p < 1.0) {
            return None;
        }
        let y = match self {
            ProbabilityScale::Normal => ppnd16(p),
            ProbabilityScale::Weibull => {
                // −ln(1 − p): `ln_1p` below 0.5, `ln(q)` above, where
                // `q = 1 − p` is exact.
                let w = if p < 0.5 {
                    -(-p).ln_1p()
                } else {
                    -(1.0 - p).ln()
                };
                w.ln()
            }
            ProbabilityScale::Logistic => {
                if p < 0.5 {
                    p.ln() - (-p).ln_1p()
                } else {
                    let q = 1.0 - p;
                    (-q).ln_1p() - q.ln()
                }
            }
        };
        y.is_finite().then_some(y)
    }

    /// The probability at position `y` on this paper, the inverse of
    /// [`Self::forward`]. `None` for a non-finite `y` or a result that is
    /// not strictly inside `(0, 1)` (a position past the f64 tails).
    pub fn inverse(self, y: f64) -> Option<f64> {
        if !y.is_finite() {
            return None;
        }
        let p = match self {
            ProbabilityScale::Normal => 0.5 * erfc(-y * std::f64::consts::FRAC_1_SQRT_2),
            ProbabilityScale::Weibull => -(-y.exp()).exp_m1(),
            ProbabilityScale::Logistic => 1.0 / (1.0 + (-y).exp()),
        };
        (p.is_finite() && p > 0.0 && p < 1.0).then_some(p)
    }
}

/// `x` evaluated as the polynomial `c[0] + c[1]·x + …` (Horner).
fn polynomial(c: &[f64], x: f64) -> f64 {
    c.iter().rev().fold(0.0, |acc, a| acc * x + a)
}

// AS241 (Wichura 1988), PPND16: the central region |p − 0.5| ≤ 0.425 and the
// two tail regions r = √(−ln min(p, 1 − p)) ≤ 5 and > 5.
const PPND16_A: [f64; 8] = [
    3.387_132_872_796_366_608_0,
    1.331_416_678_917_843_774_5e2,
    1.971_590_950_306_551_442_7e3,
    1.373_169_376_550_946_112_5e4,
    4.592_195_393_154_987_145_7e4,
    6.726_577_092_700_870_085_3e4,
    3.343_057_558_358_812_810_5e4,
    2.509_080_928_730_122_672_7e3,
];
const PPND16_B: [f64; 8] = [
    1.0,
    4.231_333_070_160_091_125_2e1,
    6.871_870_074_920_579_083_0e2,
    5.394_196_021_424_751_107_7e3,
    2.121_379_430_158_659_586_7e4,
    3.930_789_580_009_271_061_0e4,
    2.872_908_573_572_194_267_4e4,
    5.226_495_278_852_854_561_0e3,
];
const PPND16_C: [f64; 8] = [
    1.423_437_110_749_683_577_34,
    4.630_337_846_156_545_295_90,
    5.769_497_221_460_691_405_50,
    3.647_848_324_763_204_605_04,
    1.270_458_252_452_368_382_58,
    2.417_807_251_774_506_117_70e-1,
    2.272_384_498_926_918_458_33e-2,
    7.745_450_142_783_414_076_40e-4,
];
const PPND16_D: [f64; 8] = [
    1.0,
    2.053_191_626_637_758_821_87,
    1.676_384_830_183_803_849_40,
    6.897_673_349_851_000_045_50e-1,
    1.481_039_764_274_800_745_90e-1,
    1.519_866_656_361_645_719_66e-2,
    5.475_938_084_995_344_946_00e-4,
    1.050_750_071_644_416_843_24e-9,
];
const PPND16_E: [f64; 8] = [
    6.657_904_643_501_103_777_20,
    5.463_784_911_164_114_369_90,
    1.784_826_539_917_291_335_80,
    2.965_605_718_285_048_912_30e-1,
    2.653_218_952_657_612_309_30e-2,
    1.242_660_947_388_078_438_60e-3,
    2.711_555_568_743_487_578_15e-5,
    2.010_334_399_292_288_132_65e-7,
];
const PPND16_F: [f64; 8] = [
    1.0,
    5.998_322_065_558_879_376_90e-1,
    1.369_298_809_227_358_053_10e-1,
    1.487_536_129_085_061_485_25e-2,
    7.868_691_311_456_132_591_00e-4,
    1.846_318_317_510_054_681_80e-5,
    1.421_511_758_316_445_888_70e-7,
    2.044_263_103_389_939_785_64e-15,
];

/// `Φ⁻¹(p)` for `p` in `(0, 1)`: AS241 `PPND16`.
fn ppnd16(p: f64) -> f64 {
    let q = p - 0.5;
    if q.abs() <= 0.425 {
        let r = 0.180_625 - q * q;
        return q * polynomial(&PPND16_A, r) / polynomial(&PPND16_B, r);
    }
    // The tail: the smaller of p and 1 − p, exact for p ≥ 0.5.
    let tail = if q < 0.0 { p } else { 1.0 - p };
    let r = (-tail.ln()).sqrt();
    let v = if r <= 5.0 {
        let r = r - 1.6;
        polynomial(&PPND16_C, r) / polynomial(&PPND16_D, r)
    } else {
        let r = r - 5.0;
        polynomial(&PPND16_E, r) / polynomial(&PPND16_F, r)
    };
    if q < 0.0 { -v } else { v }
}

// Cody (1969), the rational Chebyshev approximations of CALERF: |x| ≤ 0.46875,
// 0.46875 < |x| ≤ 4 and |x| > 4.
const ERFC_A: [f64; 5] = [
    3.161_123_743_870_565_60,
    1.138_641_541_510_501_56e2,
    3.774_852_376_853_020_21e2,
    3.209_377_589_138_469_47e3,
    1.857_777_061_846_031_53e-1,
];
const ERFC_B: [f64; 4] = [
    2.360_129_095_234_412_09e1,
    2.440_246_379_344_441_73e2,
    1.282_616_526_077_372_28e3,
    2.844_236_833_439_170_62e3,
];
const ERFC_C: [f64; 9] = [
    5.641_884_969_886_700_89e-1,
    8.883_149_794_388_375_94,
    6.611_919_063_714_162_95e1,
    2.986_351_381_974_001_31e2,
    8.819_522_212_417_690_90e2,
    1.712_047_612_634_070_58e3,
    2.051_078_377_826_071_47e3,
    1.230_339_354_797_997_25e3,
    2.153_115_354_744_038_46e-8,
];
const ERFC_D: [f64; 8] = [
    1.574_492_611_070_983_47e1,
    1.176_939_508_913_124_99e2,
    5.371_811_018_620_098_58e2,
    1.621_389_574_566_690_19e3,
    3.290_799_235_733_459_63e3,
    4.362_619_090_143_247_16e3,
    3.439_367_674_143_721_64e3,
    1.230_339_354_803_749_42e3,
];
const ERFC_P: [f64; 6] = [
    3.053_266_349_612_323_44e-1,
    3.603_448_999_498_044_39e-1,
    1.257_817_261_112_292_46e-1,
    1.608_378_514_874_227_66e-2,
    6.587_491_615_298_378_03e-4,
    1.631_538_713_730_209_78e-2,
];
const ERFC_Q: [f64; 5] = [
    2.568_520_192_289_822_42,
    1.872_952_849_923_467_25,
    5.279_051_029_514_284_12e-1,
    6.051_834_131_244_131_91e-2,
    2.335_204_976_268_691_85e-3,
];

/// `exp(−y²)` split as `exp(−ŷ²) · exp(−(y − ŷ)(y + ŷ))` with `ŷ` `y`
/// truncated to sixteenths, as CALERF does, so the large exponent keeps its
/// digits.
fn exp_neg_square(y: f64) -> f64 {
    let head = (y * 16.0).trunc() / 16.0;
    let rest = (y - head) * (y + head);
    (-head * head).exp() * (-rest).exp()
}

/// The complementary error function `erfc(x)`: Cody's CALERF.
fn erfc(x: f64) -> f64 {
    let y = x.abs();
    let upper = if y <= 0.468_75 {
        // erf(x) on the central interval, then 1 − erf.
        let ysq = if y > 1.11e-16 { y * y } else { 0.0 };
        let (mut num, mut den) = (ERFC_A[4] * ysq, ysq);
        for i in 0..3 {
            num = (num + ERFC_A[i]) * ysq;
            den = (den + ERFC_B[i]) * ysq;
        }
        return 1.0 - x * (num + ERFC_A[3]) / (den + ERFC_B[3]);
    } else if y <= 4.0 {
        let (mut num, mut den) = (ERFC_C[8] * y, y);
        for i in 0..7 {
            num = (num + ERFC_C[i]) * y;
            den = (den + ERFC_D[i]) * y;
        }
        exp_neg_square(y) * (num + ERFC_C[7]) / (den + ERFC_D[7])
    } else if y < 26.543 {
        let ysq = 1.0 / (y * y);
        let (mut num, mut den) = (ERFC_P[5] * ysq, ysq);
        for i in 0..4 {
            num = (num + ERFC_P[i]) * ysq;
            den = (den + ERFC_Q[i]) * ysq;
        }
        let r = ysq * (num + ERFC_P[4]) / (den + ERFC_Q[4]);
        let one_over_sqrt_pi = 5.641_895_835_477_562_869_5e-1;
        exp_neg_square(y) * (one_over_sqrt_pi - r) / y
    } else {
        0.0
    };
    if x >= 0.0 { upper } else { 2.0 - upper }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALES: [ProbabilityScale; 3] = [
        ProbabilityScale::Normal,
        ProbabilityScale::Weibull,
        ProbabilityScale::Logistic,
    ];

    /// Probabilities in increasing order from `1e-30` to the closest to 1 an
    /// f64 holds, through both tails and the middle.
    fn probe_probabilities() -> impl Iterator<Item = f64> {
        let lower = (3..=30).rev().map(|k| 10f64.powi(-k));
        let middle = (1..100).map(|k| f64::from(k) / 100.0);
        let upper = (3..=15).map(|k| 1.0 - 10f64.powi(-k));
        lower
            .chain(middle)
            .chain(upper)
            .chain([1.0 - f64::EPSILON / 2.0])
    }

    #[test]
    fn ppnd16_matches_reference_quantiles() {
        // Φ⁻¹ at standard points (Wichura 1988; Abramowitz and Stegun).
        for (p, z) in [
            (0.975, 1.959_963_984_540_054),
            (0.5, 0.0),
            (0.841_344_746_068_542_9, 1.0),
            (0.001, -3.090_232_306_167_813_6),
            (1e-10, -6.361_340_902_404_056),
            (0.999_999, 4.753_424_308_817_089),
        ] {
            let got = ProbabilityScale::Normal.forward(p).unwrap();
            assert!(
                (got - z).abs() <= 1e-14 * z.abs().max(1.0),
                "Φ⁻¹({p}) = {got}, want {z}"
            );
        }
    }

    #[test]
    fn erfc_matches_reference_values() {
        for (x, want) in [
            (0.0, 1.0),
            (0.3, 0.671_373_240_540_872_7),
            (0.5, 0.479_500_122_186_953_5),
            (1.0, 0.157_299_207_050_285_13),
            (2.0, 4.677_734_981_047_266e-3),
            (3.5, 7.430_983_723_414_128e-7),
            (5.0, 1.537_459_794_428_035e-12),
            (10.0, 2.088_487_583_762_545e-45),
            (-1.0, 1.842_700_792_949_715),
        ] {
            let got = erfc(x);
            assert!(
                (got - want).abs() <= 2e-15 * want,
                "erfc({x}) = {got:e}, want {want:e}"
            );
        }
        assert_eq!(erfc(30.0), 0.0);
    }

    #[test]
    fn forward_and_inverse_round_trip_at_every_tail() {
        for scale in SCALES {
            for p in probe_probabilities() {
                let y = scale.forward(p).unwrap_or_else(|| panic!("{scale:?} {p}"));
                let back = scale.inverse(y).unwrap_or_else(|| panic!("{scale:?} {y}"));
                // Relative below 0.5, where p holds the tail's digits; above,
                // within the f64 spacing of p near 1, which is all the tail
                // `1 − p` an f64 p can carry.
                let close = if p < 0.5 {
                    (back - p).abs() <= 1e-12 * p
                } else {
                    (back - p).abs() <= 4.0 * f64::EPSILON
                };
                assert!(close, "{scale:?}: p = {p:e} → {y} → {back:e}");
            }
        }
    }

    #[test]
    fn forward_is_monotone_and_symmetric_where_the_paper_is() {
        for scale in SCALES {
            let mut previous = f64::NEG_INFINITY;
            for p in probe_probabilities() {
                let y = scale.forward(p).unwrap();
                assert!(y > previous, "{scale:?} at {p}");
                previous = y;
            }
        }
        for p in [1e-9, 0.01, 0.2, 0.4] {
            // The pair `1 − q` and `q` an f64 holds exactly.
            let (high_p, low_p) = (1.0 - p, 1.0 - (1.0 - p));
            for scale in [ProbabilityScale::Normal, ProbabilityScale::Logistic] {
                let low = scale.forward(low_p).unwrap();
                let high = scale.forward(high_p).unwrap();
                assert!((low + high).abs() <= 1e-12 * low.abs(), "{scale:?} {p}");
            }
        }
        // Weibull: y = 0 at p = 1 − 1/e.
        let at = ProbabilityScale::Weibull
            .forward(1.0 - (-1.0f64).exp())
            .unwrap();
        assert!(at.abs() < 1e-15, "{at}");
    }

    #[test]
    fn values_outside_the_open_interval_have_no_position() {
        for scale in SCALES {
            for p in [0.0, 1.0, -0.5, 1.5, f64::NAN, f64::INFINITY] {
                assert_eq!(scale.forward(p), None, "{scale:?} {p}");
            }
            assert_eq!(scale.inverse(f64::NAN), None);
            // Past the f64 upper tail: 1 − p rounds to 0.
            assert_eq!(scale.inverse(1e3), None, "{scale:?}");
        }
    }
}

/// A probability axis through the model (#209): the plan, the tick walk,
/// the labels and their ladders, the geometry, the annotation space, the
/// validation and the serialized form.
#[cfg(test)]
mod axis_tests {
    use super::ProbabilityScale;
    use crate::annotation::{AxisMap, AxisSpace};
    use crate::config::{AxisOptions, AxisScale, Category};
    use crate::format::LabelFormat;
    use crate::layout::{BreakMap, axis_fraction, guarded_probability_range};
    use crate::tick::{TickError, ladder_multiple, log_label_ladders, major_ticks, minor_ticks};

    fn axis(scale: ProbabilityScale, min: f64, max: f64) -> AxisOptions {
        let mut axis = crate::defaults::config().left_y;
        axis.scale = AxisScale::Probability(scale);
        axis.min = min;
        axis.max = max;
        axis.minor_count = 1;
        axis
    }

    fn close(got: &[f64], want: &[f64]) -> bool {
        got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-12)
    }

    #[test]
    fn majors_take_the_sparsest_tail_ladder_with_four_ticks() {
        let normal = ProbabilityScale::Normal;
        let decades: Vec<f64> = major_ticks(&axis(normal, 0.001, 0.999)).collect();
        assert!(
            close(&decades, &[0.001, 0.01, 0.1, 0.5, 0.9, 0.99, 0.999]),
            "{decades:?}"
        );
        let one_two_five: Vec<f64> = major_ticks(&axis(normal, 0.1, 0.9)).collect();
        assert!(
            close(&one_two_five, &[0.1, 0.2, 0.5, 0.8, 0.9]),
            "{one_two_five:?}"
        );
        let deep: Vec<f64> = major_ticks(&axis(normal, 1e-6, 0.5)).collect();
        assert!(
            close(&deep, &[1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 0.1, 0.5]),
            "{deep:?}"
        );
        // Too narrow for the densest ladder: a linear grid in p through 0.5.
        let narrow: Vec<f64> = major_ticks(&axis(normal, 0.45, 0.55)).collect();
        assert!(narrow.len() >= 4, "{narrow:?}");
        assert!(narrow.iter().any(|p| (p - 0.5).abs() < 1e-12), "{narrow:?}");
        let steps: Vec<f64> = narrow.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|s| (s - steps[0]).abs() < 1e-12),
            "{narrow:?}"
        );
        // Every walk rises and stays inside the range.
        for (min, max) in [
            (0.001, 0.999),
            (0.02, 0.97),
            (1e-9, 1.0 - 1e-9),
            (0.6, 0.99),
        ] {
            for scale in [
                ProbabilityScale::Normal,
                ProbabilityScale::Weibull,
                ProbabilityScale::Logistic,
            ] {
                let a = axis(scale, min, max);
                for walk in [
                    major_ticks(&a).collect::<Vec<_>>(),
                    minor_ticks(&a).collect(),
                ] {
                    assert!(walk.windows(2).all(|w| w[1] > w[0]), "{walk:?}");
                    assert!(
                        walk.iter()
                            .all(|p| *p >= min * (1.0 - 1e-9) && *p <= max + 1e-9),
                        "{min}..{max}: {walk:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn minors_are_the_next_denser_ladder_less_the_majors() {
        let normal = ProbabilityScale::Normal;
        let minors: Vec<f64> = minor_ticks(&axis(normal, 0.001, 0.999)).collect();
        assert!(
            close(
                &minors,
                &[0.002, 0.005, 0.02, 0.05, 0.2, 0.8, 0.95, 0.98, 0.995, 0.998]
            ),
            "{minors:?}"
        );
        let minors: Vec<f64> = minor_ticks(&axis(normal, 0.1, 0.9)).collect();
        assert!(close(&minors, &[0.3, 0.4, 0.6, 0.7]), "{minors:?}");
        // minor_count is a switch.
        let mut off = axis(normal, 0.001, 0.999);
        off.minor_count = 0;
        assert_eq!(minor_ticks(&off).count(), 0);
    }

    #[test]
    fn the_plan_widens_to_the_next_tail_decade() {
        let plan = |min, max| {
            AxisOptions::compute_nice_ticks(
                AxisScale::Probability(ProbabilityScale::Normal),
                min,
                max,
                7,
            )
        };
        let p = plan(0.003, 0.997).unwrap();
        assert!(
            (p.min - 0.001).abs() < 1e-15 && (p.max - 0.999).abs() < 1e-12,
            "{p:?}"
        );
        let p = plan(0.3, 0.7).unwrap();
        assert!(
            (p.min - 0.1).abs() < 1e-15 && (p.max - 0.9).abs() < 1e-12,
            "{p:?}"
        );
        let p = plan(0.6, 0.95).unwrap();
        assert!(
            (p.min - 0.5).abs() < 1e-15 && (p.max - 0.99).abs() < 1e-12,
            "{p:?}"
        );
        assert_eq!(plan(0.0, 0.9), Err(TickError::OutsideProbability));
        assert_eq!(plan(0.1, 1.0), Err(TickError::OutsideProbability));
        assert_eq!(plan(0.5, 0.2), Err(TickError::InvalidRange));
    }

    #[test]
    fn labels_keep_the_digits_of_their_tail() {
        let scale = AxisScale::Probability(ProbabilityScale::Normal);
        let label = |value: f64, format: &LabelFormat| {
            crate::chrome::format_tick_value(value, format, 1, &scale, 1.0, 0.001, 0.999)
        };
        for (value, percent, decimal) in [
            (0.001, "0.1%", "0.001"),
            (0.01, "1%", "0.01"),
            (0.5, "50%", "0.5"),
            (0.9, "90%", "0.9"),
            (0.999, "99.9%", "0.999"),
            (0.9999, "99.99%", "0.9999"),
            (1e-5, "0.001%", "0.00001"),
        ] {
            assert_eq!(label(value, &LabelFormat::Percent), percent, "{value}");
            assert_eq!(label(value, &LabelFormat::Decimal), decimal, "{value}");
        }
        // Percent on a linear axis: the value times 100 at the spacing's
        // decimals.
        let linear = crate::chrome::format_tick_value(
            0.25,
            &LabelFormat::Percent,
            2,
            &AxisScale::Linear,
            0.05,
            0.0,
            1.0,
        );
        assert_eq!(linear, "25%");
    }

    #[test]
    fn label_ladders_demote_along_the_tails_and_keep_half() {
        let scale = AxisScale::Probability(ProbabilityScale::Normal);
        assert_eq!(ladder_multiple(0.5, &scale), Some(1));
        assert_eq!(ladder_multiple(0.999, &scale), Some(1));
        assert_eq!(ladder_multiple(0.8, &scale), Some(2));
        assert_eq!(ladder_multiple(0.05, &scale), Some(5));
        assert_eq!(ladder_multiple(0.37, &scale), None);
        assert_eq!(ladder_multiple(1.0, &scale), None);
        let a = axis(ProbabilityScale::Normal, 0.1, 0.9);
        assert_eq!(log_label_ladders(&a).len(), 2);
        assert!(log_label_ladders(&axis(ProbabilityScale::Normal, 0.001, 0.999)).is_empty());
    }

    #[test]
    fn the_geometry_and_the_annotation_space_place_f_inverse() {
        for scale in [
            ProbabilityScale::Normal,
            ProbabilityScale::Weibull,
            ProbabilityScale::Logistic,
        ] {
            let a = axis(scale, 0.001, 0.999);
            let (low, high) = (scale.forward(0.001).unwrap(), scale.forward(0.999).unwrap());
            for p in [0.001, 0.02, 0.5, 0.9, 0.999] {
                let want = (scale.forward(p).unwrap() - low) / (high - low);
                let got = f64::from(axis_fraction(p, &a, 100.0));
                assert!((got - want).abs() < 1e-6, "{scale:?} {p}: {got} vs {want}");
            }
            let space = AxisSpace {
                map: AxisMap::Probability {
                    scale,
                    min: 0.001,
                    max: 0.999,
                },
                inverted: false,
                breaks: BreakMap::IDENTITY,
            };
            for p in [0.002, 0.5, 0.995] {
                let t = space.fraction(p).unwrap();
                let back = space.value(t).unwrap();
                assert!((back - p).abs() < 1e-12, "{scale:?} {p} → {t} → {back}");
            }
            assert_eq!(space.fraction(0.0), None);
            assert_eq!(space.fraction(1.0), None);
        }
        // Normal and logistic paper are symmetric about 0.5.
        let a = axis(ProbabilityScale::Normal, 0.001, 0.999);
        assert!((axis_fraction(0.5, &a, 100.0) - 0.5).abs() < 1e-6);
        assert_eq!(guarded_probability_range(0.0, 2.0), (0.001, 0.999));
    }

    #[test]
    fn probability_axes_are_held_to_their_rules() {
        let normal = AxisScale::Probability(ProbabilityScale::Normal);
        let check = |edit: &dyn Fn(&mut crate::config::Config)| {
            let mut config = crate::defaults::config();
            config.left_y.scale = normal.clone();
            config.left_y.min = 0.01;
            config.left_y.max = 0.99;
            edit(&mut config);
            config.validate_chart().err().map(|e| e.to_string())
        };
        assert_eq!(check(&|_| {}), None);
        for (edit, want) in [
            (
                Box::new(|c: &mut crate::config::Config| c.left_y.min = 0.0)
                    as Box<dyn Fn(&mut crate::config::Config)>,
                "strictly between 0 and 1",
            ),
            (
                Box::new(|c: &mut crate::config::Config| c.left_y.max = 1.0),
                "strictly between 0 and 1",
            ),
            (
                Box::new(|c: &mut crate::config::Config| {
                    c.left_y.breaks = Some(crate::config::AxisBreaks {
                        ranges: vec![crate::config::AxisBreak::new(0.2, 1.2)],
                        ..crate::defaults::axis_breaks()
                    })
                }),
                "a cut range on a probability axis",
            ),
            (
                Box::new(|c: &mut crate::config::Config| {
                    let mut bar = crate::defaults::color_bar_options();
                    bar.axis.scale = AxisScale::Probability(ProbabilityScale::Weibull);
                    bar.axis.min = 0.1;
                    bar.axis.max = 0.9;
                    c.colorbar = Some(bar);
                }),
                "colourbar axis is linear or logarithmic",
            ),
        ] {
            let got = check(&*edit).unwrap_or_default();
            assert!(got.contains(want), "{got}");
        }
        let mut categorical = axis(ProbabilityScale::Normal, 0.1, 0.9);
        categorical.categories = Some(vec![Category::new("a", "A")]);
        assert!(
            categorical
                .validate_categories()
                .unwrap_err()
                .to_string()
                .contains("probability")
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_scale_serializes_as_a_nested_variant() {
        let scale = AxisScale::Probability(ProbabilityScale::Weibull);
        let json = serde_json::to_string(&scale).unwrap();
        assert_eq!(json, r#"{"Probability":"Weibull"}"#);
        assert_eq!(serde_json::from_str::<AxisScale>(&json).unwrap(), scale);
        assert_eq!(
            serde_json::to_string(&AxisScale::Logarithmic).unwrap(),
            r#""Logarithmic""#
        );
        let format = serde_json::to_string(&LabelFormat::Percent).unwrap();
        assert_eq!(format, r#""Percent""#);
        let mut config = crate::defaults::config();
        config.bottom_x.scale = AxisScale::Probability(ProbabilityScale::Logistic);
        config.bottom_x.min = 0.05;
        config.bottom_x.max = 0.95;
        let text = serde_json::to_string(&config).unwrap();
        let back: crate::config::Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }
}
