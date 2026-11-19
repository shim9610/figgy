//! The axis value-to-fraction mapping and broken axes (#200).

use super::*;

/// The smallest positive range end a logarithmic axis falls back to.
pub const LOG_RANGE_FALLBACK_MIN: f64 = 1.0e-12;

/// The range a probability axis falls back to where its stored range is not
/// a probability range (#209): 0.1 % to 99.9 %.
pub const PROBABILITY_RANGE_FALLBACK: (f64, f64) = (0.001, 0.999);

/// A probability axis range the renderer, the ticks and the annotation space
/// can place values in: both ends strictly inside `(0, 1)` and ordered, else
/// [`PROBABILITY_RANGE_FALLBACK`]. Validation refuses such a range on a drawn
/// chart; this keeps the paths that read an unvalidated range finite.
pub fn guarded_probability_range(min: f64, max: f64) -> (f64, f64) {
    let inside = |p: f64| p.is_finite() && p > 0.0 && p < 1.0;
    if inside(min) && inside(max) && min < max {
        (min, max)
    } else {
        PROBABILITY_RANGE_FALLBACK
    }
}

/// The range of `axis` its scale can place values in: as stored on a linear
/// axis, [`guarded_log_range`] on a logarithmic one and
/// [`guarded_probability_range`] on a probability one.
fn guarded_axis_range(axis: &AxisOptions) -> (f64, f64) {
    match axis.scale {
        AxisScale::Linear => (axis.min, axis.max),
        AxisScale::Logarithmic => guarded_log_range(axis.min, axis.max),
        AxisScale::Probability(_) => guarded_probability_range(axis.min, axis.max),
    }
}

/// A logarithmic axis range the renderer, the ticks and the annotation space
/// can take logarithms of: positive, finite and ordered, with a synthetic
/// decade where the stored range gives none.
pub fn guarded_log_range(min: f64, max: f64) -> (f64, f64) {
    let min = if min.is_finite() && min > 0.0 {
        min
    } else {
        LOG_RANGE_FALLBACK_MIN
    };
    if max.is_finite() && max > min {
        return (min, max);
    }

    let upper = min * 10.0;
    if upper.is_finite() && upper > min {
        return (min, upper);
    }

    // `min` is at the top of f64 and one decade upward overflowed. Preserve a
    // finite, ordered fallback by putting the synthetic decade below it.
    let lower = min / 10.0;
    if lower.is_finite() && lower > 0.0 && lower < min {
        return (lower, min);
    }

    (LOG_RANGE_FALLBACK_MIN, LOG_RANGE_FALLBACK_MIN * 10.0)
}

/// Where `value` falls along its axis, as a fraction of the axis' screen
/// length measured from the axis' visual start, `inverted` included: the one
/// value-to-fraction mapping of the axis chrome, the annotation space and the
/// colourbar ticks.
///
/// **Not clamped**: a tick outside the range lands outside the axis, which is
/// what puts a stale tick visibly off the end instead of piled at it. The
/// colourbar's `ColorBarOptions::normalized_z` is the clamped z→colour twin.
/// A non-finite value, or a non-positive one on a logarithmic axis, has no
/// position and maps to the axis start.
pub fn axis_fraction(value: f64, axis: &AxisOptions, len: f32) -> f32 {
    let t = if axis.categories.is_some() {
        // A categorical axis is linear over its category range.
        let (min, max) = axis.data_range();
        span_fraction(value, min, max, &AxisScale::Linear)
    } else {
        raw_axis_t(value, axis)
    }
    .unwrap_or(0.0);
    let t = BreakMap::of(axis, len).remap(t);
    let t = if axis.inverted { 1.0 - t } else { t };
    t as f32
}

/// Where the raw `t` of `value` (its plain linear or decade fraction of the
/// range, before breaks and `inverted`) falls: the [`span_fraction`] of the
/// axis' scale, `None` for a value with no position. A categorical axis has
/// no breaks.
pub(super) fn raw_axis_t(value: f64, axis: &AxisOptions) -> Option<f64> {
    let (min, max) = guarded_axis_range(axis);
    span_fraction(value, min, max, &axis.scale)
}

/// The value at raw `t` of `axis`' range, the inverse of [`raw_axis_t`].
fn value_at_raw_t(t: f64, axis: &AxisOptions) -> Option<f64> {
    let v = match axis.scale {
        AxisScale::Linear => axis.min + t * (axis.max - axis.min),
        AxisScale::Logarithmic | AxisScale::Probability(_) => {
            let (min, max) = guarded_axis_range(axis);
            let (low, high) = (axis.scale.forward(min)?, axis.scale.forward(max)?);
            axis.scale.inverse(low + t * (high - low))?
        }
    };
    v.is_finite().then_some(v)
}

/// The cuts a top or right axis is drawn with (#200): its partner's
/// (`bottom_x`'s or `left_y`'s) cut ranges at the same raw fractions, in
/// its own units, so the two share one piece layout and the axis draws no
/// tick in a gap. `None` when the partner is unbroken, or `axis` has cuts
/// of its own (an overlay panel's) or either axis is categorical. The
/// partner draws the piece frames, so the mirror does not.
pub fn mirrored_breaks(
    axis: &AxisOptions,
    partner: &AxisOptions,
) -> Option<crate::config::AxisBreaks> {
    let breaks = partner.breaks.as_ref()?;
    if axis.breaks.is_some()
        || axis.categories.is_some()
        || partner.categories.is_some()
        || !BreakMap::cuts(partner).is_broken()
    {
        return None;
    }
    let flip = axis.inverted != partner.inverted;
    let mut ranges = breaks
        .ranges
        .iter()
        .map(|cut| {
            let (a, b) = (raw_axis_t(cut.from, partner)?, raw_axis_t(cut.to, partner)?);
            let (a, b) = if flip { (1.0 - b, 1.0 - a) } else { (a, b) };
            let (from, to) = (value_at_raw_t(a, axis)?, value_at_raw_t(b, axis)?);
            Some(crate::config::AxisBreak::new(from.min(to), from.max(to)))
        })
        .collect::<Option<Vec<_>>>()?;
    ranges.sort_by(|a, b| a.from.total_cmp(&b.from));
    Some(crate::config::AxisBreaks {
        ranges,
        frame_each_panel: false,
        ..breaks.clone()
    })
}

/// Whether `value` lies strictly inside one of `axis`' cut ranges (#200),
/// clamped to the range as [`BreakMap::of`] clamps them: such a value is not
/// drawn, so no tick, grid line or tick label is placed at it.
pub fn value_in_cut(axis: &AxisOptions, value: f64) -> bool {
    let map = BreakMap::cuts(axis);
    if !map.is_broken() {
        return false;
    }
    raw_axis_t(value, axis).is_some_and(|t| map.knots().iter().any(|k| t > k.from && t < k.to))
}

/// One cut of a broken axis in `t` space (#200): the raw interval
/// `from..to` of the cut values is drawn as the gap `start..end`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BreakKnot {
    pub from: f64,
    pub to: f64,
    pub start: f64,
    pub end: f64,
}

/// The mapping of a broken axis (#200) from the raw `t` of a value (its
/// plain fraction of the range) to where it is drawn, before `inverted`:
/// linear within each piece, every piece scaled alike so that the pieces
/// and the gaps fill the axis, and linear across each gap so that a line
/// crossing it stays straight. The identity on an axis without breaks, and
/// on one whose cuts leave nothing to draw.
///
/// The renderer's transform remaps by the same knots
/// ([`Self::knots`]) and draws each piece scissored to its band, so the
/// shader and the chrome place a value alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreakMap {
    knots: [BreakKnot; crate::config::MAX_AXIS_BREAKS],
    count: usize,
    scale: f64,
    /// Each piece's drawn length per raw length, in axis order (#205):
    /// all `scale` when the pieces are sized by value.
    slopes: [f64; crate::config::MAX_AXIS_BREAKS + 1],
}

impl BreakMap {
    /// The identity map.
    pub const IDENTITY: Self = Self {
        knots: [BreakKnot {
            from: 0.0,
            to: 0.0,
            start: 0.0,
            end: 0.0,
        }; crate::config::MAX_AXIS_BREAKS],
        count: 0,
        scale: 1.0,
        slopes: [1.0; crate::config::MAX_AXIS_BREAKS + 1],
    };

    /// The map of `axis` drawn `len` logical pixels long (the data area's
    /// width for an x axis, its height for a y axis): its cuts clamped to
    /// the range, the ones outside it dropped, each gap
    /// [`AxisBreaks::gap_px`](crate::config::AxisBreaks::gap_px) of `len`
    /// (#249). A categorical axis, a value range without a position, cuts
    /// past [`MAX_AXIS_BREAKS`](crate::config::MAX_AXIS_BREAKS) (refused by
    /// validation) and gaps that leave the pieces no room leave the
    /// identity.
    pub fn of(axis: &AxisOptions, len: f32) -> Self {
        let Some(breaks) = axis.breaks.as_ref() else {
            return Self::IDENTITY;
        };
        if !(len.is_finite() && len > 0.0) {
            return Self::IDENTITY;
        }
        let gap = f64::from(
            breaks
                .gap_px
                .clamp(0.0, crate::config::MAX_AXIS_BREAK_GAP_PX),
        ) / f64::from(len);
        Self::with_gap(axis, gap)
    }

    /// Where `axis`' cuts lie, the gaps aside: the map of [`Self::of`] with
    /// gaps of no width (#249). What depends only on the cut ranges and the
    /// sizing reads it without the axis' drawn length: whether the axis is
    /// cut, the pieces' raw spans, the tick plan.
    pub fn cuts(axis: &AxisOptions) -> Self {
        Self::with_gap(axis, 0.0)
    }

    /// The map of `axis` with each gap `gap` of the axis length.
    fn with_gap(axis: &AxisOptions, gap: f64) -> Self {
        let Some(breaks) = axis.breaks.as_ref() else {
            return Self::IDENTITY;
        };
        if axis.categories.is_some() || breaks.ranges.len() > crate::config::MAX_AXIS_BREAKS {
            return Self::IDENTITY;
        }
        let mut map = Self::IDENTITY;
        let mut removed = 0.0;
        let mut last = 0.0f64;
        for cut in &breaks.ranges {
            let (Some(a), Some(b)) = (raw_axis_t(cut.from, axis), raw_axis_t(cut.to, axis)) else {
                continue;
            };
            let (a, b) = (a.min(b).max(last), a.max(b).min(1.0));
            if !(b > a) {
                continue;
            }
            map.knots[map.count] = BreakKnot {
                from: a,
                to: b,
                start: 0.0,
                end: 0.0,
            };
            map.count += 1;
            removed += b - a;
            last = b;
        }
        let visible = 1.0 - removed;
        let gaps = gap * map.count as f64;
        if map.count == 0 || !(visible > 1e-9) || !(1.0 - gaps > 1e-9) {
            return Self::IDENTITY;
        }
        map.scale = (1.0 - gaps) / visible;
        // Each piece's drawn length: by its span, the same for all, or by
        // its weight (#205). A piece with no span (a cut at an end of the
        // range) gets none; weights that no longer match the pieces (a cut
        // dropped outside the range) fall back to the spans.
        let pieces = map.count + 1;
        let mut spans = [0.0f64; crate::config::MAX_AXIS_BREAKS + 1];
        {
            let mut raw = 0.0;
            for (k, knot) in map.knots[..map.count].iter().enumerate() {
                spans[k] = knot.from - raw;
                raw = knot.to;
            }
            spans[map.count] = 1.0 - raw;
        }
        let weight = |k: usize| -> f64 {
            if spans[k] <= 1e-12 {
                return 0.0;
            }
            match &breaks.sizing {
                crate::config::PieceSizing::ByValue => spans[k],
                crate::config::PieceSizing::Equal => 1.0,
                crate::config::PieceSizing::Weights(w) if w.len() == pieces => f64::from(w[k]),
                crate::config::PieceSizing::Weights(_) => spans[k],
            }
        };
        let total: f64 = (0..pieces).map(weight).sum();
        if !(total > 0.0) {
            return Self::IDENTITY;
        }
        for k in 0..pieces {
            let length = weight(k) / total * (1.0 - gaps);
            map.slopes[k] = if spans[k] > 1e-12 {
                length / spans[k]
            } else {
                map.scale
            };
        }
        // Walk the pieces: each knot's gap starts where the piece before it
        // ends.
        let (mut raw, mut drawn) = (0.0, 0.0);
        for (k, knot) in map.knots[..map.count].iter_mut().enumerate() {
            knot.start = drawn + (knot.from - raw) * map.slopes[k];
            knot.end = knot.start + gap;
            raw = knot.to;
            drawn = knot.end;
        }
        map
    }

    /// The cuts in `t` space, in axis order.
    pub fn knots(&self) -> &[BreakKnot] {
        &self.knots[..self.count]
    }

    /// The scale of the pieces when they are sized by value: drawn length
    /// per raw length, the same in every piece. A `Panels` axis, which is
    /// always sized by value, maps every piece by it.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The scale of the piece raw `t` lies in (#205): drawn length per raw
    /// length there. Inside a cut, the piece after it.
    pub fn scale_at(&self, t: f64) -> f64 {
        let piece = self.knots().iter().take_while(|k| t >= k.from).count();
        self.slopes[piece.min(self.count)]
    }

    /// Whether the pieces have scales of their own (#205): they are not all
    /// `scale`.
    pub fn per_piece(&self) -> bool {
        self.slopes[..=self.count]
            .iter()
            .any(|s| (s - self.scale).abs() > 1e-12 * self.scale.abs().max(1.0))
    }

    /// Whether the axis has a cut inside its range.
    pub fn is_broken(&self) -> bool {
        self.count > 0
    }

    /// Where raw `t` is drawn. Monotonic and continuous; outside `0..1` the
    /// first and last pieces extend.
    pub fn remap(&self, t: f64) -> f64 {
        let (mut raw, mut drawn, mut piece) = (0.0, 0.0, 0);
        for knot in self.knots() {
            if t < knot.from {
                break;
            }
            if t < knot.to {
                return knot.start
                    + (t - knot.from) / (knot.to - knot.from) * (knot.end - knot.start);
            }
            raw = knot.to;
            drawn = knot.end;
            piece += 1;
        }
        drawn + (t - raw) * self.slopes[piece]
    }

    /// The raw `t` drawn at `drawn`, the inverse of [`Self::remap`], or
    /// `None` inside a gap, where no value is drawn.
    pub fn unmap(&self, drawn: f64) -> Option<f64> {
        let (mut raw, mut at, mut piece) = (0.0, 0.0, 0);
        for knot in self.knots() {
            if drawn < knot.start {
                break;
            }
            if drawn <= knot.end {
                return None;
            }
            raw = knot.to;
            at = knot.end;
            piece += 1;
        }
        Some(raw + (drawn - at) / self.slopes[piece])
    }

    /// The drawn bands of the pieces, `start..end` in axis order before
    /// `inverted`: `0..1` on an unbroken axis.
    pub fn pieces(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        let starts = std::iter::once(0.0).chain(self.knots().iter().map(|k| k.end));
        let ends = self
            .knots()
            .iter()
            .map(|k| k.start)
            .chain(std::iter::once(1.0));
        starts.zip(ends)
    }

    /// The raw `t` span of each piece, `from..to` in axis order: `0..1` on
    /// an unbroken axis.
    pub fn raw_pieces(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        let starts = std::iter::once(0.0).chain(self.knots().iter().map(|k| k.to));
        let ends = self
            .knots()
            .iter()
            .map(|k| k.from)
            .chain(std::iter::once(1.0));
        starts.zip(ends)
    }
}

/// Where `value` falls between `min` and `max` on `scale`: linearly, by
/// decades, or by `F⁻¹(p)` on a probability axis ([`AxisScale::forward`]).
/// The formula under [`axis_fraction`] and the annotation space. `None` when
/// the value has no position: a non-finite value, an empty or non-finite
/// span, a non-positive value or bound on a logarithmic span, or one outside
/// `(0, 1)` on a probability span.
pub fn span_fraction(value: f64, min: f64, max: f64, scale: &AxisScale) -> Option<f64> {
    let (value, min, max) = (
        scale.forward(value)?,
        scale.forward(min)?,
        scale.forward(max)?,
    );
    let span = max - min;
    if !span.is_finite() || span == 0.0 {
        return None;
    }
    Some((value - min) / span)
}

#[cfg(test)]
mod break_map_tests {
    //! The broken-axis map (#200): the one value-to-fraction mapping the
    //! chrome, the annotations and the renderer's transform share.
    use super::{BreakMap, axis_fraction};
    use crate::config::{AxisBreak, AxisBreakStyle, AxisBreaks, AxisOptions, AxisScale};

    /// The drawn length of the axes these tests map (#249): a gap of
    /// `gap` of it is `gap * LEN` pixels.
    const LEN: f32 = 1000.0;

    fn axis(min: f64, max: f64, cuts: &[(f64, f64)], gap: f32) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_y();
        axis.min = min;
        axis.max = max;
        axis.breaks = Some(AxisBreaks {
            ranges: cuts.iter().map(|&(a, b)| AxisBreak::new(a, b)).collect(),
            gap_px: gap * LEN,
            ..crate::defaults::axis_breaks()
        });
        axis
    }

    #[test]
    fn one_cut_splits_the_axis_into_two_pieces_and_a_gap() {
        // 0..100 without 10..90: two pieces of 10 each share 1 - 0.1.
        let a = axis(0.0, 100.0, &[(10.0, 90.0)], 0.1);
        let map = BreakMap::of(&a, LEN);
        assert!(map.is_broken());
        // The gap is an f32 fraction, so compare at its precision.
        let close = |x: f64, y: f64| (x - y).abs() < 1e-6;
        assert!(close(map.scale(), 0.9 / 0.2));
        let pieces: Vec<_> = map.pieces().collect();
        assert_eq!(pieces.len(), 2);
        assert!(close(pieces[0].0, 0.0) && close(pieces[0].1, 0.45));
        assert!(close(pieces[1].0, 0.55) && close(pieces[1].1, 1.0));
        assert!(close(f64::from(axis_fraction(5.0, &a, LEN)), 0.225));
        assert!(close(f64::from(axis_fraction(10.0, &a, LEN)), 0.45));
        assert!(close(f64::from(axis_fraction(90.0, &a, LEN)), 0.55));
        assert!((f64::from(axis_fraction(95.0, &a, LEN)) - 0.775).abs() < 1e-6);
        // Inside the cut the value moves straight across the gap.
        assert!((f64::from(axis_fraction(50.0, &a, LEN)) - 0.5).abs() < 1e-6);
    }

    fn sized(mut a: AxisOptions, sizing: crate::config::PieceSizing) -> AxisOptions {
        if let Some(breaks) = a.breaks.as_mut() {
            breaks.sizing = sizing;
        }
        a
    }

    // #205: weights share the axis length among the pieces, each piece at
    // its own scale; remap and unmap stay inverse, and the slope of the
    // piece a value lies in is its scale.
    #[test]
    fn weighted_pieces_take_their_share_at_their_own_scale() {
        use crate::config::PieceSizing;
        let close = |x: f64, y: f64| (x - y).abs() < 1e-6;
        // 0..100 without 10..80: pieces of 10 and 20 raw units.
        let a = sized(
            axis(0.0, 100.0, &[(10.0, 80.0)], 0.1),
            PieceSizing::Weights(vec![1.0, 3.0]),
        );
        let map = BreakMap::of(&a, LEN);
        assert!(map.per_piece());
        let pieces: Vec<_> = map.pieces().collect();
        // 0.9 of the axis split 1 : 3.
        assert!(close(pieces[0].1 - pieces[0].0, 0.225), "{pieces:?}");
        assert!(close(pieces[1].1 - pieces[1].0, 0.675), "{pieces:?}");
        assert!(close(map.scale_at(0.05), 0.225 / 0.1));
        assert!(close(map.scale_at(0.9), 0.675 / 0.2));
        for t in [0.0, 0.03, 0.099, 0.81, 0.95, 1.0] {
            let drawn = map.remap(t);
            assert!(close(map.unmap(drawn).unwrap(), t), "{t}");
        }
        assert!(map.unmap(0.25).is_none(), "inside the gap");
        // Equal pieces: the same length whatever their spans.
        let e = BreakMap::of(
            &sized(axis(0.0, 100.0, &[(10.0, 80.0)], 0.1), PieceSizing::Equal),
            LEN,
        );
        let p: Vec<_> = e.pieces().collect();
        assert!(close(p[0].1 - p[0].0, p[1].1 - p[1].0));
        // By value: one scale, as before.
        let v = BreakMap::of(&axis(0.0, 100.0, &[(10.0, 80.0)], 0.1), LEN);
        assert!(!v.per_piece());
        // Weights that no longer match the pieces (a cut outside the range)
        // fall back to the spans.
        let dropped = BreakMap::of(
            &sized(
                axis(0.0, 100.0, &[(10.0, 80.0), (200.0, 300.0)], 0.1),
                PieceSizing::Weights(vec![1.0, 3.0, 5.0]),
            ),
            LEN,
        );
        assert!(!dropped.per_piece());
    }

    // #205: each weighted piece walks its own tick spacing.
    #[test]
    fn weighted_pieces_walk_their_own_ticks() {
        use crate::config::PieceSizing;
        let mut a = sized(
            axis(-0.15, 25.2, &[(0.25, 19.6)], 0.04),
            PieceSizing::Weights(vec![1.0, 9.0]),
        );
        a.major_spacing = 5.0;
        let (pieces, count) = crate::tick::piece_ticks(&a).expect("pieces of their own");
        assert_eq!(count, 2);
        // The thin ground piece keeps the axis' spacing (one major, 0); the
        // tall one takes its own, a major per unit.
        assert_eq!(pieces[0].spacing, 5.0, "{:?}", &pieces[..count]);
        assert_eq!(pieces[1].spacing, 1.0, "{:?}", &pieces[..count]);
        let majors: Vec<f64> = crate::tick::major_ticks(&a).collect();
        assert_eq!(majors, vec![0.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0]);
        assert!(!majors.iter().any(|v| *v > 0.25 && *v < 19.6), "{majors:?}");
        // Sized by value, the axis walks one grid.
        assert!(crate::tick::piece_ticks(&axis(-0.15, 25.2, &[(0.25, 19.6)], 0.04)).is_none());
    }

    #[test]
    fn piece_sizing_is_validated() {
        use crate::config::{AxisBreakStyle, PieceSizing};
        let ok = sized(
            axis(0.0, 100.0, &[(10.0, 80.0)], 0.1),
            PieceSizing::Weights(vec![1.0, 2.0]),
        );
        assert!(ok.validate_breaks().is_ok());
        for bad in [vec![1.0], vec![1.0, 0.0], vec![1.0, f32::NAN]] {
            let a = sized(
                axis(0.0, 100.0, &[(10.0, 80.0)], 0.1),
                PieceSizing::Weights(bad),
            );
            assert!(a.validate_breaks().is_err());
        }
        let mut panels = sized(axis(0.0, 100.0, &[(10.0, 80.0)], 0.1), PieceSizing::Equal);
        panels.breaks.as_mut().unwrap().style = AxisBreakStyle::Panels;
        assert!(panels.validate_breaks().is_err());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn piece_sizing_serializes_only_when_set() {
        use crate::config::PieceSizing;
        let by_value = axis(0.0, 100.0, &[(10.0, 80.0)], 0.1).breaks.unwrap();
        let json = serde_json::to_value(&by_value).unwrap();
        assert!(json.get("sizing").is_none(), "{json}");
        let weighted = crate::config::AxisBreaks {
            sizing: PieceSizing::Weights(vec![1.0, 9.0]),
            ..by_value
        };
        let json = serde_json::to_value(&weighted).unwrap();
        assert_eq!(json["sizing"]["weights"], serde_json::json!([1.0, 9.0]));
        let back: crate::config::AxisBreaks = serde_json::from_value(json).unwrap();
        assert_eq!(back, weighted);
    }

    #[test]
    fn remap_is_monotonic_continuous_and_unmap_inverts_it_outside_the_gaps() {
        let a = axis(
            -5.0,
            120.0,
            &[(1.0, 20.0), (30.0, 31.0), (60.0, 100.0)],
            0.04,
        );
        let map = BreakMap::of(&a, LEN);
        assert_eq!(map.knots().len(), 3);
        let mut last = f64::NEG_INFINITY;
        for i in -100..=1100 {
            let t = f64::from(i) / 1000.0;
            let drawn = map.remap(t);
            assert!(drawn >= last - 1e-12, "not monotonic at {t}");
            last = drawn;
            match map.unmap(drawn) {
                Some(back) => assert!((back - t).abs() < 1e-9, "{t} -> {drawn} -> {back}"),
                None => assert!(map.knots().iter().any(|k| t >= k.from && t <= k.to)),
            }
        }
        // The pieces and the gaps fill the axis exactly.
        let (pieces, gaps): (f64, f64) = (
            map.pieces().map(|(a, b)| b - a).sum(),
            map.knots().iter().map(|k| k.end - k.start).sum(),
        );
        assert!((pieces + gaps - 1.0).abs() < 1e-12);
        assert!((map.remap(1.0) - 1.0).abs() < 1e-12 && map.remap(0.0).abs() < 1e-12);
    }

    #[test]
    fn a_logarithmic_axis_is_cut_in_decades() {
        let mut a = axis(1.0, 1e6, &[(10.0, 1e4)], 0.0);
        a.scale = AxisScale::Logarithmic;
        let map = BreakMap::of(&a, LEN);
        // Six decades without three leave one decade and two: a third and
        // two thirds of the axis.
        let pieces: Vec<_> = map.pieces().collect();
        assert!((pieces[0].1 - 1.0 / 3.0).abs() < 1e-12, "{pieces:?}");
        assert!((f64::from(axis_fraction(1e5, &a, LEN)) - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn cuts_outside_the_range_do_nothing_and_ends_are_clamped() {
        let a = axis(0.0, 10.0, &[(20.0, 30.0)], 0.1);
        assert_eq!(BreakMap::of(&a, LEN), BreakMap::IDENTITY);
        assert!((f64::from(axis_fraction(5.0, &a, LEN)) - 0.5).abs() < 1e-7);
        // A cut over the lower end leaves one piece and one gap.
        let a = axis(0.0, 10.0, &[(-5.0, 2.0)], 0.1);
        let map = BreakMap::of(&a, LEN);
        assert_eq!(map.knots().len(), 1);
        assert!(map.knots()[0].from == 0.0 && map.knots()[0].start == 0.0);
        // A cut over the whole range leaves nothing to draw: the identity.
        let a = axis(0.0, 10.0, &[(-5.0, 20.0)], 0.1);
        assert_eq!(BreakMap::of(&a, LEN), BreakMap::IDENTITY);
    }

    #[test]
    fn both_styles_share_the_piece_layout_and_inverted_mirrors_it() {
        let mut a = axis(0.0, 100.0, &[(10.0, 90.0)], 0.1);
        let continuous = axis_fraction(95.0, &a, LEN);
        a.breaks.as_mut().unwrap().style = AxisBreakStyle::Panels;
        assert_eq!(axis_fraction(95.0, &a, LEN), continuous);
        a.inverted = true;
        assert!((axis_fraction(95.0, &a, LEN) - (1.0 - continuous)).abs() < 1e-6);
    }

    #[test]
    fn an_axis_without_breaks_maps_exactly_as_before() {
        let mut a = crate::defaults::axis_options_y();
        a.min = -3.0;
        a.max = 7.0;
        let before = axis_fraction(1.5, &a, LEN);
        a.breaks = Some(crate::defaults::axis_breaks());
        assert_eq!(axis_fraction(1.5, &a, LEN), before);
        assert_eq!(BreakMap::of(&a, LEN), BreakMap::IDENTITY);
    }
}
