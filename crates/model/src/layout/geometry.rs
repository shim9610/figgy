use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, BarAlign, ColorBarOptions, Config, LegendCorner};
use crate::format::LabelFormat;
use crate::sink::{Border, DrawnPart, Parts, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, TextExtents};

use super::{ChartArea, DataArea, Element, RectF, Side, axis_mut, axis_ref, tick_contribution};

/// Minimum gap between a tick end and its label, in pixels.
pub const LABEL_GAP: f32 = 4.0;

/// Width of the legend box border, stroked centred on the box edge.
pub const LEGEND_BORDER_WIDTH: f32 = 1.0;
/// Inset between a legend box and its data-area corner, in pixels.
pub const LEGEND_INSET: f32 = 6.0;

/// Axis-line endpoints on pixel centers of the plot rectangle `plot`.
/// Top/left use the first inside pixel; bottom/right use the last inside
/// pixel. A data area's rectangle is `RectF::from_rect(&data_area)`, which
/// is exact; a plot rectangle that is not integer-aligned takes the same
/// half-pixel insets.
///
/// `cross` is where the line crosses the perpendicular axis, as that axis'
/// fraction ([`axis_cross`]): the line then runs through the pixel centre
/// under that fraction, clamped between the two edge lines. `None` keeps it
/// on its own edge.
pub fn axis_anchor(side: Side, plot: &RectF, cross: Option<f32>) -> ((f32, f32), (f32, f32)) {
    let x0 = plot.x + 0.5;
    let y0 = plot.y + 0.5;
    let x1 = plot.x + plot.width - 0.5;
    let y1 = plot.y + plot.height - 0.5;
    // The pixel centre a crossing lands in, kept between the edge lines.
    let snap = |v: f32, lo: f32, hi: f32| (v.floor() + 0.5).max(lo).min(hi);
    match (side, cross) {
        (Side::Top | Side::Bottom, Some(t)) => {
            let (_, y) = point_on_rect_side(t, &Side::Left, plot);
            let y = snap(y, y0, y1);
            ((x0, y), (x1, y))
        }
        (Side::Left | Side::Right, Some(t)) => {
            let (x, _) = point_on_rect_side(t, &Side::Bottom, plot);
            let x = snap(x, x0, x1);
            ((x, y0), (x, y1))
        }
        (Side::Top, None) => ((x0, y0), (x1, y0)),
        (Side::Bottom, None) => ((x0, y1), (x1, y1)),
        (Side::Left, None) => ((x0, y0), (x0, y1)),
        (Side::Right, None) => ((x1, y0), (x1, y1)),
    }
}

/// Where the axis on `side` crosses its perpendicular axis, as that axis'
/// fraction clamped to `0..=1`: the position of
/// [`AxisOptions::cross_at`] on `left_y` for the top and bottom axes, on
/// `bottom_x` for the left and right ones. `None` without a crossing, for a
/// value with no position on that axis, and on a polar chart.
pub fn axis_cross(config: &Config, side: &Side) -> Option<f32> {
    if config.polar().is_some() {
        return None;
    }
    let (axis, perpendicular) = match side {
        Side::Top => (&config.top_x, &config.left_y),
        Side::Bottom => (&config.bottom_x, &config.left_y),
        Side::Left => (&config.left_y, &config.bottom_x),
        Side::Right => (&config.right_y, &config.bottom_x),
    };
    let value = axis.cross_at?;
    let t = if perpendicular.categories.is_some() {
        let (min, max) = perpendicular.data_range();
        span_fraction(value, min, max, &AxisScale::Linear)
    } else {
        raw_axis_t(value, perpendicular)
    }?;
    let t = if perpendicular.inverted { 1.0 - t } else { t };
    Some(t.clamp(0.0, 1.0) as f32)
}

/// Perpendicular screen translation for a detached axis.
pub fn axis_offset(side: Side, line_offset: f32) -> (f32, f32) {
    match side {
        Side::Left | Side::Right => (line_offset, 0.0),
        Side::Top | Side::Bottom => (0.0, line_offset),
    }
}

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
fn raw_axis_t(value: f64, axis: &AxisOptions) -> Option<f64> {
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

/// The colourbar and its strip rect when it is drawn: present, visible and
/// not degenerate. The renderer draws the bar, its chrome and its title only
/// then, and the strip, axis, label and title elements exist only then.
pub fn visible_colorbar_rect(cfg: &Config) -> Option<(&ColorBarOptions, RectF)> {
    let bar = cfg.colorbar.as_ref()?;
    if !bar.visible {
        return None;
    }
    let da = cfg.data_area().ok()?;
    let rect = colorbar_rect(&cfg.chart_area, &da, cfg.chart_title.top_margin, bar);
    (rect.width > 0.0 && rect.height > 0.0).then_some((bar, rect))
}

impl Config {
    /// The margin geometry the axis title on `side` is placed inside.
    pub fn title_band(&self, side: &Side) -> TitleBand {
        let axis = match side {
            Side::Top => &self.top_x,
            Side::Bottom => &self.bottom_x,
            Side::Left => &self.left_y,
            Side::Right => &self.right_y,
        };
        TitleBand {
            out_margin: axis.out_margin,
            chart_title_margin: self.chart_title.top_margin,
            edge_inset: self.colorbar_band(side),
        }
    }
}

/// Place a fraction on one side of a rectangle. Horizontal sides run
/// left-to-right; vertical sides run bottom-to-top, matching numeric axes.
/// The colourbar renderer, label hit geometry, and tests all use this one
/// mapping rather than independently deciding which screen end is low.
pub fn point_on_rect_side(t: f32, side: &Side, rect: &RectF) -> (f32, f32) {
    match side {
        Side::Top => (rect.x + t * rect.width, rect.y),
        Side::Bottom => (rect.x + t * rect.width, rect.y + rect.height),
        Side::Left => (rect.x, rect.y + rect.height - t * rect.height),
        Side::Right => (rect.x + rect.width, rect.y + rect.height - t * rect.height),
    }
}

/// Baseline origin for one measured tick label.
pub fn label_origin(
    side: Side,
    tick_position: (f32, f32),
    outward: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> (f32, f32) {
    let (x, y) = match side {
        Side::Top => (
            tick_position.0 - extents.width * 0.5,
            tick_position.1 - outward - LABEL_GAP - extents.descent,
        ),
        Side::Bottom => (
            tick_position.0 - extents.width * 0.5,
            tick_position.1 + outward + LABEL_GAP + extents.ascent,
        ),
        Side::Left => (
            tick_position.0 - outward - LABEL_GAP - extents.width,
            tick_position.1 + (extents.ascent - extents.descent) * 0.5,
        ),
        Side::Right => (
            tick_position.0 + outward + LABEL_GAP,
            tick_position.1 + (extents.ascent - extents.descent) * 0.5,
        ),
    };
    (x + offset.0, y + offset.1)
}

/// Pixel rectangle occupied by a label drawn at a baseline origin.
pub fn label_rect(origin: (f32, f32), extents: TextExtents) -> RectF {
    RectF {
        x: origin.0,
        y: origin.1 - extents.ascent,
        width: extents.width,
        height: extents.height(),
    }
}

/// Draw origin and optional rotation for a chart or axis title.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitlePlacement {
    pub origin: (f32, f32),
    pub rotation_center: (f32, f32),
    pub rotation_degrees: f32,
}

impl TitlePlacement {
    /// Axis-aligned screen rectangle of the placed text envelope, rotated
    /// by `rotation_degrees` (clockwise on screen) about `rotation_center`.
    pub fn rect(self, extents: TextExtents) -> RectF {
        let local = label_rect(self.origin, extents);
        match self.rotation_degrees {
            -90.0 => RectF {
                x: self.rotation_center.0 + (local.y - self.rotation_center.1),
                y: self.rotation_center.1 - (local.x + local.width - self.rotation_center.0),
                width: local.height,
                height: local.width,
            },
            90.0 => RectF {
                x: self.rotation_center.0 - (local.y + local.height - self.rotation_center.1),
                y: self.rotation_center.1 + (local.x - self.rotation_center.0),
                width: local.height,
                height: local.width,
            },
            0.0 => local,
            // Any other angle (a ternary chart's slanted titles): the
            // bounding box of the rectangle turned about the centre.
            degrees => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                let (cx, cy) = self.rotation_center;
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for (x, y) in [
                    (local.x, local.y),
                    (local.x + local.width, local.y),
                    (local.x, local.y + local.height),
                    (local.x + local.width, local.y + local.height),
                ] {
                    let (dx, dy) = (x - cx, y - cy);
                    let (rx, ry) = (cx + cos * dx - sin * dy, cy + sin * dx + cos * dy);
                    x0 = x0.min(rx);
                    y0 = y0.min(ry);
                    x1 = x1.max(rx);
                    y1 = y1.max(ry);
                }
                RectF {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                }
            }
        }
    }
}

pub fn chart_title_placement(
    chart_area: &ChartArea,
    top_margin: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    chart_title_placement_in(RectF::from_rect(chart_area), top_margin, offset, extents)
}

/// [`chart_title_placement`] in an `f32` rectangle: a chart that lays out
/// its own title band (the radial chart) centres its title across its
/// canvas with it.
pub fn chart_title_placement_in(
    chart_area: RectF,
    top_margin: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    let origin = (
        chart_area.x + chart_area.width * 0.5 - extents.width * 0.5 + offset.0,
        chart_area.y + (top_margin - extents.height()) * 0.5 + extents.ascent + offset.1,
    );
    TitlePlacement {
        origin,
        rotation_center: origin,
        rotation_degrees: 0.0,
    }
}

/// Room the default title band leaves around one default-size title line: the
/// default `top_margin` minus that line's nominal height, its font size (the
/// em box the renderer also gives a line without ink). Derived rather than
/// restated, so fitted bands follow the defaults if they change.
fn chart_title_band_padding() -> f32 {
    let defaults = crate::default::default_chart_title_options();
    (defaults.top_margin - defaults.text.font_size).max(0.0)
}

/// Grow `cfg.chart_title.top_margin` when the measured title is taller than
/// its band.
///
/// [`chart_title_placement`] centres the title block in the band, so a block
/// taller than the band starts above the chart area and its first line is
/// clipped. Only in that case the band grows to the block's height plus the
/// room the default band gives a single default-size line, rounded up to a
/// whole pixel. A title that already fits is left exactly as it is: text is
/// measured by its ink, so refitting every title would move the data area
/// whenever the title's letters changed.
///
/// The band is never shrunk, so a margin the host made larger is kept.
/// Nothing changes when the title is hidden or empty, when the measurement is
/// not finite, when the margin is NaN (validation reports it), or when the
/// taller band would leave no valid data area in a chart that had one.
///
/// This is the title helpers' half of the helper contract: `Chart::with_title`
/// and the browser wrapper's `set_title` derive the band from the text they set.
/// A `top_margin` injected directly through the SSoT is kept as given.
/// Returns whether the band changed.
pub fn fit_chart_title_margin(cfg: &mut Config, measure: &dyn MeasureText) -> bool {
    let title = &cfg.chart_title;
    if !title.visible || title.text.segments.is_empty() {
        return false;
    }
    let height = measure.measure_rich(&title.text).height();
    let current = title.top_margin;
    if !height.is_finite() || current.is_nan() || height <= current {
        return false;
    }
    let required = (height + chart_title_band_padding()).ceil();
    let had_data_area = cfg.data_area().is_ok();
    cfg.chart_title.top_margin = required;
    if had_data_area && cfg.data_area().is_err() {
        cfg.chart_title.top_margin = current;
        return false;
    }
    true
}

/// The margin geometry an axis title is placed inside.
///
/// A struct rather than three loose `f32` parameters: they are all pixel
/// lengths measured along the same direction, and a positional swap between
/// them would move a title without failing to compile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleBand {
    /// The axis' own outer margin — the band its title and tick labels share.
    pub out_margin: f32,
    /// The chart title band at the top of the chart area. Only `Side::Top` sits
    /// below it; the other sides ignore this.
    pub chart_title_margin: f32,
    /// How far this side's axis band is pushed in from the chart-area edge by
    /// whatever occupies the outside of that margin — today the colourbar band
    /// ([`Config::colorbar_band`](crate::config::Config::colorbar_band)), zero on
    /// the other three sides. Without it the placement assumes the axis band
    /// touches the chart edge, and a title on the colourbar's side would be
    /// drawn on top of the bar.
    pub edge_inset: f32,
}

/// Where an axis title sits inside its side's margin.
pub fn axis_title_placement(
    side: Side,
    chart_area: &ChartArea,
    data_area: &DataArea,
    band: TitleBand,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    let TitleBand {
        out_margin,
        chart_title_margin,
        edge_inset,
    } = band;
    match side {
        Side::Top | Side::Bottom => {
            let band_top = if matches!(side, Side::Top) {
                chart_area.y as f32 + chart_title_margin + edge_inset
            } else {
                (chart_area.y + chart_area.height) as f32 - out_margin - edge_inset
            };
            let origin = (
                data_area.x as f32 + data_area.width as f32 * 0.5 - extents.width * 0.5 + offset.0,
                band_top + (out_margin - extents.height()) * 0.5 + extents.ascent + offset.1,
            );
            TitlePlacement {
                origin,
                rotation_center: origin,
                rotation_degrees: 0.0,
            }
        }
        Side::Left | Side::Right => {
            let center = (
                if matches!(side, Side::Left) {
                    chart_area.x as f32 + edge_inset + out_margin * 0.5
                } else {
                    (chart_area.x + chart_area.width) as f32 - edge_inset - out_margin * 0.5
                },
                data_area.y as f32 + data_area.height as f32 * 0.5,
            );
            TitlePlacement {
                origin: (
                    center.0 - extents.width * 0.5 + offset.0,
                    center.1 + (extents.ascent - extents.descent) * 0.5 + offset.1,
                ),
                rotation_center: center,
                rotation_degrees: if matches!(side, Side::Left) {
                    -90.0
                } else {
                    90.0
                },
            }
        }
    }
}

/// Place a colourbar title from the actual painted strip rectangle.
///
/// The title sits in `axis.out_margin`, immediately beyond the strip's major
/// tick reach, and is centred along the strip's own long dimension. Therefore
/// `length_frac`, `align`, `offset_{x,y}`, and resize all move the title through
/// the same rectangle that the renderer paints and the hit map selects.
///
/// `labels_reach` is how far past the strip's outer edge the tick labels
/// reach ([`colorbar_labels_reach`](crate::chrome::colorbar_labels_reach),
/// zero without labels). A title that the band's centre would put over the
/// labels sits a small gap past them instead, as far as the band reaches
/// (#259).
pub fn colorbar_title_placement(
    side: Side,
    strip: &RectF,
    axis: &AxisOptions,
    offset: (f32, f32),
    extents: TextExtents,
    labels_reach: f32,
) -> TitlePlacement {
    let tick = axis.major_tick_length.max(0.0);
    let out_margin = axis.out_margin.max(0.0);
    // The room kept between the labels and the title, and between the title
    // and the band's outer edge: a third of the label font, the clearance
    // `auto_layout` keeps at the default 12 px labels, so a scaled export
    // keeps the 1x proportions.
    let gap = axis.label_style.font_size.max(0.0) / 3.0;
    let clear = if labels_reach.is_finite() && labels_reach > 0.0 {
        labels_reach + gap
    } else {
        0.0
    };
    // How far past the strip's outer edge the title's inner edge sits:
    // centred in the band, or past the labels, but no closer to the band's
    // outer edge (where the chart area may end) than the gap.
    let centred = tick + (out_margin - extents.height()) * 0.5;
    let inner = centred.max(clear.min(tick + out_margin - extents.height() - gap));
    match side {
        Side::Top | Side::Bottom => {
            let title_top = if matches!(side, Side::Top) {
                strip.y - inner - extents.height()
            } else {
                strip.y + strip.height + inner
            };
            let origin = (
                strip.x + strip.width * 0.5 - extents.width * 0.5 + offset.0,
                title_top + extents.ascent + offset.1,
            );
            TitlePlacement {
                origin,
                rotation_center: origin,
                rotation_degrees: 0.0,
            }
        }
        Side::Left | Side::Right => {
            // The rotated title's thickness across the strip is its height.
            let reach = inner + extents.height() * 0.5;
            let center = (
                if matches!(side, Side::Left) {
                    strip.x - reach
                } else {
                    strip.x + strip.width + reach
                },
                strip.y + strip.height * 0.5,
            );
            TitlePlacement {
                origin: (
                    center.0 - extents.width * 0.5 + offset.0,
                    center.1 + (extents.ascent - extents.descent) * 0.5 + offset.1,
                ),
                rotation_center: center,
                rotation_degrees: if matches!(side, Side::Left) {
                    -90.0
                } else {
                    90.0
                },
            }
        }
    }
}

/// Convert an axis-title local offset to its rotated screen-space offset.
pub fn axis_title_offset_to_screen(side: Side, offset: (f32, f32)) -> (f32, f32) {
    match side {
        Side::Left => (offset.1, -offset.0),
        Side::Right => (-offset.1, offset.0),
        Side::Top | Side::Bottom => offset,
    }
}

/// Convert a screen-space drag delta to an axis title's local offset frame.
pub fn screen_offset_to_axis_title(side: Side, offset: (f32, f32)) -> (f32, f32) {
    match side {
        Side::Left => (-offset.1, offset.0),
        Side::Right => (offset.1, -offset.0),
        Side::Top | Side::Bottom => offset,
    }
}

pub fn legend_rect(
    data_area: &DataArea,
    corner: LegendCorner,
    padding: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> RectF {
    let width = extents.width + padding * 2.0;
    let height = extents.height() + padding * 2.0;
    let x = match corner {
        LegendCorner::TopLeft | LegendCorner::BottomLeft => data_area.x as f32 + LEGEND_INSET,
        LegendCorner::TopRight | LegendCorner::BottomRight => {
            (data_area.x + data_area.width) as f32 - width - LEGEND_INSET
        }
    };
    let y = match corner {
        LegendCorner::TopLeft | LegendCorner::TopRight => data_area.y as f32 + LEGEND_INSET,
        LegendCorner::BottomLeft | LegendCorner::BottomRight => {
            (data_area.y + data_area.height) as f32 - height - LEGEND_INSET
        }
    };
    RectF {
        x: x + offset.0,
        y: y + offset.1,
        width,
        height,
    }
}

/// The colourbar strip's rectangle.
///
/// `side` alone decides the orientation: `Left` / `Right` give a vertical strip
/// `thickness_px` wide, `Top` / `Bottom` a horizontal one `thickness_px` tall.
/// The long dimension is `length_frac` of the data area's matching dimension,
/// placed by `align`, so the bar lines up with the plot it describes. The
/// strip's axis runs along its outer long edge — which is why its ends carry
/// `axis.min` / `axis.max` rather than the data area's corners.
///
/// **The short dimension is measured inward from the chart-area edge, not
/// outward from the data area.** The colourbar band is the *outermost* part of
/// its side's margin: from the chart edge inward it is the strip's own label
/// margin and tick, then the strip, then `gap_px`, and only then the chart
/// axis' band next to the data area. Placing the strip immediately outside the
/// data area instead would sit it on top of that axis' tick labels whenever
/// they are visible on the same side — and axis labels are drawn from the data
/// area outward, so they cannot move out of the way.
///
/// The two agree by construction: `gap_px` is what is left between the axis band
/// and the strip precisely because `colorbar_parts` charged the same terms to
/// that side's margin.
///
/// `offset_x` / `offset_y` shift the result and nothing else: they are
/// margin-noncontributing, exactly like the legend's and the titles' offsets, so
/// dragging the bar does not reflow the data area out from under the pointer.
///
/// Callers get a rect for a hidden bar too — the visibility decision belongs to
/// the drawing code and to `colorbar_contribution`, not here.
pub fn colorbar_rect(
    chart_area: &ChartArea,
    data_area: &DataArea,
    chart_title_margin: f32,
    bar: &ColorBarOptions,
) -> RectF {
    let frac = if bar.length_frac.is_finite() {
        bar.length_frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thickness = bar.thickness_px.max(0.0);
    // The strip's distance from the chart edge: its labels, then its ticks, then
    // the strip itself.
    let outer = bar.axis.out_margin.max(0.0) + bar.axis.major_tick_length.max(0.0) + thickness;
    let (dx, dy, dw, dh) = (
        data_area.x as f32,
        data_area.y as f32,
        data_area.width as f32,
        data_area.height as f32,
    );
    let (cx, cy, cw, ch) = (
        chart_area.x as f32,
        chart_area.y as f32,
        chart_area.width as f32,
        chart_area.height as f32,
    );

    // Written as one match over the four sides so a nested wildcard never stands
    // in for a side.
    let vertical_length = dh * frac;
    let horizontal_length = dw * frac;
    let anchored = match bar.side {
        Side::Left => RectF {
            x: cx + outer - thickness,
            y: dy + align_offset(dh, vertical_length, &bar.align),
            width: thickness,
            height: vertical_length,
        },
        Side::Right => RectF {
            x: cx + cw - outer,
            y: dy + align_offset(dh, vertical_length, &bar.align),
            width: thickness,
            height: vertical_length,
        },
        // The chart title owns the very top of the chart area; the colourbar
        // band is the outermost part of what is left, exactly as
        // `top_total` sums them.
        Side::Top => RectF {
            x: dx + align_offset(dw, horizontal_length, &bar.align),
            y: cy + chart_title_margin + outer - thickness,
            width: horizontal_length,
            height: thickness,
        },
        Side::Bottom => RectF {
            x: dx + align_offset(dw, horizontal_length, &bar.align),
            y: cy + ch - outer,
            width: horizontal_length,
            height: thickness,
        },
    };
    anchored.translated(bar.offset_x, bar.offset_y)
}

/// Where a strip of `length` starts within `available`. `Start` is top / left,
/// matching the screen direction the sides are named in.
fn align_offset(available: f32, length: f32, align: &BarAlign) -> f32 {
    match align {
        BarAlign::Start => 0.0,
        BarAlign::Center => (available - length) * 0.5,
        BarAlign::End => available - length,
    }
}

// ---------------------------------------------------------------------------
// auto_layout measurement (#44)
// ---------------------------------------------------------------------------

/// Clearance [`auto_layout`] keeps between an axis title and its tick
/// labels, and between either and the edges of their band: the label gap.
const AUTO_LAYOUT_CLEARANCE: f32 = LABEL_GAP;

/// What [`auto_layout`] reads from the parts of one emitted element: the
/// union of their extents, and of its text blocks the most digits one
/// prints and the widest one. Kept as numbers, so measuring stores no part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Probe {
    bounds: Option<RectF>,
    digits: usize,
    widest: f32,
}

impl Probe {
    /// Emit `element` of `cfg` into a fresh probe.
    fn of(cfg: &Config, element: &Element, measure: &dyn MeasureText) -> Self {
        let mut probe = Self::default();
        crate::select::emit_element(cfg, element, measure, &mut probe);
        probe
    }

    /// The probe of `self` and `other` together.
    fn with(self, other: Self) -> Self {
        Self {
            bounds: match (self.bounds, other.bounds) {
                (Some(a), Some(b)) => Some(crate::sink::union(a, b)),
                (a, b) => a.or(b),
            },
            digits: self.digits.max(other.digits),
            widest: self.widest.max(other.widest),
        }
    }

    fn add(&mut self, r: RectF) {
        if [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite()) {
            self.bounds = Some(self.bounds.map_or(r, |u| crate::sink::union(u, r)));
        }
    }
}

impl Sink for Probe {
    fn text(&mut self, _slot: Slot, text: &PlacedText<'_>) {
        self.add(text.rect());
        let digits = match text.text {
            TextRef::Plain { text, .. } => text.chars().filter(char::is_ascii_digit).count(),
            TextRef::Rich(rich) => rich
                .segments
                .iter()
                .filter(|s| s.text.is_ascii_digit())
                .count(),
        };
        self.digits = self.digits.max(digits);
        if text.extents.width.is_finite() {
            self.widest = self.widest.max(text.extents.width);
        }
    }

    fn stroke(&mut self, _slot: Slot, a: [f32; 2], b: [f32; 2], style: &StrokeStyle<'_>) {
        self.add(
            DrawnPart::Segment {
                a,
                b,
                width: style.width,
            }
            .extent(),
        );
    }

    fn rect(
        &mut self,
        _slot: Slot,
        rect: RectF,
        _fill: Option<Color>,
        _border: Option<Border<'_>>,
    ) {
        self.add(rect);
    }

    fn triangle(&mut self, _slot: Slot, points: [[f32; 2]; 3], _color: Color) {
        self.add(DrawnPart::Triangle(points).extent());
    }

    fn path(&mut self, slot: Slot, segments: &[crate::sink::PathSegment], style: &StrokeStyle<'_>) {
        Parts(&mut |part| self.add(part.extent())).path(slot, segments, style);
    }

    fn polygon(
        &mut self,
        _slot: Slot,
        points: &[[f32; 2]],
        _fill: Option<Color>,
        border: Option<&StrokeStyle<'_>>,
    ) {
        self.add(
            DrawnPart::Polygon {
                points: points.to_vec(),
                width: border.map_or(0.0, |b| b.width),
            }
            .extent(),
        );
    }

    fn ramp(&mut self, _slot: Slot, rect: RectF, _bar: &ColorBarOptions) {
        self.add(rect);
    }
}

/// Where `r` lies across the band on `side` whose inner edge is `inner`:
/// its near and far distance from that edge, outward positive. The band
/// runs from the data area (or the colourbar strip) toward the chart edge.
fn band_span(side: &Side, inner: f32, r: RectF) -> (f32, f32) {
    match side {
        Side::Left => (inner - (r.x + r.width), inner - r.x),
        Side::Right => (r.x - inner, r.x + r.width - inner),
        Side::Top => (inner - (r.y + r.height), inner - r.y),
        Side::Bottom => (r.y - inner, r.y + r.height - inner),
    }
}

/// The inner edge of a band on `side` of `rect`: `rect`'s edge on that side
/// moved `tick` outward.
fn band_inner_edge(side: &Side, rect: RectF, tick: f32) -> f32 {
    match side {
        Side::Left => rect.x - tick,
        Side::Right => rect.x + rect.width + tick,
        Side::Top => rect.y - tick,
        Side::Bottom => rect.y + rect.height + tick,
    }
}

/// The +1 or -1 that turns an axis title's `offset_y`, the offset across
/// its band in the title's rotated frame
/// ([`axis_title_offset_to_screen`]), into a distance outward from the
/// data area.
fn outward_sign(side: &Side) -> f32 {
    match side {
        Side::Bottom => 1.0,
        Side::Top | Side::Left | Side::Right => -1.0,
    }
}

/// One band as measured: how far its labels reach past the inner edge, and
/// the near and far distance of its title, both outward from that edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BandMeasure {
    reach: Option<f32>,
    title: Option<(f32, f32)>,
}

impl BandMeasure {
    fn of(side: &Side, inner: f32, labels: &Probe, title: &Probe) -> Self {
        Self {
            reach: labels.bounds.map(|r| band_span(side, inner, r).1.max(0.0)),
            title: title.bounds.map(|r| band_span(side, inner, r)),
        }
    }

    /// The band the labels and a title placed beside them need: the labels'
    /// reach, the clearance, the title's thickness and the clearance again
    /// to the band's outer edge; the reach and the clearance alone without a
    /// title. `None` for a band with neither.
    fn required(&self) -> Option<f32> {
        let reach = self.reach.unwrap_or(0.0);
        match self.title {
            Some((near, far)) => {
                Some(reach + AUTO_LAYOUT_CLEARANCE + (far - near) + AUTO_LAYOUT_CLEARANCE)
            }
            None => self.reach.map(|r| r + AUTO_LAYOUT_CLEARANCE),
        }
    }

    /// The outward title offset that places the title, in a band `band`
    /// wide where it is now offset outward by `offset`, beside its labels:
    /// its inner edge the clearance past their reach, rounded outward to a
    /// whole pixel. `None` when there is no title, or when the title at
    /// `offset` already clears the labels by the clearance and stays inside
    /// the band.
    fn placed_offset(&self, band: f32, offset: f32, pushed: Option<f32>) -> Option<f32> {
        const EPS: f32 = 1e-3;
        let (near, far) = self.title?;
        let thickness = far - near;
        let clear = self.reach.unwrap_or(0.0) + AUTO_LAYOUT_CLEARANCE;
        // The title's inner edge before the offset, as the placement puts
        // it: centred in the band, or for a colourbar title (`pushed`, the
        // gap it keeps past its labels) past the labels when the centre would
        // sit on them (`colorbar_title_placement`, #259); then moved by the
        // offset.
        let centred = (band - thickness) * 0.5;
        let base = match pushed {
            Some(gap) => centred.max((self.reach.unwrap_or(0.0) + gap).min(band - thickness - gap)),
            None => centred,
        };
        let at = base + offset;
        if at >= clear - EPS && at + thickness <= band + EPS {
            return None;
        }
        let placed = (clear - base).ceil();
        (placed.is_finite() && placed != offset).then_some(placed)
    }
}

// ---------------------------------------------------------------------------
// auto_layout (#44)
// ---------------------------------------------------------------------------

/// A `Decimal` or `Plain` tick label that prints more digits than this is
/// too long for [`auto_layout`]: `0.00300` has six.
const AUTO_LAYOUT_MAX_DIGITS: usize = 5;
/// A tick label wider than this many ems of its font size is too long for
/// [`auto_layout`], whatever it prints.
const AUTO_LAYOUT_MAX_LABEL_EMS: f32 = 5.0;
/// The passes [`auto_layout`] runs at most. A pass that changes nothing ends
/// it, so the result is a fixed point (and a second call a no-op) unless
/// every pass changed something; the idempotence tests cover every
/// catalogue scene and released config.
const AUTO_LAYOUT_PASSES: usize = 8;

/// `cfg` with axis bands, axis title offsets and tick-label formats chosen so
/// that its tick labels, axis titles, chart title, legend and colourbar do
/// not overlap, as `measure` lays them out.
///
/// An explicit call, the same shape as [`fit_chart_title_margin`]: the host
/// decides when to call it (typically after an auto-fit or a zoom, since the
/// range helpers reset a linear axis to `LabelFormat::Decimal`) and applies
/// the result as any other SSOT edit. The renderer never calls it. The input
/// is not modified, the same input gives the same output, and calling it on
/// its own result changes nothing. It reads only `cfg` and `measure`, and
/// changes display parameters only: never an axis range, a tick setting, a
/// series, an annotation or a pick.
///
/// Measured with `measure`'s own layout of each element
/// ([`crate::chrome`] for tick labels, which places the labels the
/// renderer draws after its pruning), on the panels the renderer draws:
///
/// - **Label format.** An axis whose labels are `Decimal` or `Plain` and not
///   categorical switches to `LabelFormat::Power` when a drawn label prints
///   more than five digits (`0.00300`) or is wider than five ems of its font
///   size. Other formats are left alone, and a format never switches back.
/// - **`out_margin`.** The band of each side grows to hold its labels and,
///   beside them, its title: the labels' reach past the tick, [`LABEL_GAP`],
///   the title's thickness and [`LABEL_GAP`] again to the band's outer edge
///   (there is no separate title gap), or the reach and the gap when the side
///   has no title. A band that already holds them is kept as it is, a side
///   without labels or a title keeps its margin, and a band is never shrunk.
///   A growth that would leave no data area is not made.
/// - **Title offsets.** A title that does not clear its labels by
///   [`LABEL_GAP`], or leaves its band, gets the offset across the band
///   (`title_option.offset_y`) that puts its inner edge [`LABEL_GAP`] past
///   the labels, rounded outward to a whole pixel. A title that already
///   clears them inside its band, centred or offset by the host, is left
///   where it is. The offset is an ordinary SSOT field: drag, hit testing and
///   selection follow it.
/// - **Overlays and the colourbar.** Each overlay axis' band grows the same
///   way (the right margin is the widest band of the group), and so does the
///   colourbar's `axis.out_margin`, measured from the strip.
/// - **Chart title and legend.** The title band follows
///   [`fit_chart_title_margin`]. A legend box over the colourbar moves to the
///   first corner, with zero offsets, where it clears the bar.
/// - **Subplots** are laid out as charts of their own.
///
/// Not covered: overlay axes drawn over each other or over the parent's
/// right axis (only `line_offset` separates them), polar angle labels
/// against radius labels (the angle band is fixed by the font size), and
/// labels that run past a chart edge along their axis.
pub fn auto_layout(cfg: &Config, measure: &dyn MeasureText) -> Config {
    let mut out = cfg.clone();
    lay_out_chart(&mut out, measure);
    for index in 0..out.subplots.len() {
        let Some(mut chart) = out.subplot_config(index) else {
            continue;
        };
        lay_out_chart(&mut chart, measure);
        adopt_chart_layout(&mut out.subplots[index].config, &chart);
    }
    out
}

/// Run [`auto_layout`]'s passes on one chart until one changes nothing.
fn lay_out_chart(cfg: &mut Config, measure: &dyn MeasureText) {
    for _ in 0..AUTO_LAYOUT_PASSES {
        if !auto_layout_pass(cfg, measure) {
            break;
        }
    }
}

const SIDES: [Side; 4] = [Side::Top, Side::Bottom, Side::Left, Side::Right];

/// One pass over a chart; returns whether it changed anything.
fn auto_layout_pass(cfg: &mut Config, measure: &dyn MeasureText) -> bool {
    let mut changed = switch_long_labels(cfg, measure);
    for side in &SIDES {
        let measured =
            crate::select::on_drawn_panel(cfg, |panel| axis_band_measure(panel, side, measure));
        if let Some((band, drawn)) = measured {
            changed |= fit_band(cfg, &band, drawn, side, BandOwner::Axis(side));
        }
    }
    for layer in 0..cfg.overlays.len() {
        let measured = cfg
            .overlay_panel(layer)
            .and_then(|panel| axis_band_measure(&panel, &Side::Right, measure));
        if let Some((band, drawn)) = measured {
            changed |= fit_band(cfg, &band, drawn, &Side::Right, BandOwner::Overlay(layer));
        }
    }
    let measured =
        crate::select::on_drawn_panel(cfg, |panel| colorbar_band_measure(panel, measure));
    if let Some((side, band, drawn)) = measured {
        changed |= fit_band(cfg, &band, drawn, &side, BandOwner::ColorBar);
    }
    changed |= fit_chart_title_margin(cfg, measure);
    changed |= clear_legend(cfg, measure);
    changed
}

/// Whether `axis`, whose drawn labels are `labels`, prints labels too long
/// for its format: see [`auto_layout`].
fn labels_too_long(axis: &AxisOptions, labels: &Probe) -> bool {
    axis.categories.is_none()
        && matches!(
            axis.label_style.format,
            LabelFormat::Decimal | LabelFormat::Plain(_)
        )
        && (labels.digits > AUTO_LAYOUT_MAX_DIGITS
            || labels.widest > AUTO_LAYOUT_MAX_LABEL_EMS * axis.label_style.font_size)
}

/// Switch every axis of `cfg` whose labels are too long to
/// `LabelFormat::Power`; returns whether one switched.
fn switch_long_labels(cfg: &mut Config, measure: &dyn MeasureText) -> bool {
    let (sides, bar) = crate::select::on_drawn_panel(cfg, |panel| {
        let sides = SIDES.map(|side| {
            let labels = Probe::of(panel, &Element::AxisLabel(side.clone()), measure);
            labels_too_long(axis_ref(panel, &side), &labels)
        });
        let bar = panel.colorbar.as_ref().is_some_and(|bar| {
            labels_too_long(
                &bar.axis,
                &Probe::of(panel, &Element::ColorBarLabel, measure),
            )
        });
        (sides, bar)
    });
    let mut changed = false;
    for (side, long) in SIDES.iter().zip(sides) {
        if long {
            axis_mut(cfg, side).label_style.format = LabelFormat::Power;
            changed = true;
        }
    }
    if bar && let Some(bar) = cfg.colorbar.as_mut() {
        bar.axis.label_style.format = LabelFormat::Power;
        changed = true;
    }
    for layer in 0..cfg.overlays.len() {
        let long = cfg.overlay_panel(layer).is_some_and(|panel| {
            let labels = Probe::of(&panel, &Element::AxisLabel(Side::Right), measure);
            labels_too_long(&panel.right_y, &labels)
        });
        if long {
            cfg.overlays[layer].y_axis.label_style.format = LabelFormat::Power;
            changed = true;
        }
    }
    changed
}

/// The band of the axis on `side` of the drawn panel `panel`, and the
/// `out_margin` it was drawn with. On a polar chart the angle and radius
/// labels are counted on every side they reach past.
fn axis_band_measure(
    panel: &Config,
    side: &Side,
    measure: &dyn MeasureText,
) -> Option<(BandMeasure, f32)> {
    let da = panel.data_area().ok()?;
    let axis = axis_ref(panel, side);
    let inner = band_inner_edge(side, RectF::from_rect(&da), tick_contribution(axis));
    let labels = if panel.polar().is_some() {
        Probe::of(panel, &Element::AxisLabel(Side::Bottom), measure).with(Probe::of(
            panel,
            &Element::AxisLabel(Side::Left),
            measure,
        ))
    } else {
        Probe::of(panel, &Element::AxisLabel(side.clone()), measure)
    };
    let title = Probe::of(panel, &Element::AxisTitle(side.clone()), measure);
    Some((
        BandMeasure::of(side, inner, &labels, &title),
        axis.out_margin,
    ))
}

/// The colourbar's band of the drawn panel `panel`, measured from the strip
/// and its tick: its side, the band and the `out_margin` it was drawn with.
fn colorbar_band_measure(
    panel: &Config,
    measure: &dyn MeasureText,
) -> Option<(Side, BandMeasure, f32)> {
    let (bar, strip) = visible_colorbar_rect(panel)?;
    let inner = band_inner_edge(&bar.side, strip, bar.axis.major_tick_length.max(0.0));
    let labels = Probe::of(panel, &Element::ColorBarLabel, measure);
    let title = Probe::of(panel, &Element::ColorBarTitle, measure);
    Some((
        bar.side.clone(),
        BandMeasure::of(&bar.side, inner, &labels, &title),
        bar.axis.out_margin,
    ))
}

/// The axis whose `out_margin` holds a band.
#[derive(Clone, Copy)]
enum BandOwner<'a> {
    /// One of the chart's four axes.
    Axis(&'a Side),
    /// Overlay layer `i`'s Y axis.
    Overlay(usize),
    /// The colourbar's axis.
    ColorBar,
}

impl BandOwner<'_> {
    fn axis<'c>(self, cfg: &'c mut Config) -> Option<&'c mut AxisOptions> {
        match self {
            BandOwner::Axis(side) => Some(axis_mut(cfg, side)),
            BandOwner::Overlay(layer) => cfg.overlays.get_mut(layer).map(|o| &mut o.y_axis),
            BandOwner::ColorBar => cfg.colorbar.as_mut().map(|bar| &mut bar.axis),
        }
    }
}

/// Fit one band of `cfg`: grow the `out_margin` of `owner`'s axis so `band`
/// fits in it, unless the band it was drawn with (`drawn`; for an overlay
/// or the parent's right axis, the group's) already holds it or the growth
/// leaves no data area; then place a title that does not clear its labels
/// inside the band beside them. Returns whether anything changed.
fn fit_band(
    cfg: &mut Config,
    band: &BandMeasure,
    drawn: f32,
    side: &Side,
    owner: BandOwner<'_>,
) -> bool {
    let Some(before) = owner.axis(cfg).map(|axis| axis.out_margin) else {
        return false;
    };
    let mut changed = false;
    let mut width = drawn;
    if let Some(required) = band.required()
        && required.is_finite()
        && drawn < required
    {
        let grown = required.ceil();
        let had_data_area = cfg.data_area().is_ok();
        if let Some(axis) = owner.axis(cfg) {
            axis.out_margin = before.max(grown);
        }
        if had_data_area && cfg.data_area().is_err() {
            if let Some(axis) = owner.axis(cfg) {
                axis.out_margin = before;
            }
        } else {
            changed = before < grown;
            width = drawn.max(grown);
        }
    }
    if let Some(axis) = owner.axis(cfg) {
        let sign = outward_sign(side);
        // A colourbar title is pushed past its labels by a third of their
        // font (`colorbar_title_placement`).
        let pushed =
            matches!(owner, BandOwner::ColorBar).then(|| axis.label_style.font_size.max(0.0) / 3.0);
        if let Some(outward) = band.placed_offset(width, sign * axis.title_option.offset_y, pushed)
        {
            axis.title_option.offset_y = sign * outward;
            changed = true;
        }
    }
    changed
}

/// Move a legend box that overlaps the colourbar to the first corner, with
/// zero offsets, where it clears every colourbar part: its own corner, then
/// the others. Returns whether it moved.
fn clear_legend(cfg: &mut Config, measure: &dyn MeasureText) -> bool {
    let lg = &cfg.legend;
    if !lg.visible || lg.content.segments.is_empty() || cfg.colorbar.is_none() {
        return false;
    }
    let corner = crate::select::on_drawn_panel(cfg, |panel| {
        let da = panel.data_area().ok()?;
        let lg = &panel.legend;
        let m = measure.measure_rich(&lg.content);
        let clashes = |corner: LegendCorner, offset: (f32, f32)| {
            let rect = legend_rect(&da, corner, lg.padding, offset, m);
            let mut hit = false;
            for element in [
                Element::ColorBar,
                Element::ColorBarAxis,
                Element::ColorBarLabel,
                Element::ColorBarTitle,
            ] {
                crate::select::emit_element(
                    panel,
                    &element,
                    measure,
                    &mut Parts(&mut |part| hit |= rects_overlap(rect, part.extent())),
                );
            }
            hit
        };
        if !clashes(lg.corner, (lg.offset_x, lg.offset_y)) {
            return None;
        }
        [
            lg.corner,
            LegendCorner::TopRight,
            LegendCorner::TopLeft,
            LegendCorner::BottomRight,
            LegendCorner::BottomLeft,
        ]
        .into_iter()
        .find(|corner| !clashes(*corner, (0.0, 0.0)))
    });
    let Some(corner) = corner else {
        return false;
    };
    cfg.legend.corner = corner;
    cfg.legend.offset_x = 0.0;
    cfg.legend.offset_y = 0.0;
    true
}

/// Whether two rectangles share an area.
fn rects_overlap(a: RectF, b: RectF) -> bool {
    let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    w > 0.0 && h > 0.0
}

/// Store what [`auto_layout`] chose for the chart of a subplot, laid out
/// through `subplot_config`, into its stored config: every field the passes
/// write and nothing else, so its linked ranges stay their targets'.
fn adopt_chart_layout(stored: &mut Config, chart: &Config) {
    adopt_axis_layout(&mut stored.top_x, &chart.top_x);
    adopt_axis_layout(&mut stored.bottom_x, &chart.bottom_x);
    adopt_axis_layout(&mut stored.left_y, &chart.left_y);
    adopt_axis_layout(&mut stored.right_y, &chart.right_y);
    for (to, from) in stored.overlays.iter_mut().zip(&chart.overlays) {
        adopt_axis_layout(&mut to.y_axis, &from.y_axis);
    }
    if let (Some(to), Some(from)) = (stored.colorbar.as_mut(), chart.colorbar.as_ref()) {
        adopt_axis_layout(&mut to.axis, &from.axis);
    }
    stored.chart_title.top_margin = chart.chart_title.top_margin;
    stored.legend.corner = chart.legend.corner;
    stored.legend.offset_x = chart.legend.offset_x;
    stored.legend.offset_y = chart.legend.offset_y;
}

/// The fields of one axis [`auto_layout`] writes: its band, its title's
/// offset across the band and a switch to `LabelFormat::Power`.
fn adopt_axis_layout(to: &mut AxisOptions, from: &AxisOptions) {
    to.out_margin = from.out_margin;
    to.title_option.offset_y = from.title_option.offset_y;
    if from.label_style.format == LabelFormat::Power {
        to.label_style.format = LabelFormat::Power;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::Rect;

    /// `cross_at` is a value of the perpendicular axis: the bottom axis line
    /// crosses `left_y` at it, the left one `bottom_x`, through the pixel
    /// centre under it and never past the plot's edge lines.
    #[test]
    fn axis_lines_cross_the_perpendicular_axis_at_cross_at() {
        let mut config = default_config();
        config.left_y.min = -10.0;
        config.left_y.max = 10.0;
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 4.0;
        let plot = RectF {
            x: 100.0,
            y: 50.0,
            width: 400.0,
            height: 200.0,
        };
        assert_eq!(axis_cross(&config, &Side::Bottom), None);
        assert_eq!(
            axis_anchor(Side::Bottom, &plot, None),
            ((100.5, 249.5), (499.5, 249.5))
        );
        config.bottom_x.cross_at = Some(0.0);
        let t = axis_cross(&config, &Side::Bottom);
        assert_eq!(t, Some(0.5));
        assert_eq!(
            axis_anchor(Side::Bottom, &plot, t),
            ((100.5, 150.5), (499.5, 150.5))
        );
        // A value past the axis range stays on the edge line.
        config.bottom_x.cross_at = Some(-50.0);
        let t = axis_cross(&config, &Side::Bottom);
        assert_eq!(t, Some(0.0));
        assert_eq!(axis_anchor(Side::Bottom, &plot, t).0.1, 249.5);
        // The left axis crosses bottom_x; an inverted perpendicular axis
        // mirrors the fraction.
        config.left_y.cross_at = Some(1.0);
        assert_eq!(axis_cross(&config, &Side::Left), Some(0.25));
        assert_eq!(axis_anchor(Side::Left, &plot, Some(0.25)).0, (200.5, 50.5));
        config.bottom_x.inverted = true;
        assert_eq!(axis_cross(&config, &Side::Left), Some(0.75));
        // A categorical perpendicular axis maps its category range.
        config.bottom_x.inverted = false;
        config.bottom_x.categories = Some(vec![
            crate::config::Category::new("a", "A"),
            crate::config::Category::new("b", "B"),
        ]);
        config.left_y.cross_at = Some(0.5);
        assert_eq!(axis_cross(&config, &Side::Left), Some(0.5));
        // Polar charts draw no Cartesian axis lines.
        config.set_polar(Some(crate::polar::PolarOptions::default()));
        assert_eq!(axis_cross(&config, &Side::Left), None);
    }

    fn areas() -> (ChartArea, DataArea) {
        (
            ChartArea(Rect {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
            }),
            DataArea(Rect {
                x: 50,
                y: 60,
                width: 200,
                height: 100,
            }),
        )
    }

    fn band(out_margin: f32, chart_title_margin: f32, edge_inset: f32) -> TitleBand {
        TitleBand {
            out_margin,
            chart_title_margin,
            edge_inset,
        }
    }

    fn extents() -> TextExtents {
        TextExtents {
            width: 20.0,
            ascent: 7.0,
            descent: 3.0,
        }
    }

    #[test]
    fn axis_anchors_preserve_half_pixels() {
        let (_, data) = areas();
        let data = RectF::from_rect(&data);
        assert_eq!(
            axis_anchor(Side::Top, &data, None),
            ((50.5, 60.5), (249.5, 60.5))
        );
        assert_eq!(
            axis_anchor(Side::Right, &data, None),
            ((249.5, 60.5), (249.5, 159.5))
        );
    }

    /// A plot rectangle that is not integer-aligned takes the same
    /// half-pixel insets from its own edges.
    #[test]
    fn axis_anchors_of_a_fractional_plot_inset_its_own_edges() {
        let plot = RectF {
            x: 50.25,
            y: 60.75,
            width: 199.5,
            height: 100.25,
        };
        assert_eq!(
            axis_anchor(Side::Top, &plot, None),
            ((50.75, 61.25), (249.25, 61.25))
        );
        assert_eq!(
            axis_anchor(Side::Bottom, &plot, None),
            ((50.75, 160.5), (249.25, 160.5))
        );
        assert_eq!(
            axis_anchor(Side::Left, &plot, None),
            ((50.75, 61.25), (50.75, 160.5))
        );
        assert_eq!(
            axis_anchor(Side::Right, &plot, None),
            ((249.25, 61.25), (249.25, 160.5))
        );
    }

    /// For every integer data area the plot rectangle's anchors are the
    /// integer ones bit for bit.
    #[test]
    fn axis_anchors_of_an_integer_data_area_are_exact() {
        for (x, y, width, height) in [
            (0, 0, 1, 1),
            (50, 60, 200, 100),
            (80, 40, 1940, 1300),
            (3, 7, 16_000_000, 9_000_000),
        ] {
            let x0 = x as f32 + 0.5;
            let y0 = y as f32 + 0.5;
            let x1 = (x + width) as f32 - 0.5;
            let y1 = (y + height) as f32 - 0.5;
            let plot = RectF::from_rect(&Rect {
                x,
                y,
                width,
                height,
            });
            assert_eq!(axis_anchor(Side::Top, &plot, None), ((x0, y0), (x1, y0)));
            assert_eq!(axis_anchor(Side::Bottom, &plot, None), ((x0, y1), (x1, y1)));
            assert_eq!(axis_anchor(Side::Left, &plot, None), ((x0, y0), (x0, y1)));
            assert_eq!(axis_anchor(Side::Right, &plot, None), ((x1, y0), (x1, y1)));
        }
    }

    #[test]
    fn label_origin_and_rect_cover_all_sides_and_offsets() {
        let tick = (100.0, 200.0);
        let offset = (2.0, 4.0);
        let m = extents();
        let expected = [
            (Side::Top, (92.0, 192.0)),
            (Side::Bottom, (92.0, 220.0)),
            (Side::Left, (73.0, 206.0)),
            (Side::Right, (111.0, 206.0)),
        ];
        for (side, origin) in expected {
            let actual = label_origin(side, tick, 5.0, offset, m);
            assert_eq!(actual, origin);
            assert_eq!(label_rect(actual, m).height, 10.0);
        }
    }

    #[test]
    fn title_placement_keeps_rotation_signs_and_offset_frames() {
        let (chart, data) = areas();
        let m = extents();
        let left = axis_title_placement(
            Side::Left,
            &chart,
            &data,
            band(40.0, 32.0, 0.0),
            (2.0, 3.0),
            m,
        );
        let right = axis_title_placement(
            Side::Right,
            &chart,
            &data,
            band(40.0, 32.0, 0.0),
            (2.0, 3.0),
            m,
        );
        assert_eq!(left.rotation_degrees, -90.0);
        assert_eq!(right.rotation_degrees, 90.0);
        assert_eq!(
            left.rect(m),
            RectF {
                x: 28.0,
                y: 98.0,
                width: 10.0,
                height: 20.0
            }
        );
        assert_eq!(
            right.rect(m),
            RectF {
                x: 282.0,
                y: 102.0,
                width: 10.0,
                height: 20.0
            }
        );

        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let local = (7.0, -4.0);
            assert_eq!(
                screen_offset_to_axis_title(side.clone(), axis_title_offset_to_screen(side, local)),
                local
            );
        }
    }

    /// 8 px per char on the widest line, 10 up / 3 down per line, 13 px line
    /// pitch — the shape of the renderer's multi-line envelope.
    struct LineMeasure;
    impl MeasureText for LineMeasure {
        fn measure_rich(&self, rt: &crate::text::RichText) -> TextExtents {
            let text: String = rt.segments.iter().map(|s| s.text).collect();
            let lines = text.split('\n').count();
            let widest = text.split('\n').map(|l| l.chars().count()).max();
            TextExtents {
                width: widest.unwrap_or(0) as f32 * 8.0,
                ascent: 10.0,
                descent: 3.0 + (lines - 1) as f32 * 13.0,
            }
        }
    }

    fn titled(text: &str) -> Config {
        let mut cfg = default_config();
        cfg.chart_title.text.segments = crate::text::rich_segments_from_text(text);
        cfg
    }

    #[test]
    fn the_title_band_padding_comes_from_the_defaults() {
        let defaults = crate::default::default_chart_title_options();
        assert_eq!(
            chart_title_band_padding(),
            defaults.top_margin - defaults.text.font_size
        );
    }

    // A three-line title (39 px) is taller than the default 32 px band, so
    // centring it would start above the chart area. Fitting grows the band to
    // the block plus the default padding, which the placement then centres.
    #[test]
    fn fitting_grows_the_band_to_hold_a_multi_line_title() {
        let mut cfg = titled("one\ntwo\nthree");
        let m = LineMeasure.measure_rich(&cfg.chart_title.text);
        assert!(m.height() > cfg.chart_title.top_margin);

        assert!(fit_chart_title_margin(&mut cfg, &LineMeasure));
        let band = cfg.chart_title.top_margin;
        assert_eq!(band, (m.height() + chart_title_band_padding()).ceil());

        let ca = &cfg.chart_area;
        let rect = chart_title_placement(ca, band, (0.0, 0.0), m).rect(m);
        assert!(rect.y >= ca.y as f32, "first line inside the chart area");
        assert!(
            rect.y + rect.height <= ca.y as f32 + band,
            "block inside the band"
        );
        assert!(cfg.data_area().is_ok());

        // Already fitted: a second call is a no-op.
        assert!(!fit_chart_title_margin(&mut cfg, &LineMeasure));
        assert_eq!(cfg.chart_title.top_margin, band);
    }

    #[test]
    fn fitting_leaves_a_single_default_line_and_a_larger_band_alone() {
        let mut cfg = titled("Title");
        assert!(!fit_chart_title_margin(&mut cfg, &LineMeasure));
        assert_eq!(
            cfg,
            titled("Title"),
            "the default band already fits one line"
        );

        // A block that fits the band, even with less than the default padding
        // around it, renders today without clipping and is left alone.
        let mut snug = titled("Title");
        snug.chart_title.top_margin = 14.0;
        assert!(LineMeasure.measure_rich(&snug.chart_title.text).height() <= 14.0);
        assert!(!fit_chart_title_margin(&mut snug, &LineMeasure));
        assert_eq!(snug.chart_title.top_margin, 14.0);

        // Grow-only: a band the user enlarged is never shrunk to the text.
        let mut roomy = titled("one\ntwo\nthree");
        roomy.chart_title.top_margin = 120.0;
        assert!(!fit_chart_title_margin(&mut roomy, &LineMeasure));
        assert_eq!(roomy.chart_title.top_margin, 120.0);
    }

    #[test]
    fn fitting_skips_hidden_empty_and_unusable_titles() {
        let mut empty = default_config();
        assert!(!fit_chart_title_margin(&mut empty, &LineMeasure));
        assert_eq!(empty, default_config());

        let mut hidden = titled("one\ntwo\nthree");
        hidden.chart_title.visible = false;
        assert!(!fit_chart_title_margin(&mut hidden, &LineMeasure));
        assert_eq!(hidden.chart_title.top_margin, 32.0);

        // A non-finite margin is left for validation to report.
        let mut nan = titled("one\ntwo\nthree");
        nan.chart_title.top_margin = f32::NAN;
        assert!(!fit_chart_title_margin(&mut nan, &LineMeasure));
        assert!(nan.chart_title.top_margin.is_nan());

        // Growing would leave no data area in a chart that had one: the band
        // stays as it was.
        let mut short = titled(&["line"; 40].join("\n"));
        short.chart_area.0.height = 300;
        assert!(short.data_area().is_ok());
        assert!(!fit_chart_title_margin(&mut short, &LineMeasure));
        assert_eq!(short.chart_title.top_margin, 32.0);
        assert!(short.data_area().is_ok());
    }

    #[test]
    fn colorbar_title_tracks_the_actual_strip_on_every_side() {
        let strip = RectF {
            x: 70.0,
            y: 80.0,
            width: 40.0,
            height: 100.0,
        };
        let moved = strip.translated(13.0, -9.0);
        let axis = crate::default::default_axis_options_colorbar();
        let m = extents();

        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let before =
                colorbar_title_placement(side.clone(), &strip, &axis, (0.0, 0.0), m, 0.0).rect(m);
            let after = colorbar_title_placement(side, &moved, &axis, (0.0, 0.0), m, 0.0).rect(m);
            assert_eq!(after, before.translated(13.0, -9.0));
        }

        let right =
            colorbar_title_placement(Side::Right, &strip, &axis, (0.0, 0.0), m, 0.0).rect(m);
        assert_eq!(right.y + right.height * 0.5, strip.y + strip.height * 0.5);

        let top = colorbar_title_placement(Side::Top, &strip, &axis, (0.0, 0.0), m, 0.0).rect(m);
        assert_eq!(top.x + top.width * 0.5, strip.x + strip.width * 0.5);
    }

    #[test]
    fn colorbar_title_clears_the_tick_labels_on_every_side() {
        let strip = RectF {
            x: 300.0,
            y: 200.0,
            width: 40.0,
            height: 100.0,
        };
        let mut axis = crate::default::default_axis_options_colorbar();
        // A band wide enough for the labels and the title, whose centre
        // would sit on the labels (#259).
        axis.out_margin = 60.0;
        let m = extents();
        let reach = 30.0;
        let clear = reach + axis.label_style.font_size / 3.0 - 1e-3;
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let r =
                colorbar_title_placement(side.clone(), &strip, &axis, (0.0, 0.0), m, reach).rect(m);
            match side {
                Side::Right => assert!(r.x >= strip.x + strip.width + clear, "{r:?}"),
                Side::Left => assert!(r.x + r.width <= strip.x - clear, "{r:?}"),
                Side::Bottom => assert!(r.y >= strip.y + strip.height + clear, "{r:?}"),
                Side::Top => assert!(r.y + r.height <= strip.y - clear, "{r:?}"),
            }
        }
        // A band too narrow to clear them: the title goes no further than
        // the band's outer edge, where the chart area may end.
        axis.out_margin = 36.0;
        let tick = axis.major_tick_length.max(0.0);
        let band_end = tick + axis.out_margin + 1e-3;
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let r =
                colorbar_title_placement(side.clone(), &strip, &axis, (0.0, 0.0), m, reach).rect(m);
            match side {
                Side::Right => assert!(r.x + r.width <= strip.x + strip.width + band_end, "{r:?}"),
                Side::Left => assert!(r.x >= strip.x - band_end, "{r:?}"),
                Side::Bottom => {
                    assert!(r.y + r.height <= strip.y + strip.height + band_end, "{r:?}")
                }
                Side::Top => assert!(r.y >= strip.y - band_end, "{r:?}"),
            }
        }
        // A band wide enough keeps the title at its centre.
        axis.out_margin = 200.0;
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let with = colorbar_title_placement(side.clone(), &strip, &axis, (0.0, 0.0), m, reach);
            let without = colorbar_title_placement(side, &strip, &axis, (0.0, 0.0), m, 0.0);
            assert_eq!(with.rect(m), without.rect(m));
        }
    }

    #[test]
    fn legend_rect_covers_all_corners_and_offsets() {
        let (_, data) = areas();
        let m = extents();
        let expected = [
            (LegendCorner::TopLeft, (57.0, 68.0)),
            (LegendCorner::TopRight, (215.0, 68.0)),
            (LegendCorner::BottomLeft, (57.0, 136.0)),
            (LegendCorner::BottomRight, (215.0, 136.0)),
        ];
        for (corner, (x, y)) in expected {
            let rect = legend_rect(&data, corner, 5.0, (1.0, 2.0), m);
            assert_eq!(
                rect,
                RectF {
                    x,
                    y,
                    width: 30.0,
                    height: 20.0
                }
            );
        }
    }

    /// A config whose data area really is the chart area minus its margins, so
    /// the band relationships the strip is placed by actually hold. The
    /// synthetic `areas()` fixture above cannot: its data area is not derived
    /// from any margin.
    fn colorbar_config(side: Side) -> (crate::config::Config, DataArea) {
        let mut cfg = crate::default::default_config();
        let mut bar = crate::default::default_colorbar_options();
        bar.side = side;
        bar.thickness_px = 20.0;
        bar.gap_px = 10.0;
        bar.length_frac = 0.5;
        bar.align = BarAlign::Center;
        cfg.colorbar = Some(bar);
        let da = cfg.data_area().expect("data area");
        (cfg, da)
    }

    fn rect_of(cfg: &crate::config::Config, da: &DataArea) -> RectF {
        colorbar_rect(
            &cfg.chart_area,
            da,
            cfg.chart_title.top_margin,
            cfg.colorbar.as_ref().expect("colourbar"),
        )
    }

    /// The band the strip sits in, taken apart. Every side must show the same
    /// three-part structure outward from the data area: the chart axis' band,
    /// then `gap_px`, then the strip — with the strip's own tick and label
    /// margin between it and the chart-area edge.
    #[test]
    fn the_colorbar_strip_sits_outside_the_axis_band_not_on_top_of_it() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let (cfg, da) = colorbar_config(side.clone());
            let bar = cfg.colorbar.as_ref().expect("colourbar");
            let rect = rect_of(&cfg, &da);
            let axis = crate::layout::axis_ref(&cfg, &side);
            let axis_band = axis.out_margin + axis.major_tick_length;
            let outer_band = bar.axis.out_margin + bar.axis.major_tick_length;
            let ca = &cfg.chart_area;

            let (from_data_area, to_chart_edge) = match side {
                Side::Left => (da.x as f32 - (rect.x + rect.width), rect.x - ca.x as f32),
                Side::Right => (
                    rect.x - (da.x + da.width) as f32,
                    (ca.x + ca.width) as f32 - (rect.x + rect.width),
                ),
                Side::Top => (
                    da.y as f32 - (rect.y + rect.height),
                    rect.y - (ca.y as f32 + cfg.chart_title.top_margin),
                ),
                Side::Bottom => (
                    rect.y - (da.y + da.height) as f32,
                    (ca.y + ca.height) as f32 - (rect.y + rect.height),
                ),
            };
            // Between the data area and the strip: the axis' own band, then the
            // gap. Nothing of the strip reaches into the axis' label space.
            assert!(
                (from_data_area - (axis_band + bar.gap_px)).abs() <= 1.0,
                "{side:?}: {from_data_area} px from the data area, expected axis band \
                 {axis_band} + gap {}",
                bar.gap_px
            );
            // Between the strip and the chart edge: the strip's tick and labels.
            assert!(
                (to_chart_edge - outer_band).abs() <= 1.0,
                "{side:?}: {to_chart_edge} px to the chart edge, expected {outer_band}"
            );
        }
    }

    // Orientation, length, and alignment along the side.
    #[test]
    fn colorbar_orientation_and_length_follow_side_and_length_frac() {
        let (cfg, da) = colorbar_config(Side::Right);
        let vertical = rect_of(&cfg, &da);
        assert_eq!(vertical.width, 20.0);
        assert_eq!(vertical.height, da.height as f32 * 0.5);
        // Centred along the data area.
        assert_eq!(vertical.y, da.y as f32 + da.height as f32 * 0.25);

        let (cfg, da) = colorbar_config(Side::Bottom);
        let horizontal = rect_of(&cfg, &da);
        assert_eq!(horizontal.height, 20.0);
        assert_eq!(horizontal.width, da.width as f32 * 0.5);
        assert_eq!(horizontal.x, da.x as f32 + da.width as f32 * 0.25);
    }

    // `align` places the strip along its side; `Start` is top / left in screen
    // direction, matching how the sides are named.
    #[test]
    fn colorbar_align_places_the_strip_along_its_side() {
        let (mut cfg, da) = colorbar_config(Side::Right);
        let quarter = da.height as f32 * 0.5;

        cfg.colorbar.as_mut().unwrap().align = BarAlign::Start;
        assert_eq!(rect_of(&cfg, &da).y, da.y as f32);
        cfg.colorbar.as_mut().unwrap().align = BarAlign::End;
        assert_eq!(rect_of(&cfg, &da).y, (da.y + da.height) as f32 - quarter);

        let (mut cfg, da) = colorbar_config(Side::Bottom);
        let half = da.width as f32 * 0.5;
        cfg.colorbar.as_mut().unwrap().align = BarAlign::Start;
        assert_eq!(rect_of(&cfg, &da).x, da.x as f32);
        cfg.colorbar.as_mut().unwrap().align = BarAlign::End;
        assert_eq!(rect_of(&cfg, &da).x, (da.x + da.width) as f32 - half);
    }

    // A full-length bar spans its side exactly, whatever the alignment.
    #[test]
    fn a_full_length_colorbar_spans_its_side() {
        for align in [BarAlign::Start, BarAlign::Center, BarAlign::End] {
            let (mut cfg, da) = colorbar_config(Side::Right);
            {
                let bar = cfg.colorbar.as_mut().unwrap();
                bar.length_frac = 1.0;
                bar.align = align.clone();
            }
            let rect = rect_of(&cfg, &da);
            assert_eq!(rect.y, da.y as f32, "{align:?}");
            assert_eq!(rect.height, da.height as f32, "{align:?}");
        }
    }

    // Nonsense dims produce a degenerate rect, not a panic and not a rect that
    // reaches back over the data area. `Config::validate` is what rejects them.
    #[test]
    fn colorbar_rect_survives_values_validate_would_reject() {
        let (mut cfg, da) = colorbar_config(Side::Right);
        for frac in [0.0, -1.0, f32::NAN, 2.0] {
            cfg.colorbar.as_mut().unwrap().length_frac = frac;
            let rect = rect_of(&cfg, &da);
            assert!(
                rect.height >= 0.0,
                "frac {frac} gave height {}",
                rect.height
            );
            assert!(
                rect.height <= da.height as f32,
                "frac {frac} overran its side"
            );
        }
        cfg.colorbar.as_mut().unwrap().length_frac = 0.5;
        cfg.colorbar.as_mut().unwrap().thickness_px = -5.0;
        let rect = rect_of(&cfg, &da);
        assert_eq!(rect.width, 0.0);
    }

    // The axis title band is pushed in by whatever occupies the outside of its
    // side's margin. Without the inset a title on the colourbar's side would be
    // centred on top of the bar.
    #[test]
    fn axis_title_placement_is_pushed_in_by_the_colorbar_band() {
        let (cfg, da) = colorbar_config(Side::Right);
        let colorbar_band = cfg.colorbar_band(&Side::Right);
        assert!(colorbar_band > 0.0);

        let placement = |inset: f32| {
            axis_title_placement(
                Side::Right,
                &cfg.chart_area,
                &da,
                band(cfg.right_y.out_margin, cfg.chart_title.top_margin, inset),
                (0.0, 0.0),
                extents(),
            )
        };
        let with_band = placement(colorbar_band);
        let without = placement(0.0);
        assert!(
            (without.rotation_center.0 - with_band.rotation_center.0 - colorbar_band).abs() < 1e-3,
            "the inset must move the title inward by exactly the band"
        );

        // And the sides the bar is not on are unaffected, because their band is 0.
        for side in [Side::Top, Side::Bottom, Side::Left] {
            assert_eq!(cfg.colorbar_band(&side), 0.0);
        }
    }

    // auto_layout measurement (#44).

    /// Every glyph 8 px wide, 10 px up and 3 px down: the axis titles and
    /// the chart title.
    const GLYPH: f32 = 8.0;

    /// A measure whose every glyph is `GLYPH` wide, 10 px up and 3 px down:
    /// the chrome (`crate::chrome`) places and measures the tick labels the
    /// renderer would draw with it.
    struct Labels;

    /// `axis` with the labels `0`, `0.00300` and `0.00600`: six digits, the
    /// widest 7 glyphs (56 px).
    fn long_labels(axis: &mut AxisOptions) {
        axis.min = 0.0;
        axis.max = 0.006;
        axis.major_spacing = 0.003;
    }

    /// `axis` with the labels `-1.2345`, `0` and `1.2345`: five digits, long
    /// but not too long for `Decimal`, the widest 7 glyphs (56 px).
    fn five_digit_labels(axis: &mut AxisOptions) {
        axis.min = -1.2345;
        axis.max = 1.2345;
        axis.major_spacing = 1.2345;
        axis.label_style.significant_digits = 5;
    }

    impl MeasureText for Labels {
        fn measure_rich(&self, rt: &crate::text::RichText) -> TextExtents {
            TextExtents {
                width: rt.segments.len() as f32 * GLYPH,
                ascent: 10.0,
                descent: 3.0,
            }
        }
    }

    // A band is measured outward from its inner edge on every side: the
    // near and the far distance of a rect 10..20 px out.
    #[test]
    fn band_span_measures_outward_from_the_inner_edge() {
        let r = |x, y| RectF {
            x,
            y,
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(band_span(&Side::Left, 100.0, r(80.0, 0.0)), (10.0, 20.0));
        assert_eq!(band_span(&Side::Right, 100.0, r(110.0, 0.0)), (10.0, 20.0));
        assert_eq!(band_span(&Side::Top, 100.0, r(0.0, 80.0)), (10.0, 20.0));
        assert_eq!(band_span(&Side::Bottom, 100.0, r(0.0, 110.0)), (10.0, 20.0));
        let plot = r(50.0, 50.0);
        assert_eq!(band_inner_edge(&Side::Left, plot, 5.0), 45.0);
        assert_eq!(band_inner_edge(&Side::Right, plot, 5.0), 65.0);
        assert_eq!(band_inner_edge(&Side::Top, plot, 5.0), 45.0);
        assert_eq!(band_inner_edge(&Side::Bottom, plot, 5.0), 65.0);
        // `offset_y` moves a title outward on the bottom, inward elsewhere.
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let (dx, dy) = axis_title_offset_to_screen(side.clone(), (0.0, 1.0));
            let outward = match side {
                Side::Left => -dx,
                Side::Right => dx,
                Side::Top => -dy,
                Side::Bottom => dy,
            };
            assert_eq!(outward, outward_sign(&side), "{side:?}");
        }
    }

    // A probe keeps the union of the parts, the most digits a label prints
    // and the widest label: the three `0.00300` labels of the left axis.
    #[test]
    fn a_probe_reads_the_drawn_labels_without_storing_them() {
        let mut cfg = default_config();
        long_labels(&mut cfg.left_y);
        let measure = Labels;
        let probe = Probe::of(&cfg, &Element::AxisLabel(Side::Left), &measure);
        assert_eq!(probe.digits, 6);
        assert_eq!(probe.widest, 56.0);
        let da = cfg.data_area().unwrap();
        let bounds = probe.bounds.expect("drawn labels");
        assert_eq!(bounds.x, da.x as f32 - 5.0 - LABEL_GAP - 56.0);
        assert_eq!(bounds.width, 56.0);
        assert!(bounds.height > da.height as f32);
        let hidden = Probe::of(&cfg, &Element::AxisLabel(Side::Top), &measure);
        assert_eq!(hidden, Probe::default());
        assert_eq!(probe.with(hidden), probe);

        // Measured from the band's inner edge, the tick beyond the data
        // area: the labels reach the gap and 56 px, the 13 px title sits centred in the 110 px band.
        let mut titled = cfg.clone();
        titled.left_y.title_option.text.segments = crate::text::rich_segments_from_text("y");
        let title = Probe::of(&titled, &Element::AxisTitle(Side::Left), &measure);
        let inner = band_inner_edge(&Side::Left, RectF::from_rect(&da), 5.0);
        let band = BandMeasure::of(&Side::Left, inner, &probe, &title);
        assert_eq!(band.reach, Some(LABEL_GAP + 56.0));
        assert_eq!(band.title, Some((55.0 - 6.5, 55.0 + 6.5)));
    }

    // A title placed beside labels reaching `R` needs a band of
    // `R + gap + t + gap`; a title that does not clear them gets the offset
    // that puts its inner edge the gap past them.
    #[test]
    fn a_band_needs_room_for_its_labels_and_a_title_beside_them() {
        let both = BandMeasure {
            reach: Some(30.0),
            title: Some((40.0, 53.0)),
        };
        assert_eq!(both.required(), Some(30.0 + LABEL_GAP + 13.0 + LABEL_GAP));
        let labels = BandMeasure {
            reach: Some(30.0),
            title: None,
        };
        assert_eq!(labels.required(), Some(30.0 + LABEL_GAP));
        assert_eq!(BandMeasure::default().required(), None);
        // In a 110 px band the centred title's inner edge is at 48.5, clear
        // of labels reaching 30 + 4: it stays, and so does an offset one that
        // still clears them inside the band.
        assert_eq!(both.placed_offset(110.0, 0.0, None), None);
        assert_eq!(both.placed_offset(110.0, -14.5, None), None);
        assert_eq!(both.placed_offset(110.0, 40.0, None), None);
        // Into the labels, or out of the band: placed at 34, i.e. -14.5,
        // rounded outward.
        assert_eq!(both.placed_offset(110.0, -20.0, None), Some(-14.0));
        assert_eq!(both.placed_offset(110.0, 50.0, None), Some(-14.0));
        // A band of exactly 51 px: the title's inner edge goes from 19 to 34.
        assert_eq!(both.placed_offset(51.0, 0.0, None), Some(15.0));
        assert_eq!(both.placed_offset(51.0, 15.0, None), None);
        assert_eq!(labels.placed_offset(110.0, -20.0, None), None, "no title");
        // A colourbar title is already placed past its labels (#259): in the
        // 51 px band it needs no offset, and an offset that would pull it back
        // over them moves it to the same place.
        assert_eq!(both.placed_offset(51.0, 0.0, Some(LABEL_GAP)), None);
        assert_eq!(both.placed_offset(51.0, -10.0, Some(LABEL_GAP)), Some(0.0));
    }

    // auto_layout (#44).

    fn with_titles(left: &str) -> Config {
        let mut cfg = default_config();
        cfg.left_y.title_option.text.segments = crate::text::rich_segments_from_text(left);
        cfg.bottom_x.title_option.text.segments = crate::text::rich_segments_from_text("x");
        cfg
    }

    /// The parts of `element` as `measure` and the model place them.
    fn rects(cfg: &Config, element: Element, measure: &dyn MeasureText) -> Vec<RectF> {
        let mut out = Vec::new();
        crate::select::emit_element(
            cfg,
            &element,
            measure,
            &mut Parts(&mut |part| out.push(part.extent())),
        );
        out
    }

    /// The smallest gap between the title on `side` and any of its labels,
    /// across the band.
    fn title_clearance(cfg: &Config, side: Side, measure: &dyn MeasureText) -> f32 {
        let title = rects(cfg, Element::AxisTitle(side.clone()), measure);
        let title = title.first().expect("a placed title");
        rects(cfg, Element::AxisLabel(side.clone()), measure)
            .iter()
            .map(|l| match side {
                Side::Left => l.x - (title.x + title.width),
                Side::Right => title.x - (l.x + l.width),
                Side::Top => l.y - (title.y + title.height),
                Side::Bottom => title.y - (l.y + l.height),
            })
            .fold(f32::INFINITY, f32::min)
    }

    // Short labels beside a short title already clear it, so nothing moves:
    // the default document stays exactly as it is.
    #[test]
    fn auto_layout_keeps_a_document_that_already_clears() {
        let measure = Labels;
        let cfg = with_titles("y");
        assert!(title_clearance(&cfg, Side::Left, &measure) >= LABEL_GAP);
        assert_eq!(auto_layout(&cfg, &measure), cfg);
        assert_eq!(auto_layout(&default_config(), &measure), default_config());
    }

    /// The gap between the title on `side` and the outer edge of its band.
    fn title_outer_gap(cfg: &Config, side: Side, measure: &dyn MeasureText) -> f32 {
        let title = rects(cfg, Element::AxisTitle(side.clone()), measure)[0];
        let ca = RectF::from_rect(&cfg.chart_area);
        let inset = cfg.colorbar_band(&side);
        match side {
            Side::Left => title.x - (ca.x + inset),
            Side::Right => ca.x + ca.width - inset - (title.x + title.width),
            Side::Top => title.y - (ca.y + cfg.chart_title.top_margin + inset),
            Side::Bottom => ca.y + ca.height - inset - (title.y + title.height),
        }
    }

    // `-1.2345` labels (56 px) reach into the centred title of the default
    // 110 px left band. The band already holds labels, gap, title and gap, so
    // it is kept and the title gets the offset that puts it the gap past the
    // labels. In a 60 px band the band grows to exactly that sum and the
    // title fills it. The input is untouched and a second call changes
    // nothing.
    #[test]
    fn auto_layout_places_a_title_beside_its_labels_in_a_tight_band() {
        // Five digits: long, but not too long for `Decimal`.
        let measure = Labels;
        // Past the tick: the gap and 56 px; the title is 13 px thick.
        let (reach, title) = (LABEL_GAP + 56.0, 13.0);
        for (band, grown) in [
            (110.0, 110.0),
            (60.0, reach + LABEL_GAP + title + LABEL_GAP),
        ] {
            let mut cfg = with_titles("Optical depth");
            five_digit_labels(&mut cfg.left_y);
            cfg.left_y.out_margin = band;
            let input = cfg.clone();
            assert!(
                title_clearance(&cfg, Side::Left, &measure) < 0.0,
                "{band}: overlaps as laid out"
            );

            let out = auto_layout(&cfg, &measure);
            assert_eq!(cfg, input, "the input is not modified");
            assert_eq!(out.left_y.out_margin, grown, "{band}");
            let clearance = title_clearance(&out, Side::Left, &measure);
            assert!(
                (LABEL_GAP..LABEL_GAP + 1.0).contains(&clearance),
                "{band}: {clearance}"
            );
            assert!(title_outer_gap(&out, Side::Left, &measure) >= LABEL_GAP - 1.0);
            let offset = out.left_y.title_option.offset_y;
            assert!(offset < 0.0 && offset == offset.round(), "{band}: {offset}");
            // Everything else is the input's.
            let mut back = out.clone();
            back.left_y.out_margin = cfg.left_y.out_margin;
            back.left_y.title_option.offset_y = 0.0;
            assert_eq!(back, cfg);
            assert_eq!(auto_layout(&out, &measure), out, "{band}: idempotent");
        }
    }

    // A side with no labels and no title keeps its margin, however small,
    // and a side with labels but no title only needs room for the labels.
    #[test]
    fn auto_layout_sizes_a_band_without_a_title_for_its_labels_alone() {
        let measure = Labels;
        let mut cfg = default_config();
        five_digit_labels(&mut cfg.left_y);
        cfg.left_y.out_margin = 20.0;
        cfg.right_y.out_margin = 1.0;
        let out = auto_layout(&cfg, &measure);
        let reach = LABEL_GAP + 56.0;
        assert_eq!(out.left_y.out_margin, (reach + LABEL_GAP).ceil());
        assert_eq!(out.right_y.out_margin, 1.0);
        assert_eq!(out.top_x.out_margin, cfg.top_x.out_margin);
    }

    // A title the host offset into its labels is placed beside them, by
    // whole pixels; a title offset the other way, but still clear and inside
    // its band, is left where it is, as is a centred one that clears.
    #[test]
    fn auto_layout_leaves_a_title_that_clears_and_places_one_that_does_not() {
        let measure = Labels;
        let mut cfg = with_titles("y");
        assert_eq!(auto_layout(&cfg, &measure), cfg, "centred and clear");
        // Left: offset_y is screen x; +30 moves the title inward.
        cfg.left_y.title_option.offset_y = 30.0;
        assert!(title_clearance(&cfg, Side::Left, &measure) < LABEL_GAP);
        let out = auto_layout(&cfg, &measure);
        assert_eq!(out.left_y.out_margin, cfg.left_y.out_margin);
        let moved = out.left_y.title_option.offset_y;
        assert!(moved < 30.0 && moved == moved.round(), "{moved}");
        let clearance = title_clearance(&out, Side::Left, &measure);
        assert!(
            (LABEL_GAP..LABEL_GAP + 1.0).contains(&clearance),
            "{clearance}"
        );
        assert_eq!(auto_layout(&out, &measure), out);

        let mut outward = with_titles("y");
        outward.left_y.title_option.offset_y = -10.0;
        assert_eq!(auto_layout(&outward, &measure), outward);
    }

    // Labels printing more than five digits switch an axis to `Power`; a
    // Power, Timestamp or categorical axis and short labels keep theirs.
    #[test]
    fn auto_layout_switches_long_decimal_labels_to_power() {
        let long = Labels;
        let mut long_cfg = default_config();
        long_labels(&mut long_cfg.left_y);
        long_labels(&mut long_cfg.bottom_x);
        let out = auto_layout(&long_cfg, &long);
        assert_eq!(out.left_y.label_style.format, LabelFormat::Power);
        assert_eq!(out.bottom_x.label_style.format, LabelFormat::Power);
        // Hidden labels are not drawn, so they are never too long.
        assert_eq!(out.top_x.label_style.format, LabelFormat::Decimal);
        assert_eq!(auto_layout(&out, &long), out);

        let short = Labels;
        assert_eq!(auto_layout(&default_config(), &short), default_config());

        let mut kept = long_cfg.clone();
        kept.left_y.label_style.format =
            LabelFormat::Timestamp(crate::format::TimestampLabelFormat::default());
        kept.bottom_x.categories = Some(vec![
            crate::config::Category::new("a", "A"),
            crate::config::Category::new("b", "B"),
        ]);
        let out = auto_layout(&kept, &long);
        assert_eq!(
            out.left_y.label_style.format,
            kept.left_y.label_style.format
        );
        assert_eq!(out.bottom_x.label_style.format, LabelFormat::Decimal);
    }

    // A band that would leave no data area is not grown.
    #[test]
    fn auto_layout_never_grows_a_band_past_the_data_area() {
        let measure = Labels;
        let mut cfg = with_titles("Optical depth");
        // 60 + 5 + 32 + 5 px of margins leave 20 px; the 81 px band the
        // labels and title need would leave none.
        cfg.left_y.out_margin = 60.0;
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 122,
            height: 400,
        });
        assert!(cfg.data_area().is_ok());
        let out = auto_layout(&cfg, &measure);
        assert_eq!(out.left_y.out_margin, cfg.left_y.out_margin);
        assert!(out.data_area().is_ok());
    }

    // The colourbar's band grows from the strip outward the same way, and a
    // legend box dragged over the bar goes back to the first corner where it
    // clears it.
    #[test]
    fn auto_layout_fits_the_colorbar_band_and_clears_the_legend_off_it() {
        let measure = Labels;
        let mut cfg = default_config();
        let mut bar = crate::default::default_colorbar_options();
        five_digit_labels(&mut bar.axis);
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text.segments = crate::text::rich_segments_from_text("z");
        cfg.colorbar = Some(bar);
        cfg.legend.visible = true;
        cfg.legend.content.segments = crate::text::rich_segments_from_text("series");
        cfg.legend.offset_x = 120.0;
        cfg.legend.offset_y = 200.0;
        let out = auto_layout(&cfg, &measure);
        let bar = out.colorbar.as_ref().unwrap();
        let reach = LABEL_GAP + 56.0;
        assert_eq!(bar.axis.out_margin, reach + LABEL_GAP + 13.0 + LABEL_GAP);
        // The bar's title sits the gap past its labels.
        let labels = rects(&out, Element::ColorBarLabel, &measure);
        let title = rects(&out, Element::ColorBarTitle, &measure)[0];
        let gap = labels
            .iter()
            .map(|l| title.x - (l.x + l.width))
            .fold(f32::INFINITY, f32::min);
        assert!((LABEL_GAP..LABEL_GAP + 1.0).contains(&gap), "{gap}");
        assert_eq!((out.legend.offset_x, out.legend.offset_y), (0.0, 0.0));
        let legend = rects(&out, Element::Legend, &measure)[0];
        for element in [
            Element::ColorBar,
            Element::ColorBarLabel,
            Element::ColorBarTitle,
        ] {
            for r in rects(&out, element, &measure) {
                assert!(!rects_overlap(legend, r), "{legend:?} {r:?}");
            }
        }
        assert_eq!(auto_layout(&out, &measure), out);
    }

    // An overlay's labels and title widen the shared right band through the
    // overlay's own axis, and a subplot is laid out as a chart of its own.
    #[test]
    fn auto_layout_fits_overlay_and_subplot_bands() {
        let measure = Labels;
        let mut cfg = default_config();
        let mut overlay = crate::layer::OverlayLayer::default();
        overlay.y_axis.title_option.visible = true;
        overlay.y_axis.title_option.text.segments = crate::text::rich_segments_from_text("p");
        let right = cfg.margins().right;
        cfg.overlays.push(overlay);
        let mut sub = crate::layer::Subplot::default();
        sub.config = with_titles("Optical depth");
        sub.config.chart_area = ChartArea(Rect {
            x: 1000,
            y: 0,
            width: 600,
            height: 400,
        });
        cfg.subplots.push(sub);

        let out = auto_layout(&cfg, &measure);
        assert!(out.margins().right > right);
        let panel = out.overlay_panel(0).unwrap();
        assert!(title_clearance(&panel, Side::Right, &measure) >= LABEL_GAP - 1e-3);
        let chart = out.subplot_config(0).unwrap();
        assert!(title_clearance(&chart, Side::Left, &measure) >= LABEL_GAP - 1e-3);
        assert_eq!(
            out.subplots[0].config.chart_area,
            cfg.subplots[0].config.chart_area
        );
        assert_eq!(auto_layout(&out, &measure), out);
    }
}

#[cfg(test)]
mod break_map_tests {
    //! The broken-axis map (#200): the one value-to-fraction mapping the
    //! chrome, the annotations and the renderer's transform share.
    use super::{BreakMap, axis_fraction};
    use crate::config::{AxisBreak, AxisBreakStyle, AxisBreaks, AxisOptions, AxisScale};
    use crate::default::default_axis_options_y;

    /// The drawn length of the axes these tests map (#249): a gap of
    /// `gap` of it is `gap * LEN` pixels.
    const LEN: f32 = 1000.0;

    fn axis(min: f64, max: f64, cuts: &[(f64, f64)], gap: f32) -> AxisOptions {
        let mut axis = default_axis_options_y();
        axis.min = min;
        axis.max = max;
        axis.breaks = Some(AxisBreaks {
            ranges: cuts.iter().map(|&(a, b)| AxisBreak::new(a, b)).collect(),
            gap_px: gap * LEN,
            ..AxisBreaks::default()
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
        let mut a = default_axis_options_y();
        a.min = -3.0;
        a.max = 7.0;
        let before = axis_fraction(1.5, &a, LEN);
        a.breaks = Some(AxisBreaks::default());
        assert_eq!(axis_fraction(1.5, &a, LEN), before);
        assert_eq!(BreakMap::of(&a, LEN), BreakMap::IDENTITY);
    }
}
