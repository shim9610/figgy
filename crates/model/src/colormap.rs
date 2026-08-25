//! Continuous colour ramps — the single source for every z→colour mapping.
//!
//! Separate from [`crate::preset::ColorCycle`] because the two answer different
//! questions. A cycle hands out *discrete* colours by series index and wraps;
//! a colormap is a *continuous* ramp sampled at a normalized position, and
//! wrapping it would be meaningless. They sit side by side rather than merged.
//!
//! Everything that turns a z value into a colour goes through [`sample`] or the
//! [`lut`] it builds: the CPU colourbar strip, the GPU lookup texture, and
//! whatever picks per-level contour colours. One ramp definition, so a heatmap
//! cell and the colourbar tick beside it cannot disagree about what a value
//! looks like.
//!
//! No dependencies beyond [`Color`] — this module is pure data and arithmetic,
//! which is what lets the renderer bake it into a texture and the raster path
//! draw it directly.

use crate::color::Color;

/// Entries in the baked lookup table. Matches the GPU texture width the
/// renderer uploads, so the CPU strip and the shader sample the same steps.
pub const LUT_LEN: usize = 256;

const fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

/// A continuous colour ramp.
///
/// The named ramps are fieldless so a host UI can list them in a dropdown and
/// they cross the wasm boundary as plain integers, following
/// [`crate::preset::ColorCycle`]. `Custom` carries its own stops for hosts that
/// need a ramp figgy does not ship.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColorMap {
    /// Perceptually uniform blue→green→yellow. The safe default for unsigned
    /// magnitudes.
    #[default]
    Viridis,
    /// Perceptually uniform black→purple→orange→white.
    Magma,
    /// High-contrast rainbow. Reads well for fine structure, poor for
    /// judging magnitude — and unreadable in greyscale print.
    Turbo,
    /// Black→white. For print, and for overlaying contour lines on top.
    GrayScale,
    /// Diverging red→white→blue. For signed data around a meaningful zero;
    /// set `axis.min = -axis.max` or the white midpoint lands off zero and the
    /// ramp lies about the sign.
    RdBu,
    /// Host-supplied stops, sampled in order at evenly spaced positions.
    ///
    /// Fewer than two stops cannot define a ramp: [`sample`] and [`lut`] fall
    /// back to fully transparent rather than inventing an endpoint, so a
    /// malformed custom map draws nothing instead of drawing a lie.
    Custom { stops: Vec<Color> },
}

// Ramp control points, sampled from the reference implementations at even
// spacing. Eight stops per named ramp: enough that linear interpolation between
// them stays visually smooth at 256 LUT entries, few enough to read here.

const VIRIDIS: &[Color] = &[
    rgb8(68, 1, 84),
    rgb8(72, 40, 120),
    rgb8(62, 74, 137),
    rgb8(49, 104, 142),
    rgb8(38, 130, 142),
    rgb8(53, 183, 121),
    rgb8(145, 213, 66),
    rgb8(253, 231, 37),
];

const MAGMA: &[Color] = &[
    rgb8(0, 0, 4),
    rgb8(28, 16, 68),
    rgb8(79, 18, 123),
    rgb8(129, 37, 129),
    rgb8(181, 54, 122),
    rgb8(229, 80, 100),
    rgb8(251, 143, 97),
    rgb8(252, 253, 191),
];

const TURBO: &[Color] = &[
    rgb8(48, 18, 59),
    rgb8(70, 107, 227),
    rgb8(40, 187, 236),
    rgb8(60, 231, 160),
    rgb8(163, 252, 60),
    rgb8(248, 202, 47),
    rgb8(238, 108, 26),
    rgb8(122, 4, 3),
];

const GRAY_SCALE: &[Color] = &[Color::BLACK, Color::WHITE];

const RD_BU: &[Color] = &[
    rgb8(103, 0, 31),
    rgb8(178, 24, 43),
    rgb8(214, 96, 77),
    rgb8(244, 165, 130),
    rgb8(247, 247, 247),
    rgb8(146, 197, 222),
    rgb8(67, 147, 195),
    rgb8(5, 48, 97),
];

impl ColorMap {
    /// The ramp's control points, in ramp order.
    pub fn stops(&self) -> &[Color] {
        match self {
            Self::Viridis => VIRIDIS,
            Self::Magma => MAGMA,
            Self::Turbo => TURBO,
            Self::GrayScale => GRAY_SCALE,
            Self::RdBu => RD_BU,
            Self::Custom { stops } => stops,
        }
    }

    /// Colour at normalized position `t`.
    ///
    /// `t` is clamped to `[0, 1]`, so a caller that has not normalized its z
    /// range gets the endpoint colour rather than an out-of-range index. NaN
    /// clamps to the low end — but a NaN z should be drawn with the
    /// colourbar's `nan_color` instead of being pushed through here, because
    /// "missing" and "smallest" are different facts.
    pub fn sample(&self, t: f32) -> Color {
        sample(self, t)
    }

    /// The ramp baked to [`LUT_LEN`] entries.
    ///
    /// A convenience for callers that want the ramp as a table — a legend image,
    /// a texture upload of their own. figgy's own paths do **not** use it: the
    /// colourbar strip and `field_columnar.wgsl` both evaluate
    /// [`Self::sample`] over the same stops, which is what makes their colours
    /// agree exactly instead of within a quantization step.
    pub fn lut(&self) -> [Color; LUT_LEN] {
        lut(self)
    }
}

/// Colour at normalized position `t` — see [`ColorMap::sample`].
pub fn sample(map: &ColorMap, t: f32) -> Color {
    let stops = map.stops();
    match stops.len() {
        // A ramp needs two ends. Rather than invent one, draw nothing: a
        // malformed `Custom` map is visibly absent instead of quietly wrong.
        0 => Color::new(0.0, 0.0, 0.0, 0.0),
        1 => stops[0],
        n => {
            // NaN fails both comparisons, so clamp explicitly to the low end.
            let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
            let scaled = t * (n - 1) as f32;
            // The last segment owns t = 1: clamping the index there leaves the
            // fraction measured against *that* segment's start, so the ramp
            // reaches `stops[n - 1]` exactly instead of stopping one stop short.
            let index = (scaled.floor() as usize).min(n - 2);
            let frac = scaled - index as f32;
            // A stop landed on exactly is returned as itself. `Color::lerp` is
            // `a + (b - a) * t`, which at t = 1 lands an ulp off `b` — small,
            // but it would put a colour at the colourbar's end that its axis
            // label does not name, which is what this function must not do.
            if frac <= 0.0 {
                stops[index]
            } else if frac >= 1.0 {
                stops[index + 1]
            } else {
                stops[index].lerp(stops[index + 1], frac)
            }
        }
    }
}

/// The ramp baked to [`LUT_LEN`] entries — see [`ColorMap::lut`].
///
/// A fixed array, not a `Vec`: baking the ramp is not allowed to allocate. The
/// caller decides where the 4 KiB lives — straight into a `write_texture` for
/// the GPU path, or a local for the colourbar strip — and neither one needs a
/// heap block whose size was known at compile time.
pub fn lut(map: &ColorMap) -> [Color; LUT_LEN] {
    let last = (LUT_LEN - 1) as f32;
    core::array::from_fn(|i| sample(map, i as f32 / last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_ramp_has_at_least_two_stops() {
        for map in [
            ColorMap::Viridis,
            ColorMap::Magma,
            ColorMap::Turbo,
            ColorMap::GrayScale,
            ColorMap::RdBu,
        ] {
            assert!(
                map.stops().len() >= 2,
                "{map:?} cannot define a ramp with {} stop(s)",
                map.stops().len()
            );
        }
    }

    #[test]
    fn endpoints_are_the_first_and_last_stop_exactly() {
        // The colourbar draws its ends at t = 0 and t = 1 and its axis labels
        // them with axis.min / axis.max, so an endpoint that interpolated even
        // slightly would put a colour next to a number it is not.
        for map in [ColorMap::Viridis, ColorMap::RdBu, ColorMap::GrayScale] {
            let stops = map.stops();
            assert_eq!(map.sample(0.0), stops[0]);
            assert_eq!(map.sample(1.0), stops[stops.len() - 1]);
        }
    }

    #[test]
    fn out_of_range_and_nan_clamp_instead_of_panicking() {
        let map = ColorMap::Viridis;
        assert_eq!(map.sample(-5.0), map.sample(0.0));
        assert_eq!(map.sample(5.0), map.sample(1.0));
        assert_eq!(map.sample(f32::NAN), map.sample(0.0));
        assert_eq!(map.sample(f32::INFINITY), map.sample(1.0));
        assert_eq!(map.sample(f32::NEG_INFINITY), map.sample(0.0));
    }

    #[test]
    fn grayscale_is_a_straight_ramp_from_black_to_white() {
        let map = ColorMap::GrayScale;
        let mid = map.sample(0.5);
        assert!((mid.r - 0.5).abs() < 1e-6, "midpoint is {mid:?}");
        assert!((mid.g - 0.5).abs() < 1e-6);
        assert!((mid.b - 0.5).abs() < 1e-6);
        assert_eq!(mid.a, 1.0);
    }

    #[test]
    fn a_custom_ramp_with_too_few_stops_draws_nothing() {
        let empty = ColorMap::Custom { stops: Vec::new() };
        assert_eq!(empty.sample(0.5).a, 0.0, "no stops cannot be a ramp");

        // One stop is a constant, not a ramp — but it is unambiguous, so it is
        // honoured rather than blanked.
        let single = ColorMap::Custom {
            stops: vec![Color::new(0.2, 0.4, 0.6, 1.0)],
        };
        assert_eq!(single.sample(0.0), single.sample(1.0));
        assert_eq!(single.sample(0.5).r, 0.2);
    }

    #[test]
    fn a_custom_ramp_interpolates_its_stops_evenly() {
        let map = ColorMap::Custom {
            stops: vec![Color::BLACK, Color::WHITE, Color::BLACK],
        };
        assert_eq!(map.sample(0.0), Color::BLACK);
        assert_eq!(map.sample(1.0), Color::BLACK);
        let mid = map.sample(0.5);
        assert!(
            (mid.r - 1.0).abs() < 1e-6,
            "the middle stop is white: {mid:?}"
        );
        let quarter = map.sample(0.25);
        assert!((quarter.r - 0.5).abs() < 1e-6, "quarter is {quarter:?}");
    }

    #[test]
    fn the_lut_is_the_ramp_at_lut_len_steps() {
        let map = ColorMap::Magma;
        let table = map.lut();
        assert_eq!(table.len(), LUT_LEN);
        assert_eq!(table[0], map.sample(0.0));
        assert_eq!(table[LUT_LEN - 1], map.sample(1.0));
        // The GPU texture and this table must be the same steps, which only
        // holds if entry i is the ramp at i/(LUT_LEN-1).
        let last = (LUT_LEN - 1) as f32;
        for (i, entry) in table.iter().enumerate() {
            assert_eq!(*entry, map.sample(i as f32 / last), "entry {i}");
        }
    }

    #[test]
    fn ramps_are_monotone_in_luminance_where_they_claim_to_be() {
        // Viridis, Magma and GrayScale are perceptually ordered: a reader
        // judges magnitude by brightness, so a dip would misreport the data.
        // Turbo and RdBu make no such claim and are excluded on purpose.
        for map in [ColorMap::Viridis, ColorMap::Magma, ColorMap::GrayScale] {
            let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            let table = map.lut();
            for pair in table.windows(2) {
                let (a, b) = (luminance(pair[0]), luminance(pair[1]));
                assert!(b + 1e-4 >= a, "{map:?} luminance dips: {a} then {b}");
            }
        }
    }
}
