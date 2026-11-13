use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, BarAlign, ColorBarOptions, Config, LegendCorner};
use crate::format::LabelFormat;
use crate::sink::{Border, DrawnPart, Parts, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, RichText, TextExtents};

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
    title: MeasuredTitle,
) -> TitlePlacement {
    chart_title_placement_in(RectF::from_rect(chart_area), top_margin, offset, title)
}

/// A chart title as its placement reads it: the ink of its text
/// ([`MeasureText::measure_rich`]), the line of its font at its font size
/// ([`MeasureText::font_line`]) and that font size, which sets the padding
/// of its band ([`Self::padding`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredTitle {
    /// The measured ink: the first line's rise above its baseline and
    /// everything below it.
    pub ink: TextExtents,
    /// The font's own line ascent and descent at the title's font size.
    pub font_line: TextExtents,
    /// The title's font size (`RichText::font_size`).
    pub font_size: f32,
}

impl MeasuredTitle {
    /// `text` measured by `measure`: its ink, and the line of its font
    /// family at its font size.
    pub fn of(text: &RichText, measure: &dyn MeasureText) -> Self {
        Self {
            ink: measure.measure_rich(text),
            font_line: measure.font_line(&text.font, text.font_size),
            font_size: text.font_size,
        }
    }

    /// The padding [`auto_layout`] fits above and below the title in its
    /// band: 0.3 em of its font size, at least the 8 px edge padding (8.4 px
    /// at the default 28 px title).
    pub fn padding(&self) -> f32 {
        AUTO_LAYOUT_EDGE_PADDING.max(AUTO_LAYOUT_TITLE_PADDING_EMS * self.font_size)
    }

    /// The reach above the first baseline: the font's line ascent, or the
    /// ink's when it rises higher (a superscript, a fraction, a ∑ limit).
    pub fn ascent(&self) -> f32 {
        self.font_line.ascent.max(self.ink.ascent)
    }

    /// The reach below the first baseline: the font's line descent, or the
    /// ink's when it reaches lower (a subscript, a second line).
    pub fn descent(&self) -> f32 {
        self.font_line.descent.max(self.ink.descent)
    }

    /// The room the title takes in its band above its first baseline: the
    /// padding and [`Self::ascent`].
    fn above(&self) -> f32 {
        self.padding() + self.ascent()
    }

    /// The room the title takes in its band below its first baseline:
    /// [`Self::descent`] and the padding.
    fn below(&self) -> f32 {
        self.descent() + self.padding()
    }
}

/// [`chart_title_placement`] in an `f32` rectangle: a chart that lays out
/// its own title band (the radial chart) places its title across its
/// canvas with it.
///
/// The title is centred across `chart_area` and set on a baseline in the
/// band `top_margin` high at its top, so titles of one font and size share
/// a baseline whatever their letters:
///
/// - The title's line, [`MeasuredTitle::ascent`] above the first baseline
///   and [`MeasuredTitle::descent`] below it (the font's line, or the ink
///   where it reaches further), is centred in the band. In the band
///   [`auto_layout`] fits, the line and [`MeasuredTitle::padding`] above and
///   below it, the baseline is the padding and the ascent below the band's
///   top. The placement reads no pixel constant, so a chart scaled by `s`
///   (a 2x export) places its title `s` times as far.
/// - The ink is then kept inside the band: in a band lower than the line,
///   the title moves down or up just enough to keep its ink in.
/// - Ink taller than the band is centred in it, as the band cannot hold it.
///
/// `offset` then moves the title (a drag, or [`auto_layout`]'s shared
/// baseline for a row of a grid); it is not held inside the band.
pub fn chart_title_placement_in(
    chart_area: RectF,
    top_margin: f32,
    offset: (f32, f32),
    title: MeasuredTitle,
) -> TitlePlacement {
    let ink = title.ink;
    let top = chart_area.y;
    let baseline = if ink.height() > top_margin {
        top + (top_margin - ink.height()) * 0.5 + ink.ascent
    } else {
        (top + centred_baseline(top_margin, title.above(), title.below()))
            .max(top + ink.ascent)
            .min(top + top_margin - ink.descent)
    };
    let origin = (
        chart_area.x + chart_area.width * 0.5 - ink.width * 0.5 + offset.0,
        baseline + offset.1,
    );
    TitlePlacement {
        origin,
        rotation_center: origin,
        rotation_degrees: 0.0,
    }
}

/// The baseline, from the top of a band `band` high, of a line that takes
/// `above` over its baseline and `below` under it, centred in the band.
/// [`chart_title_placement_in`] and [`align_titles`] share it, so a row's
/// titles that need no move get exactly none.
fn centred_baseline(band: f32, above: f32, below: f32) -> f32 {
    (band - above - below) * 0.5 + above
}

/// Room the default title band leaves around one default-size title line: the
/// default `top_margin` minus that line's nominal height, its font size (the
/// em box the renderer also gives a line without ink). Derived rather than
/// restated, so fitted bands follow the defaults if they change.
fn chart_title_band_padding() -> f32 {
    let defaults = crate::defaults::chart_title_options();
    (defaults.top_margin - defaults.text.font_size).max(0.0)
}

/// Grow `cfg.chart_title.top_margin` when the measured title is taller than
/// its band.
///
/// The title's height is its line: [`MeasuredTitle::ascent`] above the first
/// baseline and [`MeasuredTitle::descent`] below it, the font's line or the
/// ink where it reaches further. A band lower than that cannot hold the
/// title on its baseline ([`chart_title_placement`]), and a band lower than
/// the ink clips its first line. Only in that case the band grows to the
/// line's height plus the room the default band gives a single default-size
/// line, rounded up to a whole pixel. A title that already fits is left
/// exactly as it is, so the data area does not move when the title's letters
/// change.
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
    let measured = MeasuredTitle::of(&title.text, measure);
    let height = measured.ascent() + measured.descent();
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

/// The room [`colorbar_title_placement`] keeps between the colourbar's
/// labels and its title, and between the title and the band's outer edge:
/// half the label font ([`AUTO_LAYOUT_TITLE_GAP_EMS`]) and half the title
/// font ([`AUTO_LAYOUT_EDGE_PADDING_EMS`]), the proportional parts of what
/// [`auto_layout`] keeps (#270), so a scaled export keeps the 1x proportions.
fn colorbar_title_gaps(axis: &AxisOptions) -> (f32, f32) {
    (
        AUTO_LAYOUT_TITLE_GAP_EMS * axis.label_style.font_size.max(0.0),
        AUTO_LAYOUT_EDGE_PADDING_EMS * axis.title_option.text.font_size.max(0.0),
    )
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
    // and the band's outer edge: half the label font and half the title font
    // ([`colorbar_title_gaps`]).
    let (gap, edge) = colorbar_title_gaps(axis);
    let clear = if labels_reach.is_finite() && labels_reach > 0.0 {
        labels_reach + gap
    } else {
        0.0
    };
    // How far past the strip's outer edge the title's inner edge sits:
    // centred in the band, or past the labels, but no closer to the band's
    // outer edge (where the chart area may end) than the edge room.
    let centred = tick + (out_margin - extents.height()) * 0.5;
    let inner = centred.max(clear.min(tick + out_margin - extents.height() - edge));
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

/// The least padding [`auto_layout`] keeps between what a fitted band
/// holds and the band's outer edge, in pixels: twice [`LABEL_GAP`]. A band
/// with nothing to hold is this padding alone; one that holds text or a
/// drawn axis pads by [`AUTO_LAYOUT_EDGE_PADDING_EMS`] of its font when that
/// is more ([`edge_padding`]).
const AUTO_LAYOUT_EDGE_PADDING: f32 = 8.0;

/// The padding [`auto_layout`] keeps between what a fitted band holds and
/// the band's outer edge, as a fraction of the font size of the text at
/// that edge (the band's title, else the labels it holds, else the labels
/// of the axis whose line or ticks it holds): 0.5 em, never less than
/// [`AUTO_LAYOUT_EDGE_PADDING`] (9 px at the default 18 px labels, 11 px
/// beside the default 22 px axis title) (#270).
const AUTO_LAYOUT_EDGE_PADDING_EMS: f32 = 0.5;

/// The gap [`auto_layout`] keeps between an axis or colourbar title and its
/// labels, as a fraction of the labels' font size: 0.5 em, never less than
/// [`LABEL_GAP`] (8 px at 16 px labels) (#270). The colourbar title's
/// placement keeps the proportional part alone
/// ([`colorbar_title_placement`]), so a scaled export keeps the 1x
/// proportions.
const AUTO_LAYOUT_TITLE_GAP_EMS: f32 = 0.5;

/// The gap [`auto_layout`] keeps between a title and labels of font size
/// `label_size` ([`AUTO_LAYOUT_TITLE_GAP_EMS`], at least [`LABEL_GAP`]).
fn title_gap(label_size: f32) -> f32 {
    LABEL_GAP.max(AUTO_LAYOUT_TITLE_GAP_EMS * label_size)
}

/// The padding [`auto_layout`] keeps past text of font size `size` at a
/// band's outer edge ([`AUTO_LAYOUT_EDGE_PADDING_EMS`], at least
/// [`AUTO_LAYOUT_EDGE_PADDING`]); a band that holds nothing passes 0 and
/// gets the least padding.
fn edge_padding(size: f32) -> f32 {
    AUTO_LAYOUT_EDGE_PADDING.max(AUTO_LAYOUT_EDGE_PADDING_EMS * size)
}

/// The padding [`auto_layout`] keeps above and below the chart title in its
/// band, as a fraction of the title's font size: 0.3 em, never less than
/// [`AUTO_LAYOUT_EDGE_PADDING`] (8.4 px at the default 28 px title)
/// ([`MeasuredTitle::padding`]).
const AUTO_LAYOUT_TITLE_PADDING_EMS: f32 = 0.3;

/// What [`auto_layout`] reads from the parts of one emitted element: the
/// union of their extents, and of its text blocks the most digits one
/// prints, the widest one and the largest font size. Kept as numbers, so
/// measuring stores no part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Probe {
    bounds: Option<RectF>,
    digits: usize,
    widest: f32,
    font: f32,
}

impl Probe {
    /// Emit `element` of `cfg` into a fresh probe.
    fn of(cfg: &Config, element: &Element, measure: &dyn MeasureText) -> Self {
        let mut probe = Self::default();
        probe.emit(cfg, element, measure);
        probe
    }

    /// Emit `element` of `cfg` into this probe, with what it holds.
    fn emit(&mut self, cfg: &Config, element: &Element, measure: &dyn MeasureText) {
        crate::select::emit_element(cfg, element, measure, self);
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
            font: self.font.max(other.font),
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
        let (digits, size) = match text.text {
            TextRef::Plain { text, size, .. } => {
                (text.chars().filter(char::is_ascii_digit).count(), size)
            }
            TextRef::Rich(rich) => (
                rich.segments
                    .iter()
                    .filter(|s| s.text.is_ascii_digit())
                    .count(),
                rich.segments
                    .iter()
                    .map(|s| s.font_size.unwrap_or(rich.font_size))
                    .fold(0.0, f32::max),
            ),
        };
        self.digits = self.digits.max(digits);
        if size.is_finite() {
            self.font = self.font.max(size);
        }
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

/// One band as measured: how far the labels it holds reach past the inner
/// edge, and the near and far distance of its title, both outward from that
/// edge; with the gap its title keeps from the labels ([`title_gap`]) and
/// the padding at its outer edge ([`edge_padding`]). The labels are every
/// label run the band has to hold (see [`auto_layout`]); only the part past
/// the inner edge counts.
///
/// A legend box the host placed past the data area's edge on this side
/// adds `legend`, the room it needs from the inner edge (its reach and the
/// edge padding of its font), and, when it lies beside the title along the
/// band, `blocked`: its near and far distance outward, which the title
/// keeps the gap from ([`BandMeasure::hold_legend`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct BandMeasure {
    reach: Option<f32>,
    title: Option<(f32, f32)>,
    gap: f32,
    padding: f32,
    legend: Option<f32>,
    blocked: Option<(f32, f32)>,
}

impl BandMeasure {
    /// The band on `side` whose inner edge is `inner`, holding `labels` and
    /// `title`; `label_size` is the font size of the axis' own labels. The
    /// outer edge is padded for the title, or without one for the labels
    /// that run into the band (the largest font among them), or without
    /// either for `bare`: the font of the axis whose line or ticks the band
    /// holds, 0 (the least padding) when it holds nothing.
    fn of(
        side: &Side,
        inner: f32,
        labels: &Probe,
        title: &Probe,
        label_size: f32,
        bare: f32,
    ) -> Self {
        let reach = labels.bounds.map(|r| band_span(side, inner, r).1.max(0.0));
        let title_span = title.bounds.map(|r| band_span(side, inner, r));
        let edge = if title_span.is_some() {
            title.font
        } else if reach.is_some_and(|reach| reach > 0.0) {
            labels.font
        } else {
            bare
        };
        Self {
            reach,
            title: title_span,
            gap: title_gap(label_size),
            padding: edge_padding(edge),
            legend: None,
            blocked: None,
        }
    }

    /// Hold the legend box `legend` in the band on `side` whose inner edge
    /// is `inner`, `tick` past the data area, when the box reaches past the
    /// data area's edge on that side: the band then needs the box's reach
    /// past the inner edge and the edge padding of the legend's font. When
    /// the box also lies beside the title `title` along the band, the
    /// title is placed past it ([`BandMeasure::title_clear`]). A box inside
    /// the data area on this side changes nothing (#270).
    fn hold_legend(&mut self, side: &Side, inner: f32, tick: f32, legend: &Probe, title: &Probe) {
        let Some(r) = legend.bounds else {
            return;
        };
        let (near, far) = band_span(side, inner, r);
        if far <= -tick {
            return;
        }
        let room = far.max(0.0) + edge_padding(legend.font);
        self.legend = Some(self.legend.map_or(room, |held| held.max(room)));
        if title.bounds.is_some_and(|t| along_overlap(side, t, r)) {
            self.blocked = Some(match self.blocked {
                Some((n, f)) => (n.min(near), f.max(far)),
                None => (near, far),
            });
        }
    }

    /// Whether a title `thickness` thick whose inner edge is `at` comes
    /// within the gap of the legend beside it.
    fn title_blocked(&self, at: f32, thickness: f32) -> bool {
        const EPS: f32 = 1e-3;
        self.blocked.is_some_and(|(near, far)| {
            at < far + self.gap - EPS && at + thickness > near - self.gap + EPS
        })
    }

    /// Where the inner edge of a title `thickness` thick goes, outward from
    /// the inner edge: the gap past the labels, or, when a legend beside
    /// it would be in the way there, the gap past the legend.
    fn title_clear(&self, thickness: f32) -> f32 {
        let beside = self.reach.unwrap_or(0.0) + self.gap;
        match self.blocked {
            Some((_, far)) if self.title_blocked(beside, thickness) => far + self.gap,
            _ => beside,
        }
    }

    /// The band the labels and a title placed the gap past them need: the
    /// labels' reach, the gap, the title's thickness and the edge padding
    /// to the band's outer edge; the reach and the edge padding without a
    /// title, and the edge padding alone for a band with neither. A legend
    /// past the data area moves the title past it, and the band holds the
    /// legend and its padding too.
    fn required(&self) -> f32 {
        let reach = self.reach.unwrap_or(0.0);
        let held = match self.title {
            Some((near, far)) => self.title_clear(far - near) + (far - near) + self.padding,
            None => reach + self.padding,
        };
        self.legend.map_or(held, |legend| held.max(legend))
    }

    /// The outward title offset that places the title, in a band `band`
    /// wide where it is now offset outward by `offset`, beside its labels:
    /// its inner edge the gap past their reach, rounded outward to a whole
    /// pixel. `None` when there is no title, or when the title at `offset`
    /// already clears the labels by the gap and stays inside the band.
    fn placed_offset(&self, band: f32, offset: f32, pushed: Option<(f32, f32)>) -> Option<f32> {
        const EPS: f32 = 1e-3;
        let (near, far) = self.title?;
        let thickness = far - near;
        let clear = self.reach.unwrap_or(0.0) + self.gap;
        // The title's inner edge before the offset, as the placement puts
        // it: centred in the band, or for a colourbar title (`pushed`, the
        // gap it keeps past its labels and the room it keeps from the band's
        // outer edge, [`colorbar_title_gaps`]) past the labels when the
        // centre would sit on them (`colorbar_title_placement`, #259); then
        // moved by the offset.
        let centred = (band - thickness) * 0.5;
        let base = match pushed {
            Some((gap, edge)) => {
                centred.max((self.reach.unwrap_or(0.0) + gap).min(band - thickness - edge))
            }
            None => centred,
        };
        let at = base + offset;
        if at >= clear - EPS && at + thickness <= band + EPS && !self.title_blocked(at, thickness) {
            return None;
        }
        let placed = (self.title_clear(thickness) - base).ceil();
        (placed.is_finite() && placed != offset).then_some(placed)
    }

    /// The outward offset that puts an axis title, centred in a band `band`
    /// wide before the offset, the gap past its labels (or past a legend in
    /// the way, [`BandMeasure::title_clear`]), rounded outward to a whole
    /// pixel, wherever it sits now. `None` without a title.
    fn beside_offset(&self, band: f32) -> Option<f32> {
        let (near, far) = self.title?;
        let thickness = far - near;
        let placed = (self.title_clear(thickness) - (band - thickness) * 0.5).ceil();
        placed.is_finite().then_some(placed)
    }
}

/// Whether `a` and `b` share a stretch along the band on `side`: their
/// horizontal extents for the top and bottom bands, their vertical extents
/// for the left and right ones.
fn along_overlap(side: &Side, a: RectF, b: RectF) -> bool {
    match side {
        Side::Top | Side::Bottom => a.x < b.x + b.width && b.x < a.x + a.width,
        Side::Left | Side::Right => a.y < b.y + b.height && b.y < a.y + a.height,
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
/// - **`out_margin`.** The band of each side is set to exactly what it
///   holds, smaller or larger than it was: the reach of its labels past the
///   tick, the title gap, the title's thickness and an edge padding to the
///   band's outer edge, rounded up to a whole pixel; the reach and the
///   padding when the side has no title, and the padding alone when it holds
///   nothing. The title gap is half the font size of the axis' labels, at
///   least [`LABEL_GAP`]; the edge padding is half the font size of what
///   sits at the outer edge (the title, else the labels that run into the
///   band, the largest of their fonts, else the axis' own label font when
///   the band holds its drawn line or ticks), at least 8 px, which is also
///   the padding of a band that holds nothing drawn. Both scale with the
///   fonts, so a 2x export of the result is the 1x layout scaled (#270).
///   The labels a band holds are its own tick labels and category group
///   headers, and the part of any other label that runs past the data
///   area on its side: the end labels of the perpendicular axes (the top y
///   label in the top band, the last x label in the right band), and on
///   polar, ternary, 3D and parallel charts every label, and the axis titles
///   a ternary, 3D or polar chart places itself, wherever they are
///   drawn. A band change that would leave no data area is not made.
/// - **Title offsets.** A title that does not clear its labels by the
///   title gap, or leaves its band, gets the offset across the band
///   (`title_option.offset_y`) that puts its inner edge the gap past the
///   labels, rounded outward to a whole pixel. A title that already
///   clears them inside its band, centred or offset by the host, is left
///   where it is. The offset is an ordinary SSOT field: drag, hit testing and
///   selection follow it.
/// - **Overlays and the colourbar.** Each overlay axis' band is set the same
///   way from its own labels and title (the right margin is the widest band
///   of the group's Y axes, the top margin of its X axes), and so is the
///   colourbar's `axis.out_margin`, measured
///   from the strip; [`colorbar_title_placement`] puts its title half the
///   label font past its labels.
/// - **Chart title.** `chart_title.top_margin` is set to the title's line
///   ([`MeasuredTitle`]: the font's line ascent and descent, or the ink's
///   where it reaches further, as a superscript, a subscript, a formula or a
///   second line does) plus a padding above and below it of 0.3 em of the
///   title's font size, at least the edge padding, rounded up to a whole
///   pixel. The placement centres the line in the band
///   ([`chart_title_placement`]), so the first baseline sits that padding
///   and the ascent below the band's top, and titles of one font and size
///   share a baseline whatever their letters. A hidden or empty title gets
///   a band of 0, and the top axis band then holds the padding at the top
///   of the chart. This is `auto_layout`'s own rule:
///   [`fit_chart_title_margin`], which the title helpers use, only grows a
///   band too small for its title.
/// - **Legend.** A legend box that reaches past the data area is the host's
///   placement and takes priority: its corner and offsets are kept, and
///   each band it crosses (the document's legend and each overlay layer's)
///   grows to hold its reach past the band's inner edge and the edge
///   padding of the legend's font, measured from the box the legend
///   element emits. When that side's axis title, beside its labels, would
///   come within the title gap of the box, the title is placed the title
///   gap past the legend instead, and the band holds it there; the tick
///   labels stay with their axis. The legend is anchored to a data-area
///   corner, so its reach does not change while the bands do. A legend
///   inside the data area changes no band. A legend box over the colourbar
///   after that moves to the first corner, with zero offsets, where it
///   clears the bar.
/// - **Subplots** are laid out as charts of their own, and then the plots
///   of a grid are lined up. The document's own chart and each subplot are
///   cells; cells with the same chart-area x and width form a column, cells
///   with the same y and height a row. In a column every plot gets the
///   widest left and right margin of the column; in a row the deepest
///   bottom margin and the deepest top margin, after each titled cell of the
///   row gets the row's chart title band: the deepest reach above the
///   baseline and the deepest below it of the row's titles, with their
///   padding. Where a row holds more than one title, each gets the
///   `chart_title.offset_y` that puts its baseline where that deepest reach
///   puts it, whatever offset it had, so the titles share one baseline.
///   Then cells of a row with the same chart-area width get the same plot
///   width, and cells of a column with the same height the same plot
///   height, what a cell lacks going to its right and bottom bands (in a
///   regular grid this keeps the edges lined up). A band is widened through
///   its axis' `out_margin` (the right one through the group band, so
///   overlay axes and a colourbar count), and a title in a band wider than
///   it needs is placed the title gap past its labels, whatever offset it
///   had. A cell that shares no row or column with another keeps its own
///   fit, and a widening that would leave a cell no data area is not made.
///
/// Not covered: overlay axes drawn over each other or over the parent's
/// right or top axis (only `line_offset` separates them; a polar chart's radius
/// labels keep their gap from its angle labels by themselves, #272), an
/// overlay axis' title against a legend right of the plot, and labels that
/// collide with each other along an axis or at a corner.
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
    align_cells(&mut out, measure);
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
///
/// The four axis bands are measured together, from one drawing of the
/// panel, and then set: a change on one side moves the labels of the
/// others, which the next pass measures again.
fn auto_layout_pass(cfg: &mut Config, measure: &dyn MeasureText) -> bool {
    let doc: &Config = cfg;
    let (long, bands) = crate::select::on_drawn_panel(doc, |panel| {
        let labels = SIDES.map(|side| Probe::of(panel, &Element::AxisLabel(side), measure));
        let long = long_labels(panel, &labels, measure);
        let held = held_labels(panel, &labels, measure);
        let bands = SIDES
            .each_ref()
            .map(|side| axis_band_measure(Some(doc), panel, side, &held, measure));
        (long, bands)
    });
    if switch_long_labels(cfg, long, measure) {
        // The labels change: the next pass measures them again.
        return true;
    }
    let mut changed = false;
    for (side, measured) in SIDES.iter().zip(bands) {
        if let Some((band, drawn)) = measured {
            changed |= fit_band(cfg, &band, drawn, side, BandOwner::Axis(side));
        }
    }
    for (layer, side) in overlay_axes(cfg) {
        if !overlay_draws(cfg, layer, side) {
            continue;
        }
        let measured = cfg.overlay_panel(layer).and_then(|panel| {
            let labels = Probe::of(&panel, &Element::AxisLabel(side.clone()), measure);
            axis_band_measure(None, &panel, side, &labels, measure)
        });
        if let Some((band, drawn)) = measured {
            changed |= fit_band(cfg, &band, drawn, side, BandOwner::Overlay(layer, side));
        }
    }
    let measured =
        crate::select::on_drawn_panel(cfg, |panel| colorbar_band_measure(panel, measure));
    if let Some((side, band, drawn)) = measured {
        changed |= fit_band(cfg, &band, drawn, &side, BandOwner::ColorBar);
    }
    changed |= fit_title_band(cfg, measure);
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

/// Which labels of the drawn panel `panel` are too long: the four axes',
/// whose drawn labels are `labels` (in [`SIDES`] order), and the
/// colourbar's.
fn long_labels(
    panel: &Config,
    labels: &[Probe; 4],
    measure: &dyn MeasureText,
) -> ([bool; 4], bool) {
    let sides = std::array::from_fn(|i| labels_too_long(axis_ref(panel, &SIDES[i]), &labels[i]));
    let bar = panel.colorbar.as_ref().is_some_and(|bar| {
        labels_too_long(
            &bar.axis,
            &Probe::of(panel, &Element::ColorBarLabel, measure),
        )
    });
    (sides, bar)
}

/// Switch every axis of `cfg` whose labels are too long to
/// `LabelFormat::Power`: the four axes and the colourbar as `long` found
/// them ([`long_labels`]), and every overlay axis. Returns whether one
/// switched.
fn switch_long_labels(
    cfg: &mut Config,
    (sides, bar): ([bool; 4], bool),
    measure: &dyn MeasureText,
) -> bool {
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
    for (layer, side) in overlay_axes(cfg) {
        if !overlay_draws(cfg, layer, side) {
            continue;
        }
        let long = cfg.overlay_panel(layer).is_some_and(|panel| {
            let labels = Probe::of(&panel, &Element::AxisLabel(side.clone()), measure);
            labels_too_long(axis_ref(&panel, side), &labels)
        });
        if long && let Some(axis) = BandOwner::Overlay(layer, side).axis(cfg) {
            axis.label_style.format = LabelFormat::Power;
            changed = true;
        }
    }
    changed
}

/// Every (layer, side) an overlay of `cfg` may draw an axis on: the top
/// side (its own X axis) and the right side (its own Y axis). It borrows
/// nothing, so the loop body may edit `cfg`; [`overlay_draws`] says which of
/// them are drawn.
fn overlay_axes(cfg: &Config) -> impl Iterator<Item = (usize, &'static Side)> + use<> {
    (0..cfg.overlays.len()).flat_map(|layer| [(layer, &Side::Top), (layer, &Side::Right)])
}

/// Whether overlay `layer` of `cfg` draws an axis of its own on `side`.
fn overlay_draws(cfg: &Config, layer: usize, side: &Side) -> bool {
    cfg.overlays
        .get(layer)
        .is_some_and(|overlay| super::overlay_axis_on(overlay, side).is_some())
}

/// Whether the drawn panel `panel` places its axis titles along its own
/// edges rather than in the bands: a ternary or 3D chart, or a polar chart,
/// whose titles sit beside its disc (#272).
fn places_own_titles(panel: &Config) -> bool {
    panel.ternary().is_some() || panel.view_3d().is_some() || panel.polar().is_some()
}

/// Every label of the drawn panel `panel` an axis band may have to hold,
/// as one probe: the four axes' tick labels (`labels`), the category group
/// headers, a parallel chart's axis labels and the axis titles a ternary or
/// 3D chart places itself. Each band counts only the part past its inner
/// edge ([`BandMeasure::of`]), so one union serves the four sides.
fn held_labels(panel: &Config, labels: &[Probe; 4], measure: &dyn MeasureText) -> Probe {
    let mut held = labels
        .iter()
        .fold(Probe::default(), |all, side| all.with(*side));
    for side in [Side::Top, Side::Bottom] {
        held.emit(panel, &Element::CategoryGroupLabel(side), measure);
    }
    if let Some(parallel) = panel.parallel() {
        for axis in 0..parallel.axes.len() {
            held.emit(panel, &Element::ParallelAxisLabel(axis), measure);
        }
    }
    if places_own_titles(panel) {
        for side in SIDES {
            held.emit(panel, &Element::AxisTitle(side), measure);
        }
    }
    held
}

/// The band of the axis on `side` of the drawn panel `panel`, holding the
/// labels `held` and the axis' title, and the `out_margin` it was drawn
/// with. A title a ternary or 3D chart places itself is among `held`
/// instead ([`held_labels`]). With `doc`, the document `panel` is drawn
/// from, the band also holds every legend of the document placed past the
/// data area on `side` ([`BandMeasure::hold_legend`]): the panel's own and
/// each overlay layer's.
fn axis_band_measure(
    doc: Option<&Config>,
    panel: &Config,
    side: &Side,
    held: &Probe,
    measure: &dyn MeasureText,
) -> Option<(BandMeasure, f32)> {
    let da = panel.data_area().ok()?;
    let axis = axis_ref(panel, side);
    let tick = tick_contribution(axis);
    let inner = band_inner_edge(side, RectF::from_rect(&da), tick);
    let title = if places_own_titles(panel) {
        Probe::default()
    } else {
        Probe::of(panel, &Element::AxisTitle(side.clone()), measure)
    };
    // An axis line or tick drawn on the data area's edge or past it (the
    // plot frame) pads like the axis' labels, so a frame on a side without
    // text does not crowd the chart edge. A ternary, polar or parallel
    // chart draws its axes inside the data area: they pad by the least.
    let frame = Probe::of(panel, &Element::Axis(side.clone()), measure).bounds;
    let bare = match frame {
        Some(r) if band_span(side, inner, r).1 >= -tick - 1.0 => axis.label_style.font_size,
        _ => 0.0,
    };
    let label_size = axis.label_style.font_size;
    let mut band = BandMeasure::of(side, inner, held, &title, label_size, bare);
    if let Some(doc) = doc {
        let legend = Probe::of(panel, &Element::Legend, measure);
        band.hold_legend(side, inner, tick, &legend, &title);
        for layer in 0..doc.overlays.len() {
            if doc.overlays[layer].legend.is_some()
                && let Some(overlay) = doc.overlay_panel(layer)
            {
                let legend = Probe::of(&overlay, &Element::Legend, measure);
                band.hold_legend(side, inner, tick, &legend, &title);
            }
        }
    }
    Some((band, axis.out_margin))
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
        BandMeasure::of(
            &bar.side,
            inner,
            &labels,
            &title,
            bar.axis.label_style.font_size,
            0.0,
        ),
        bar.axis.out_margin,
    ))
}

/// The axis whose `out_margin` holds a band.
#[derive(Clone, Copy)]
enum BandOwner<'a> {
    /// One of the chart's four axes.
    Axis(&'a Side),
    /// The axis overlay layer `i` draws on a side: its own Y axis on the
    /// right, its own X axis on the top.
    Overlay(usize, &'a Side),
    /// The colourbar's axis.
    ColorBar,
}

impl BandOwner<'_> {
    fn axis<'c>(self, cfg: &'c mut Config) -> Option<&'c mut AxisOptions> {
        match self {
            BandOwner::Axis(side) => Some(axis_mut(cfg, side)),
            BandOwner::Overlay(layer, side) => {
                cfg.overlays.get_mut(layer).and_then(|overlay| match side {
                    Side::Top if !overlay.link_x => Some(&mut overlay.x_axis),
                    Side::Right if !overlay.link_y => Some(&mut overlay.y_axis),
                    _ => None,
                })
            }
            BandOwner::ColorBar => cfg.colorbar.as_mut().map(|bar| &mut bar.axis),
        }
    }
}

/// The band `owner`'s axis is drawn in, as an `out_margin`: its own, or
/// for the parent's right and top axes and the overlay axes the band of the
/// group ([`super::right_out_margin_effective`],
/// [`super::top_out_margin_effective`]).
fn drawn_band(cfg: &Config, owner: BandOwner<'_>) -> f32 {
    match owner {
        BandOwner::Axis(Side::Right) | BandOwner::Overlay(_, Side::Right) => {
            super::right_out_margin_effective(cfg)
        }
        BandOwner::Axis(Side::Top) | BandOwner::Overlay(_, Side::Top) => {
            super::top_out_margin_effective(cfg)
        }
        BandOwner::Overlay(_, _) => 0.0,
        BandOwner::Axis(side) => axis_ref(cfg, side).out_margin,
        BandOwner::ColorBar => cfg.colorbar.as_ref().map_or(0.0, |bar| bar.axis.out_margin),
    }
}

/// Fit one band of `cfg`: set the `out_margin` of `owner`'s axis to
/// exactly what `band` needs, smaller or larger, unless the change leaves
/// no data area; then place a title that does not clear its labels inside
/// the band beside them. `drawn` is the band the axis was drawn with: for
/// an overlay or the parent's right axis, the group's. Returns whether
/// anything changed.
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
    // A colourbar title is pushed past its labels by half their font
    // (`colorbar_title_placement`); an axis title is placed by its offset.
    let pushed = match owner {
        BandOwner::ColorBar => owner.axis(cfg).as_deref().map(colorbar_title_gaps),
        BandOwner::Axis(_) | BandOwner::Overlay(..) => None,
    };
    let mut changed = false;
    let mut width = drawn;
    let required = band.required().ceil();
    if required.is_finite() && required != before {
        let had_data_area = cfg.data_area().is_ok();
        let group = drawn_band(cfg, owner);
        if let Some(axis) = owner.axis(cfg) {
            axis.out_margin = required;
        }
        if had_data_area && cfg.data_area().is_err() {
            if let Some(axis) = owner.axis(cfg) {
                axis.out_margin = before;
            }
        } else {
            changed = true;
            // The group band moves by what the change moved it, if at all.
            width = drawn + (drawn_band(cfg, owner) - group);
        }
    }
    if let Some(axis) = owner.axis(cfg) {
        let sign = outward_sign(side);
        if let Some(outward) = band.placed_offset(width, sign * axis.title_option.offset_y, pushed)
        {
            axis.title_option.offset_y = sign * outward;
            changed = true;
        }
    }
    changed
}

/// Set `cfg.chart_title.top_margin` to what the title needs on its
/// baseline ([`chart_title_placement`]): [`MeasuredTitle::padding`],
/// [`MeasuredTitle::ascent`], [`MeasuredTitle::descent`] and the padding
/// again, rounded up to a whole pixel ([`title_room`]). A hidden or empty
/// title gets 0. Nothing changes when the measurement is not finite, when
/// the margin is NaN (validation reports it), or when the change would
/// leave no data area in a chart that had one. Returns whether the band
/// changed.
fn fit_title_band(cfg: &mut Config, measure: &dyn MeasureText) -> bool {
    let required = title_room(cfg, measure).map_or(0.0, |(above, below)| (above + below).ceil());
    let current = cfg.chart_title.top_margin;
    if !required.is_finite() || current.is_nan() || required == current {
        return false;
    }
    let had_data_area = cfg.data_area().is_ok();
    cfg.chart_title.top_margin = required;
    if had_data_area && cfg.data_area().is_err() {
        cfg.chart_title.top_margin = current;
        return false;
    }
    true
}

/// The room the chart title of `cfg` takes in its band above and below its
/// first baseline, each with its padding ([`MeasuredTitle`]); `None` for a
/// hidden or empty title.
fn title_room(cfg: &Config, measure: &dyn MeasureText) -> Option<(f32, f32)> {
    let title = &cfg.chart_title;
    (title.visible && !title.text.segments.is_empty()).then(|| {
        let measured = MeasuredTitle::of(&title.text, measure);
        (measured.above(), measured.below())
    })
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

/// The plot margins of one cell of a document ([`Config::margins`]) and its
/// chart title band: what [`auto_layout`] lines up across a grid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CellBands {
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
    title: f32,
}

impl CellBands {
    /// What `cfg` has now.
    fn of(cfg: &Config) -> Self {
        let m = cfg.margins();
        Self {
            top: m.top,
            bottom: m.bottom,
            left: m.left,
            right: m.right,
            title: cfg.chart_title.top_margin,
        }
    }

    /// The margin on `side`.
    fn side(&self, side: &Side) -> f32 {
        match side {
            Side::Top => self.top,
            Side::Bottom => self.bottom,
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }
}

/// Cell `i` of the document `doc`: its own chart for 0, subplot `i - 1`
/// after it, as stored (bands do not depend on linked ranges).
fn cell(doc: &Config, i: usize) -> &Config {
    match i.checked_sub(1) {
        None => doc,
        Some(index) => &doc.subplots[index].config,
    }
}

/// [`cell`], to change.
fn cell_mut(doc: &mut Config, i: usize) -> &mut Config {
    match i.checked_sub(1) {
        None => doc,
        Some(index) => &mut doc.subplots[index].config,
    }
}

/// Whether two cells share a grid column: the same chart-area x and width.
fn same_column(a: &Config, b: &Config) -> bool {
    a.chart_area.x == b.chart_area.x && a.chart_area.width == b.chart_area.width
}

/// Whether two cells share a grid row: the same chart-area y and height.
fn same_row(a: &Config, b: &Config) -> bool {
    a.chart_area.y == b.chart_area.y && a.chart_area.height == b.chart_area.height
}

/// Whether cell `i` of `doc` shares a row or a column with another cell.
fn in_grid(doc: &Config, i: usize) -> bool {
    let me = cell(doc, i);
    (0..=doc.subplots.len())
        .filter(|&k| k != i)
        .map(|k| cell(doc, k))
        .any(|other| same_row(me, other) || same_column(me, other))
}

/// Whether a cell has a chart title band.
fn titled(cfg: &Config) -> bool {
    cfg.chart_title.top_margin > 0.0
}

/// The chart titles of the row of one cell: the deepest room above and
/// below the first baseline among the titled cells of the row
/// ([`title_room`]), and whether a titled cell other than that one is
/// among them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RowTitles {
    above: f32,
    below: f32,
    shared: bool,
}

impl RowTitles {
    /// The titles of the row of cell `i` of `doc`, itself included.
    fn of(doc: &Config, i: usize, measure: &dyn MeasureText) -> Self {
        let me = cell(doc, i);
        let mut row = Self::default();
        for k in 0..=doc.subplots.len() {
            let other = cell(doc, k);
            if !same_row(me, other) || !titled(other) {
                continue;
            }
            if let Some((above, below)) = title_room(other, measure) {
                row.above = row.above.max(above);
                row.below = row.below.max(below);
                row.shared |= k != i;
            }
        }
        row
    }

    /// The title band that holds every title of the row on one baseline.
    fn band(self) -> f32 {
        (self.above + self.below).ceil()
    }
}

/// The edges cell `i` of `doc` is lined up to: the widest left and right
/// margins of its column, the deepest bottom margin of its row, and the
/// deepest top margin of its row once every titled cell of the row has the
/// row's chart title band (a cell without a title band keeps none). The
/// row's title band holds the deepest reach above the baseline and the
/// deepest below it of the row's titles ([`RowTitles`]), so the titles can
/// share one baseline ([`align_titles`]), and is at least the deepest title
/// band of the row. A cell alone in its row and column gets its own bands.
///
/// Raising a cell to these bands leaves every maximum unchanged, so the
/// cells can be raised one after another in place.
fn edge_alignment(doc: &Config, i: usize, measure: &dyn MeasureText) -> CellBands {
    let me = cell(doc, i);
    let cells = doc.subplots.len() + 1;
    let row_title = (0..cells)
        .map(|k| cell(doc, k))
        .filter(|other| same_row(me, other) && titled(other))
        .fold(me.chart_title.top_margin, |deepest, other| {
            deepest.max(other.chart_title.top_margin)
        })
        .max(RowTitles::of(doc, i, measure).band());
    let mut to = CellBands::of(me);
    if titled(me) {
        to.title = row_title;
    }
    for other in (0..cells).map(|k| cell(doc, k)) {
        let m = other.margins();
        if same_column(me, other) {
            to.left = to.left.max(m.left);
            to.right = to.right.max(m.right);
        }
        if same_row(me, other) {
            let title = other.chart_title.top_margin;
            let aligned = if titled(other) { row_title } else { title };
            to.top = to.top.max(m.top - title + aligned);
            to.bottom = to.bottom.max(m.bottom);
        }
    }
    to
}

/// Put the chart titles of each row of `doc` that holds more than one on
/// one baseline: the baseline of the row's line, the deepest reach above
/// and below the baseline of its titles ([`RowTitles`]), centred in the
/// band as [`chart_title_placement`] centres one title's. A titled cell
/// whose band holds the row's titles ([`RowTitles::band`]) gets the
/// `chart_title.offset_y` that moves its baseline there, whatever offset it
/// had; a title whose own line is the row's gets 0. The offsets depend only
/// on the titles' text and the bands, so the cells can be set one after
/// another in place.
fn align_titles(doc: &mut Config, measure: &dyn MeasureText) {
    for i in 0..=doc.subplots.len() {
        let me = cell(doc, i);
        let row = RowTitles::of(doc, i, measure);
        let band = me.chart_title.top_margin;
        if !row.shared || !titled(me) || band < row.band() {
            continue;
        }
        let Some((above, below)) = title_room(me, measure) else {
            continue;
        };
        let shared = centred_baseline(band, row.above, row.below);
        cell_mut(doc, i).chart_title.offset_y = shared - centred_baseline(band, above, below);
    }
}

/// The bands that give cell `i` of `doc` the plot size of its neighbours:
/// the widest plot margins across (left and right) of the cells of its row
/// with its chart-area width, the deepest down (top and bottom) of the cells
/// of its column with its height. What its own margins lack goes to the
/// right and the bottom band. In a regular grid, after
/// [`edge_alignment`], every cell of a column lacks the same width and
/// every cell of a row the same height, so the edges stay lined up.
fn size_alignment(doc: &Config, i: usize) -> CellBands {
    let me = cell(doc, i);
    let mut to = CellBands::of(me);
    let (own_across, own_down) = (to.left + to.right, to.top + to.bottom);
    let (mut across, mut down) = (own_across, own_down);
    for other in (0..=doc.subplots.len()).map(|k| cell(doc, k)) {
        let m = other.margins();
        if same_row(me, other) && other.chart_area.width == me.chart_area.width {
            across = across.max(m.left + m.right);
        }
        if same_column(me, other) && other.chart_area.height == me.chart_area.height {
            down = down.max(m.top + m.bottom);
        }
    }
    to.right += across - own_across;
    to.bottom += down - own_down;
    to
}

/// Line up the plots of the cells of `doc` that share a grid row or column,
/// each laid out on its own first: their edges ([`edge_alignment`]), then
/// their sizes ([`size_alignment`]), then the chart titles of each row on
/// one baseline ([`align_titles`]) and the axis titles in the bands that
/// grew ([`place_widened_titles`]).
fn align_cells(doc: &mut Config, measure: &dyn MeasureText) {
    let cells = doc.subplots.len() + 1;
    for i in 0..cells {
        let to = edge_alignment(doc, i, measure);
        raise_bands(cell_mut(doc, i), to);
    }
    for i in 0..cells {
        let to = size_alignment(doc, i);
        raise_bands(cell_mut(doc, i), to);
    }
    align_titles(doc, measure);
    for i in 0..cells {
        if !in_grid(doc, i) {
            continue;
        }
        match i.checked_sub(1) {
            None => {
                place_widened_titles(doc, measure);
            }
            Some(index) => {
                let Some(mut chart) = doc.subplot_config(index) else {
                    continue;
                };
                if place_widened_titles(&mut chart, measure) {
                    adopt_chart_layout(&mut doc.subplots[index].config, &chart);
                }
            }
        }
    }
}

/// Raise the bands of one cell to `to`, never lower them: the chart title
/// band to its own, then each axis band by what its plot margin lacks (the
/// right and top ones through the group bands,
/// [`super::right_out_margin_effective`] and
/// [`super::top_out_margin_effective`], so overlay axes and a colourbar
/// count). Nothing changes when that leaves
/// a chart that had a data area without one.
fn raise_bands(chart: &mut Config, to: CellBands) {
    let had_data_area = chart.data_area().is_ok();
    let fitted = (
        chart.chart_title.top_margin,
        chart.top_x.out_margin,
        chart.bottom_x.out_margin,
        chart.left_y.out_margin,
        chart.right_y.out_margin,
    );
    if to.title > chart.chart_title.top_margin {
        chart.chart_title.top_margin = to.title;
    }
    let now = CellBands::of(chart);
    for side in &SIDES {
        let by = to.side(side) - now.side(side);
        if by > 0.0 {
            let band = match side {
                Side::Right => super::right_out_margin_effective(chart),
                Side::Top => super::top_out_margin_effective(chart),
                Side::Bottom | Side::Left => axis_ref(chart, side).out_margin,
            };
            axis_mut(chart, side).out_margin = band + by;
        }
    }
    if had_data_area && chart.data_area().is_err() {
        (
            chart.chart_title.top_margin,
            chart.top_x.out_margin,
            chart.bottom_x.out_margin,
            chart.left_y.out_margin,
            chart.right_y.out_margin,
        ) = fitted;
    }
}

/// Place each axis title of `chart` whose band is wider than what it holds
/// (the band [`BandMeasure::required`] for its labels and title, as
/// [`fit_band`] sets it) the title gap past its labels
/// ([`BandMeasure::beside_offset`]) rather than centred in the band. For
/// the parent's right axis and the overlay axes the band is the group's.
/// Returns whether an offset changed.
fn place_widened_titles(chart: &mut Config, measure: &dyn MeasureText) -> bool {
    let doc: &Config = chart;
    let bands = crate::select::on_drawn_panel(doc, |panel| {
        let labels = SIDES.map(|side| Probe::of(panel, &Element::AxisLabel(side), measure));
        let held = held_labels(panel, &labels, measure);
        SIDES
            .each_ref()
            .map(|side| axis_band_measure(Some(doc), panel, side, &held, measure))
    });
    let mut changed = false;
    for (side, measured) in SIDES.iter().zip(bands) {
        if let Some((band, _)) = measured {
            changed |= place_beside(chart, &band, side, BandOwner::Axis(side));
        }
    }
    for (layer, side) in overlay_axes(chart) {
        if !overlay_draws(chart, layer, side) {
            continue;
        }
        let measured = chart.overlay_panel(layer).and_then(|panel| {
            let labels = Probe::of(&panel, &Element::AxisLabel(side.clone()), measure);
            axis_band_measure(None, &panel, side, &labels, measure)
        });
        if let Some((band, _)) = measured {
            changed |= place_beside(chart, &band, side, BandOwner::Overlay(layer, side));
        }
    }
    changed
}

/// Give the title of `owner`'s axis, whose band on `side` measures `band`,
/// the offset that puts it the title gap past its labels, when the band it
/// is drawn in is wider than `band` needs. Returns whether the offset
/// changed.
fn place_beside(cfg: &mut Config, band: &BandMeasure, side: &Side, owner: BandOwner<'_>) -> bool {
    let width = drawn_band(cfg, owner);
    if width <= band.required().ceil() {
        return false;
    }
    let Some(outward) = band.beside_offset(width) else {
        return false;
    };
    let offset = outward_sign(side) * outward;
    match owner.axis(cfg) {
        Some(axis) if axis.title_option.offset_y != offset => {
            axis.title_option.offset_y = offset;
            true
        }
        _ => false,
    }
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
        adopt_axis_layout(&mut to.x_axis, &from.x_axis);
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
    use crate::layout::Rect;

    /// `cross_at` is a value of the perpendicular axis: the bottom axis line
    /// crosses `left_y` at it, the left one `bottom_x`, through the pixel
    /// centre under it and never past the plot's edge lines.
    #[test]
    fn axis_lines_cross_the_perpendicular_axis_at_cross_at() {
        let mut config = crate::defaults::config();
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
        config.set_polar(Some(crate::defaults::polar_options()));
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
        let mut cfg = crate::defaults::config();
        cfg.chart_title.text.segments = crate::text::rich_segments_from_text(text);
        cfg
    }

    #[test]
    fn the_title_band_padding_comes_from_the_defaults() {
        let defaults = crate::defaults::chart_title_options();
        assert_eq!(
            chart_title_band_padding(),
            defaults.top_margin - defaults.text.font_size
        );
    }

    // A three-line title (39 px) is taller than the default 32 px band, so
    // it cannot sit on its baseline and its ink would be clipped. Fitting
    // grows the band to the block plus the default padding, and the
    // placement keeps the block inside it.
    #[test]
    fn fitting_grows_the_band_to_hold_a_multi_line_title() {
        let mut cfg = titled("one\ntwo\nthree");
        let m = LineMeasure.measure_rich(&cfg.chart_title.text);
        assert!(m.height() > cfg.chart_title.top_margin);

        assert!(fit_chart_title_margin(&mut cfg, &LineMeasure));
        let band = cfg.chart_title.top_margin;
        assert_eq!(band, (m.height() + chart_title_band_padding()).ceil());

        let ca = &cfg.chart_area;
        let measured = MeasuredTitle::of(&cfg.chart_title.text, &LineMeasure);
        let rect = chart_title_placement(ca, band, (0.0, 0.0), measured).rect(m);
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
        let mut empty = crate::defaults::config();
        assert!(!fit_chart_title_margin(&mut empty, &LineMeasure));
        assert_eq!(empty, crate::defaults::config());

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

    /// A measure with a font line of 12 px up and 4 px down whose ink
    /// depends on the letters: 10 px up for every title, 3 px down with a
    /// descender, none without; 15 px up with a superscript, 6 px down with
    /// a subscript, 22 px up and 9 px down with a `∑` (a formula with
    /// limits), and 16 px more down per further line. 8 px per character.
    struct Ink;
    impl MeasureText for Ink {
        fn measure_rich(&self, rt: &crate::text::RichText) -> TextExtents {
            let seg = |f: &dyn Fn(&crate::text::RichSegment) -> bool| rt.segments.iter().any(f);
            let lines = rt.segments.iter().filter(|s| s.text == '\n').count() as f32;
            let mut ascent: f32 = 10.0;
            let mut descent: f32 = 0.0;
            if seg(&|s| "gjpqy".contains(s.text)) {
                descent = 3.0;
            }
            if seg(&|s| s.superscript) {
                ascent = 15.0;
            }
            if seg(&|s| s.subscript) {
                descent = descent.max(6.0);
            }
            if seg(&|s| s.text == '∑') {
                (ascent, descent) = (22.0, 9.0);
            }
            TextExtents {
                width: rt.segments.len() as f32 * 8.0,
                ascent,
                descent: descent + 16.0 * lines,
            }
        }

        fn font_line(&self, _: &str, _: f32) -> TextExtents {
            TextExtents {
                width: 0.0,
                ascent: 12.0,
                descent: 4.0,
            }
        }
    }

    /// The first baseline and the ink rectangle of the chart title of `cfg`
    /// as [`Ink`] places it, relative to the top of its chart area.
    fn title_ink(cfg: &Config) -> (f32, RectF) {
        let measured = MeasuredTitle::of(&cfg.chart_title.text, &Ink);
        let t = &cfg.chart_title;
        let placed = chart_title_placement(
            &cfg.chart_area,
            t.top_margin,
            (t.offset_x, t.offset_y),
            measured,
        );
        let rect = placed.rect(measured.ink);
        let top = cfg.chart_area.y as f32;
        (placed.origin.1 - top, rect.translated(0.0, -top))
    }

    /// Whether `rect`, relative to the top of the chart area, lies in a
    /// title band `band` high.
    fn in_band(rect: RectF, band: f32) -> bool {
        rect.y >= -1e-3 && rect.y + rect.height <= band + 1e-3
    }

    // Titles of one font and size sit on one baseline whether or not their
    // letters descend: in the band `auto_layout` fits, which is the same for
    // both, and in a host-set band, the default one or a larger one. In the
    // fitted band the baseline is the padding and the font's ascent below
    // the band's top, give or take half the pixel the band is rounded up by.
    #[test]
    fn titles_with_and_without_descenders_share_the_baseline() {
        let (low, flat) = (titled("Temperature"), titled("Pressure"));
        let (a, b) = (auto_layout(&low, &Ink), auto_layout(&flat, &Ink));
        assert_eq!(a.chart_title.top_margin, b.chart_title.top_margin);
        // 28 px font: 8.4 px padding, the font's 12 + 4 px line.
        assert_eq!(a.chart_title.top_margin, (2.0f32 * 8.4 + 16.0).ceil());
        // The 33 px band centres the 32.8 px of line and padding.
        assert!((title_ink(&a).0 - (0.1 + 8.4 + 12.0)).abs() < 1e-3);
        assert_eq!(title_ink(&a).0, title_ink(&b).0);
        for band in [32.0, 60.0, 120.0] {
            let (mut a, mut b) = (low.clone(), flat.clone());
            a.chart_title.top_margin = band;
            b.chart_title.top_margin = band;
            let ((at, ar), (bt, br)) = (title_ink(&a), title_ink(&b));
            assert_eq!(at, bt, "band {band}");
            assert!(in_band(ar, band) && in_band(br, band), "band {band}");
        }
        // The offsets still move the title.
        let mut moved = a.clone();
        moved.chart_title.offset_x = 5.0;
        moved.chart_title.offset_y = 7.0;
        let ((y, r), (y0, r0)) = (title_ink(&moved), title_ink(&a));
        assert!((y - y0 - 7.0).abs() < 1e-3 && (r.x - r0.x - 5.0).abs() < 1e-3);
    }

    // Ink that reaches past the font's line (a superscript, a subscript, a
    // formula with limits, a second line) still fits the band `auto_layout`
    // fits, with the padding above it and below it.
    #[test]
    fn scripted_two_line_and_math_titles_fit_their_band() {
        let mut formula = titled("∑ x");
        formula.chart_title.text.segments[0].text = '∑';
        for cfg in [
            titled("CO₂"),
            titled("x² + y²"),
            titled("one\ntwo"),
            formula,
        ] {
            let out = auto_layout(&cfg, &Ink);
            let band = out.chart_title.top_margin;
            let (_, rect) = title_ink(&out);
            assert!(in_band(rect, band), "{band} {rect:?}");
            assert!(rect.y >= 8.4 - 1e-3, "{rect:?}");
            assert!(
                band - (rect.y + rect.height) >= 8.4 - 1e-3,
                "{band} {rect:?}"
            );
            assert_eq!(auto_layout(&out, &Ink), out);
        }
    }

    // In a row of a grid the titles share one band and one baseline, the
    // deepest of the row's, even when one reaches above the font's line and
    // another below it; the titles of another row keep their own.
    #[test]
    fn an_aligned_row_shares_one_baseline() {
        let mut doc = grid_cell(0, 0, "Pressure");
        let mut formula = grid_cell(480, 0, "∑ x");
        formula.chart_title.text.segments[0].text = '∑';
        formula.chart_title.offset_y = 30.0;
        let mut row = [
            formula,
            grid_cell(960, 0, "one\ntwo"),
            grid_cell(0, 420, "Temperature"),
            grid_cell(480, 420, "Humidity"),
        ];
        // A dragged title of a row is put back on the row's baseline.
        row[2].chart_title.offset_y = -6.0;
        for config in row {
            let mut sub = crate::defaults::subplot();
            sub.config = config;
            doc.subplots.push(sub);
        }
        let out = auto_layout(&doc, &Ink);
        let cells: Vec<Config> = std::iter::once(out.clone())
            .chain((0..4).map(|i| out.subplot_config(i).unwrap()))
            .collect();
        let placed: Vec<(f32, RectF)> = cells.iter().map(title_ink).collect();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        for i in [1, 2] {
            assert!(close(placed[i].0, placed[0].0), "top row baseline {i}");
            assert_eq!(
                cells[i].chart_title.top_margin,
                cells[0].chart_title.top_margin
            );
        }
        // The formula's 22 px rise sets the top row's baseline, the second
        // line's 16 px reach below it its band; the 55 px band centres the
        // 54.8 px of both and the padding.
        assert!(close(placed[0].0, 0.1 + 8.4 + 22.0), "{}", placed[0].0);
        assert_eq!(
            cells[0].chart_title.top_margin,
            (8.4f32 + 22.0 + 16.0 + 8.4).ceil()
        );
        assert!(close(placed[3].0, placed[4].0), "bottom row baseline");
        assert!(close(placed[3].0, 0.1 + 8.4 + 12.0), "{}", placed[3].0);
        for (cfg, (_, rect)) in cells.iter().zip(&placed) {
            assert!(in_band(*rect, cfg.chart_title.top_margin), "{rect:?}");
        }
        assert_eq!(auto_layout(&out, &Ink), out, "idempotent");
    }

    // A host-set band lower than the title's line keeps the ink inside it:
    // the line is centred, the ink moved just inside, and two titles of one
    // font keep one baseline while their ink fits. A title taller than its
    // band is centred in it, as before the baseline placement.
    #[test]
    fn a_small_fixed_band_keeps_the_ink_inside_and_centres_a_taller_title() {
        for band in [13.0, 14.0, 16.0, 18.0, 24.0] {
            let mut low = titled("Temperature");
            low.chart_title.top_margin = band;
            let mut flat = titled("Pressure");
            flat.chart_title.top_margin = band;
            let ((lt, lr), (ft, fr)) = (title_ink(&low), title_ink(&flat));
            assert!(in_band(lr, band) && in_band(fr, band), "band {band}");
            if band >= 16.0 {
                assert_eq!(lt, ft, "band {band}");
            }
        }
        let mut tall = titled("one\ntwo\nthree");
        tall.chart_title.top_margin = 20.0;
        let ink = Ink.measure_rich(&tall.chart_title.text);
        let (baseline, rect) = title_ink(&tall);
        assert_eq!(baseline, (20.0 - ink.height()) * 0.5 + ink.ascent);
        assert_eq!(rect.y + rect.height * 0.5, 10.0);
    }

    // The placement reads no pixel constant: a chart and its title scaled
    // by 2 (a 2x export) place the title twice as far, in a band that has
    // room for the padding and in one that has not.
    #[test]
    fn the_title_placement_scales_with_the_chart() {
        let one = MeasuredTitle {
            ink: TextExtents {
                width: 80.0,
                ascent: 10.0,
                descent: 3.0,
            },
            font_line: TextExtents {
                width: 0.0,
                ascent: 12.0,
                descent: 4.0,
            },
            font_size: 14.0,
        };
        let two = MeasuredTitle {
            ink: TextExtents {
                width: 160.0,
                ascent: 20.0,
                descent: 6.0,
            },
            font_line: TextExtents {
                width: 0.0,
                ascent: 24.0,
                descent: 8.0,
            },
            font_size: 28.0,
        };
        let area = RectF {
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        };
        let twice = RectF {
            x: 20.0,
            y: 40.0,
            width: 600.0,
            height: 400.0,
        };
        for band in [14.0, 20.0, 40.0] {
            let a = chart_title_placement_in(area, band, (3.0, 2.0), one).origin;
            let b = chart_title_placement_in(twice, 2.0 * band, (6.0, 4.0), two).origin;
            assert!(
                (b.0 - 2.0 * a.0).abs() < 1e-3 && (b.1 - 2.0 * a.1).abs() < 1e-3,
                "band {band}: {a:?} {b:?}"
            );
        }
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
        let axis = crate::defaults::axis_options_colorbar();
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
        let mut axis = crate::defaults::axis_options_colorbar();
        // A band wide enough for the labels and the title, whose centre
        // would sit on the labels (#259).
        axis.out_margin = 60.0;
        let m = extents();
        let reach = 30.0;
        let clear = reach + AUTO_LAYOUT_TITLE_GAP_EMS * axis.label_style.font_size - 1e-3;
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
        let mut cfg = crate::defaults::config();
        let mut bar = crate::defaults::color_bar_options();
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
        let mut cfg = crate::defaults::config();
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

        assert_eq!(probe.font, LABEL_SIZE);

        // Measured from the band's inner edge, the tick beyond the data
        // area: the labels reach the gap and 56 px, the 13 px title sits
        // centred in the 110 px band. The title keeps half the 18 px label
        // font from the labels, and the band's outer edge is padded by half
        // the 22 px title font.
        let mut titled = cfg.clone();
        titled.left_y.title_option.text.segments = crate::text::rich_segments_from_text("y");
        let title = Probe::of(&titled, &Element::AxisTitle(Side::Left), &measure);
        assert_eq!(title.font, TITLE_SIZE);
        let inner = band_inner_edge(&Side::Left, RectF::from_rect(&da), 5.0);
        let band = BandMeasure::of(&Side::Left, inner, &probe, &title, LABEL_SIZE, 0.0);
        assert_eq!(band.reach, Some(LABEL_GAP + 56.0));
        assert_eq!(band.title, Some((55.0 - 6.5, 55.0 + 6.5)));
        assert_eq!((band.gap, band.padding), (9.0, 11.0));
        // Without a title the labels set the padding; with nothing past the
        // inner edge the band pads for its frame, or by the least padding
        // without one.
        let none = Probe::default();
        let untitled = BandMeasure::of(&Side::Left, inner, &probe, &none, LABEL_SIZE, 0.0);
        assert_eq!(untitled.padding, 9.0);
        let outside = BandMeasure::of(&Side::Left, 0.0, &probe, &none, LABEL_SIZE, 0.0);
        assert_eq!(outside.reach, Some(0.0));
        assert_eq!(outside.padding, AUTO_LAYOUT_EDGE_PADDING);
        let framed = BandMeasure::of(&Side::Left, 0.0, &probe, &none, LABEL_SIZE, 30.0);
        assert_eq!(framed.padding, 15.0);
    }

    // A title placed beside labels reaching `R` needs a band of
    // `R + gap + t + padding`; a title that does not clear them gets the
    // offset that puts its inner edge the gap past them.
    #[test]
    fn a_band_needs_room_for_its_labels_and_a_title_beside_them() {
        let both = BandMeasure {
            reach: Some(30.0),
            title: Some((40.0, 53.0)),
            gap: LABEL_GAP,
            padding: AUTO_LAYOUT_EDGE_PADDING,
            legend: None,
            blocked: None,
        };
        assert_eq!(
            both.required(),
            30.0 + LABEL_GAP + 13.0 + AUTO_LAYOUT_EDGE_PADDING
        );
        let band = |reach, title, gap, padding| BandMeasure {
            reach,
            title,
            gap,
            padding,
            legend: None,
            blocked: None,
        };
        let wider = band(Some(30.0), Some((40.0, 53.0)), 9.0, 11.0);
        assert_eq!(wider.required(), 30.0 + 9.0 + 13.0 + 11.0);
        let labels = band(Some(30.0), None, LABEL_GAP, 9.0);
        assert_eq!(labels.required(), 30.0 + 9.0);
        let empty = band(None, None, LABEL_GAP, AUTO_LAYOUT_EDGE_PADDING);
        assert_eq!(empty.required(), AUTO_LAYOUT_EDGE_PADDING);
        // The gap and the padding are half the font, at least 4 and 8 px.
        assert_eq!((title_gap(6.0), title_gap(16.0)), (LABEL_GAP, 8.0));
        assert_eq!((edge_padding(0.0), edge_padding(25.0)), (8.0, 12.5));
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
        // A band of exactly 55 px: the title's inner edge goes from 21 to 34.
        assert_eq!(both.placed_offset(55.0, 0.0, None), Some(13.0));
        assert_eq!(both.placed_offset(55.0, 13.0, None), None);
        assert_eq!(labels.placed_offset(110.0, -20.0, None), None, "no title");
        // A colourbar title is already placed past its labels (#259): in the
        // 55 px band it needs no offset, and an offset that would pull it back
        // over them moves it to the same place.
        let pushed = Some((LABEL_GAP, AUTO_LAYOUT_EDGE_PADDING));
        assert_eq!(both.placed_offset(55.0, 0.0, pushed), None);
        assert_eq!(both.placed_offset(55.0, -10.0, pushed), Some(0.0));
        // One whose placement keeps more room from the band's edge than the
        // band has past it is moved the rest of the way.
        let edge = Some((LABEL_GAP, AUTO_LAYOUT_EDGE_PADDING + 1.0));
        assert_eq!(both.placed_offset(55.0, 0.0, edge), Some(1.0));
    }

    // auto_layout (#44, #270).

    /// The width of the widest default label (`0.00` .. `1.00`, four
    /// glyphs).
    const DEFAULT_LABEL: f32 = 4.0 * GLYPH;
    /// The default tick-label and axis-title font sizes.
    const LABEL_SIZE: f32 = 18.0;
    const TITLE_SIZE: f32 = 22.0;
    /// What `auto_layout` keeps at the default fonts: the title gap (half
    /// the 18 px label font), the edge padding past labels (the same) and
    /// past an axis title (half the 22 px title font).
    const GAP: f32 = 9.0;
    const LABEL_PADDING: f32 = 9.0;
    const TITLE_PADDING: f32 = 11.0;

    fn with_titles(left: &str) -> Config {
        let mut cfg = crate::defaults::config();
        cfg.left_y.title_option.text.segments = crate::text::rich_segments_from_text(left);
        cfg.bottom_x.title_option.text.segments = crate::text::rich_segments_from_text("x");
        cfg
    }

    /// `out` with every field `auto_layout` writes on the four axes and the
    /// chart title set back to `input`'s.
    fn without_layout(mut out: Config, input: &Config) -> Config {
        for side in SIDES {
            let (to, from) = (axis_mut(&mut out, &side), axis_ref(input, &side));
            to.out_margin = from.out_margin;
            to.title_option.offset_y = from.title_option.offset_y;
            to.label_style.format = from.label_style.format.clone();
        }
        out.chart_title.top_margin = input.chart_title.top_margin;
        out
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

    /// The room between what the band on `side` holds and the band's outer
    /// edge: its `out_margin` less how far past its inner edge any tick
    /// label, group header or the side's own title reaches (0 when none
    /// does).
    fn edge_gap(cfg: &Config, side: Side, measure: &dyn MeasureText) -> (f32, f32) {
        let da = RectF::from_rect(&cfg.data_area().expect("a data area"));
        let axis = axis_ref(cfg, &side);
        let inner = band_inner_edge(&side, da, tick_contribution(axis));
        let mut reach = 0.0f32;
        for element in [
            Element::AxisLabel(Side::Top),
            Element::AxisLabel(Side::Bottom),
            Element::AxisLabel(Side::Left),
            Element::AxisLabel(Side::Right),
            Element::CategoryGroupLabel(Side::Top),
            Element::CategoryGroupLabel(Side::Bottom),
            Element::AxisTitle(side.clone()),
        ] {
            for r in rects(cfg, element, measure) {
                reach = reach.max(band_span(&side, inner, r).1);
            }
        }
        (axis.out_margin - reach, reach)
    }

    /// Every band of `cfg` ends the edge padding past what it holds (half
    /// the font of its title, else of the labels that run into it, else of
    /// its axis' labels when it holds the axis line, at least 8 px): to the
    /// pixel it is rounded up to, or, beside a title placed by a rounded
    /// offset, within a pixel either way.
    fn assert_padded(cfg: &Config, measure: &dyn MeasureText) {
        let label_font = SIDES
            .iter()
            .map(|side| axis_ref(cfg, side).label_style.font_size)
            .fold(0.0, f32::max);
        for side in SIDES {
            let (gap, reach) = edge_gap(cfg, side.clone(), measure);
            let titled = !rects(cfg, Element::AxisTitle(side.clone()), measure).is_empty();
            let padding = if titled {
                edge_padding(axis_ref(cfg, &side).title_option.text.font_size)
            } else if reach > 0.0 {
                edge_padding(label_font)
            } else if !rects(cfg, Element::Axis(side.clone()), measure).is_empty() {
                edge_padding(axis_ref(cfg, &side).label_style.font_size)
            } else {
                AUTO_LAYOUT_EDGE_PADDING
            };
            let low = if titled { padding - 1.0 } else { padding };
            assert!(
                gap >= low - 1e-3 && gap < padding + 1.0,
                "{side:?}: {gap} for {padding}"
            );
        }
    }

    // Every band is set to what it holds: the default document's 110 px
    // left and 80 px bottom bands shrink to their labels and the padding,
    // the top and right bands hold the overhang of the end labels, and the
    // empty chart title gets no band. The input is untouched and a second
    // call changes nothing.
    #[test]
    fn auto_layout_sets_each_band_to_what_it_holds() {
        let measure = Labels;
        for cfg in [crate::defaults::config(), with_titles("y")] {
            let input = cfg.clone();
            let out = auto_layout(&cfg, &measure);
            assert_eq!(cfg, input, "the input is not modified");
            assert_padded(&out, &measure);
            assert_eq!(out.chart_title.top_margin, 0.0);
            assert!(out.left_y.out_margin < cfg.left_y.out_margin);
            assert!(out.bottom_x.out_margin < cfg.bottom_x.out_margin);
            assert_eq!(without_layout(out.clone(), &cfg), cfg);
            assert_eq!(auto_layout(&out, &measure), out, "idempotent");
        }
        let out = auto_layout(&crate::defaults::config(), &measure);
        assert_eq!(
            out.left_y.out_margin,
            LABEL_GAP + DEFAULT_LABEL + LABEL_PADDING
        );
        assert_eq!(out.bottom_x.out_margin, LABEL_GAP + 13.0 + LABEL_PADDING);
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
    // 110 px left band. Whether the band was wider (110 px) or narrower
    // (60 px), it is set to exactly the labels, the gap, the title and the
    // padding, and the title gets the offset that puts it the gap past the
    // labels. The input is untouched and a second call changes nothing.
    #[test]
    fn auto_layout_places_a_title_beside_its_labels_in_a_tight_band() {
        // Five digits: long, but not too long for `Decimal`.
        let measure = Labels;
        // Past the tick: the gap and 56 px; the title is 13 px thick.
        let (reach, title) = (LABEL_GAP + 56.0, 13.0);
        for band in [110.0, 60.0] {
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
            assert_eq!(
                out.left_y.out_margin,
                reach + GAP + title + TITLE_PADDING,
                "{band}"
            );
            let clearance = title_clearance(&out, Side::Left, &measure);
            assert!((GAP..GAP + 1.0).contains(&clearance), "{band}: {clearance}");
            let outer = title_outer_gap(&out, Side::Left, &measure);
            assert!(outer >= TITLE_PADDING - 1.0, "{band}: {outer}");
            let offset = out.left_y.title_option.offset_y;
            assert!(offset < 0.0 && offset == offset.round(), "{band}: {offset}");
            // Everything else is the input's.
            assert_eq!(without_layout(out.clone(), &cfg), cfg);
            assert_eq!(auto_layout(&out, &measure), out, "{band}: idempotent");
        }
    }

    // A side with labels but no title needs room for the labels and the
    // padding alone, grown or shrunk to it; a side with nothing of its own
    // holds the padding and the overhang of the perpendicular labels.
    #[test]
    fn auto_layout_sizes_a_band_without_a_title_for_its_labels_alone() {
        let measure = Labels;
        let needed = LABEL_GAP + 56.0 + LABEL_PADDING;
        for band in [20.0, 110.0] {
            let mut cfg = crate::defaults::config();
            five_digit_labels(&mut cfg.left_y);
            cfg.left_y.out_margin = band;
            cfg.right_y.out_margin = 1.0;
            let out = auto_layout(&cfg, &measure);
            assert_eq!(out.left_y.out_margin, needed, "{band}");
            // The last x label runs past the right tick: the right band
            // holds it and the padding.
            assert!(out.right_y.out_margin > AUTO_LAYOUT_EDGE_PADDING);
            assert_padded(&out, &measure);
        }
    }

    // The band is set exactly, so a title that clears its labels sits the
    // gap past them and the padding inside the band's outer edge. A title the
    // host offset into its labels, or out of the band, goes back there; one
    // offset outward but still inside the band is left where it is.
    #[test]
    fn auto_layout_leaves_a_title_that_clears_and_places_one_that_does_not() {
        let measure = Labels;
        let out = auto_layout(&with_titles("y"), &measure);
        let clearance = title_clearance(&out, Side::Left, &measure);
        assert!((GAP..GAP + 1.0).contains(&clearance), "{clearance}");
        // Left: offset_y is screen x; +30 moves the title inward, into the
        // labels.
        let mut inward = out.clone();
        inward.left_y.title_option.offset_y += 30.0;
        assert!(title_clearance(&inward, Side::Left, &measure) < GAP);
        assert_eq!(auto_layout(&inward, &measure), out);
        // 5 px outward is still inside the 11 px padding: kept.
        let mut outward = out.clone();
        outward.left_y.title_option.offset_y -= 5.0;
        assert_eq!(auto_layout(&outward, &measure), outward);
        // 12 px outward leaves the band: placed again.
        outward.left_y.title_option.offset_y -= 7.0;
        assert_eq!(auto_layout(&outward, &measure), out);
    }

    // Labels printing more than five digits switch an axis to `Power`; a
    // Power, Timestamp or categorical axis and short labels keep theirs.
    #[test]
    fn auto_layout_switches_long_decimal_labels_to_power() {
        let long = Labels;
        let mut long_cfg = crate::defaults::config();
        long_labels(&mut long_cfg.left_y);
        long_labels(&mut long_cfg.bottom_x);
        let out = auto_layout(&long_cfg, &long);
        assert_eq!(out.left_y.label_style.format, LabelFormat::Power);
        assert_eq!(out.bottom_x.label_style.format, LabelFormat::Power);
        // Hidden labels are not drawn, so they are never too long.
        assert_eq!(out.top_x.label_style.format, LabelFormat::Decimal);
        assert_eq!(auto_layout(&out, &long), out);

        let short = auto_layout(&crate::defaults::config(), &Labels);
        for side in SIDES {
            assert_eq!(
                axis_ref(&short, &side).label_style.format,
                LabelFormat::Decimal
            );
        }

        let mut kept = long_cfg.clone();
        kept.left_y.label_style.format =
            LabelFormat::Timestamp(crate::defaults::timestamp_label_format());
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

    // A band that would leave no data area is not set: the 85 px left band
    // the 56 px labels and the title need does not fit a 100 px chart, even
    // with the right band at its smallest.
    #[test]
    fn auto_layout_never_sets_a_band_past_the_data_area() {
        let measure = Labels;
        let mut cfg = with_titles("Optical depth");
        five_digit_labels(&mut cfg.left_y);
        // 60 + 5 + 15 + 5 px of margins leave 15 px.
        cfg.left_y.out_margin = 60.0;
        cfg.right_y.out_margin = 15.0;
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 400,
        });
        assert!(cfg.data_area().is_ok());
        let out = auto_layout(&cfg, &measure);
        assert_eq!(out.left_y.out_margin, cfg.left_y.out_margin);
        assert!(out.data_area().is_ok());
        assert_eq!(auto_layout(&out, &measure), out);
    }

    // The colourbar's band is set from the strip outward the same way. A
    // legend box dragged past the data area over the bar is the host's
    // placement: it stays, and the right band grows to hold it inside the
    // bar (#270).
    #[test]
    fn auto_layout_fits_the_colorbar_band_and_clears_the_legend_off_it() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        let mut bar = crate::defaults::color_bar_options();
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
        // The bar's title sits half the 18 px label font past its labels,
        // and the band ends half the title's font past the title.
        let (reach, gap) = (LABEL_GAP + 56.0, 9.0);
        let padding = edge_padding(bar.axis.title_option.text.font_size);
        assert_eq!(bar.axis.out_margin, reach + gap + 13.0 + padding);
        let labels = rects(&out, Element::ColorBarLabel, &measure);
        let title = rects(&out, Element::ColorBarTitle, &measure)[0];
        let placed = labels
            .iter()
            .map(|l| title.x - (l.x + l.width))
            .fold(f32::INFINITY, f32::min);
        assert!((gap..gap + 1.0).contains(&placed), "{placed}");
        let ca = RectF::from_rect(&out.chart_area);
        let outer = ca.x + ca.width - (title.x + title.width);
        assert!(outer >= padding - 1e-3, "{outer}");
        assert_eq!(
            (out.legend.corner, out.legend.offset_x, out.legend.offset_y),
            (LegendCorner::TopRight, 120.0, 200.0)
        );
        assert_legend_clear(&out, &measure);
        assert_eq!(auto_layout(&out, &measure), out);
    }

    // A legend inside the data area under a colourbar dragged onto the plot
    // goes to the first corner, with zero offsets, where it clears the bar.
    #[test]
    fn auto_layout_clears_an_inside_legend_off_a_dragged_colorbar() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        let mut bar = crate::defaults::color_bar_options();
        bar.length_frac = 1.0;
        cfg.colorbar = Some(bar);
        cfg.legend.content.segments = crate::text::rich_segments_from_text("series");
        // Laid out without the legend, then the bar is dragged over the
        // middle of the top-right legend box; offsets do not change bands.
        let laid = auto_layout(&cfg, &measure);
        let mut dragged = laid.clone();
        dragged.legend.visible = true;
        let legend = rects(&dragged, Element::Legend, &measure)[0];
        let strip = rects(&laid, Element::ColorBar, &measure)[0];
        if let Some(bar) = dragged.colorbar.as_mut() {
            bar.offset_x = (legend.x + legend.width * 0.5 - (strip.x + strip.width * 0.5)).round();
        }
        let out = auto_layout(&dragged, &measure);
        assert_eq!(
            (out.legend.corner, out.legend.offset_x, out.legend.offset_y),
            (LegendCorner::TopLeft, 0.0, 0.0)
        );
        assert_legend_clear(&out, &measure);
        assert_eq!(auto_layout(&out, &measure), out);
    }

    /// The legend of `cfg` overlaps no colourbar part, tick label or axis
    /// title, and lies inside the chart area with at least the edge padding
    /// of its font to the chart edge, or to the colourbar band, on every
    /// side it crosses past the data area.
    fn assert_legend_clear(cfg: &Config, measure: &dyn MeasureText) {
        let legend = rects(cfg, Element::Legend, measure)[0];
        let mut others = Vec::new();
        for element in [
            Element::ColorBar,
            Element::ColorBarLabel,
            Element::ColorBarTitle,
        ] {
            others.extend(rects(cfg, element, measure));
        }
        for side in SIDES {
            others.extend(rects(cfg, Element::AxisLabel(side.clone()), measure));
            others.extend(rects(cfg, Element::AxisTitle(side), measure));
        }
        for r in others {
            assert!(!rects_overlap(legend, r), "{legend:?} {r:?}");
        }
        let da = RectF::from_rect(&cfg.data_area().expect("a data area"));
        let ca = RectF::from_rect(&cfg.chart_area);
        let padding = edge_padding(cfg.legend.content.font_size);
        for side in SIDES {
            let past = band_span(&side, band_inner_edge(&side, da, 0.0), legend).1 > 0.0;
            let inset = cfg.colorbar_band(&side);
            let room = match side {
                Side::Left => legend.x - (ca.x + inset),
                Side::Right => ca.x + ca.width - inset - (legend.x + legend.width),
                Side::Top => legend.y - (ca.y + cfg.chart_title.top_margin + inset),
                Side::Bottom => ca.y + ca.height - inset - (legend.y + legend.height),
            };
            if past {
                assert!(room >= padding - 1e-3, "{side:?}: {room} for {padding}");
            }
        }
    }

    /// `laid` with the legend of `cfg`.
    fn cfg_laid(laid: &Config, cfg: &Config) -> Config {
        let mut out = laid.clone();
        out.legend = cfg.legend.clone();
        out
    }

    /// The default document with axis titles `y` and `x`, and a visible
    /// legend of `glyphs` glyphs at `corner` with the offsets `offset`.
    fn with_legend(glyphs: usize, corner: LegendCorner, offset: (f32, f32)) -> Config {
        let mut cfg = with_titles("y");
        cfg.legend.visible = true;
        cfg.legend.content.segments = crate::text::rich_segments_from_text(&"s".repeat(glyphs));
        cfg.legend.corner = corner;
        (cfg.legend.offset_x, cfg.legend.offset_y) = offset;
        cfg
    }

    // A legend the host moved below the plot keeps its corner and offsets;
    // the bottom band grows to hold it and the edge padding, so it is not
    // clipped and covers neither the x labels nor the x title. A short one
    // beside the title leaves the title beside its labels; a wide one under
    // the title pushes the title past it, the title gap from the legend.
    #[test]
    fn auto_layout_holds_a_legend_below_the_plot() {
        let measure = Labels;
        for (glyphs, offset_x, pushed) in [(6, 0.0, false), (60, 150.0, true)] {
            let cfg = with_legend(glyphs, LegendCorner::BottomLeft, (offset_x, 70.0));
            let out = auto_layout(&cfg, &measure);
            assert_eq!(out.legend, cfg.legend, "the legend is not moved");
            assert!(
                out.bottom_x.out_margin
                    > auto_layout(&with_titles("y"), &measure).bottom_x.out_margin
            );
            assert_legend_clear(&out, &measure);
            let legend = rects(&out, Element::Legend, &measure)[0];
            let title = rects(&out, Element::AxisTitle(Side::Bottom), &measure)[0];
            let past = title.y - (legend.y + legend.height);
            if pushed {
                assert!((GAP..GAP + 1.0).contains(&past), "{past}");
            } else {
                // Above the legend, clear of the labels.
                assert!(past < 0.0, "{past}");
                let clearance = title_clearance(&out, Side::Bottom, &measure);
                assert!(clearance >= GAP - 1e-3, "{clearance}");
            }
            assert_eq!(auto_layout(&out, &measure), out, "idempotent");
        }
    }

    // The same on the left: a legend moved left of the plot level with the
    // y title widens the left band to hold it and its padding, and the
    // title goes past it; the tick labels stay with the axis.
    #[test]
    fn auto_layout_holds_a_legend_left_of_the_plot() {
        let measure = Labels;
        let plain = auto_layout(&with_titles("y"), &measure);
        let da = plain.data_area().expect("a data area");
        // Level with the centred title, its right edge 4 px left of the
        // labels: where the title beside them would be.
        let mut cfg = with_legend(6, LegendCorner::TopLeft, (0.0, 0.0));
        let at_corner = rects(&cfg_laid(&plain, &cfg), Element::Legend, &measure)[0];
        let labels_left = rects(&plain, Element::AxisLabel(Side::Left), &measure)
            .iter()
            .map(|r| r.x)
            .fold(f32::INFINITY, f32::min);
        cfg.legend.offset_x = (labels_left - 4.0 - (at_corner.x + at_corner.width)).floor();
        cfg.legend.offset_y =
            (da.height as f32 * 0.5 - LEGEND_INSET - at_corner.height * 0.5).round();
        let out = auto_layout(&cfg, &measure);
        assert_eq!(out.legend, cfg.legend, "the legend is not moved");
        assert!(out.left_y.out_margin > plain.left_y.out_margin);
        assert_legend_clear(&out, &measure);
        let legend = rects(&out, Element::Legend, &measure)[0];
        let title = rects(&out, Element::AxisTitle(Side::Left), &measure)[0];
        let past = legend.x - (title.x + title.width);
        assert!((GAP..GAP + 1.0).contains(&past), "{past}");
        let labels = rects(&out, Element::AxisLabel(Side::Left), &measure);
        let plain_labels = rects(&plain, Element::AxisLabel(Side::Left), &measure);
        let out_da = out.data_area().expect("a data area");
        assert_eq!(
            labels[0].x + labels[0].width - out_da.x as f32,
            plain_labels[0].x + plain_labels[0].width - da.x as f32,
            "the labels stay with the axis"
        );
        assert_eq!(auto_layout(&out, &measure), out, "idempotent");
    }

    // A legend inside the data area changes no band: the layout is the one
    // of the same document without a legend.
    #[test]
    fn auto_layout_leaves_the_bands_of_an_inside_legend() {
        let measure = Labels;
        for corner in [
            LegendCorner::TopLeft,
            LegendCorner::TopRight,
            LegendCorner::BottomLeft,
            LegendCorner::BottomRight,
        ] {
            let cfg = with_legend(20, corner, (0.0, 0.0));
            let mut hidden = cfg.clone();
            hidden.legend.visible = false;
            let mut out = auto_layout(&cfg, &measure);
            assert_eq!(out.legend, cfg.legend);
            out.legend.visible = false;
            assert_eq!(out, auto_layout(&hidden, &measure));
        }
    }

    // An overlay's labels and title set the shared right band through the
    // overlay's own axis, and a subplot is laid out as a chart of its own.
    #[test]
    fn auto_layout_fits_overlay_and_subplot_bands() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        let mut overlay = crate::defaults::overlay_layer();
        overlay.y_axis.title_option.visible = true;
        overlay.y_axis.title_option.text.segments = crate::text::rich_segments_from_text("p");
        cfg.overlays.push(overlay);
        let mut sub = crate::defaults::subplot();
        sub.config = with_titles("Optical depth");
        sub.config.chart_area = ChartArea(Rect {
            x: 1000,
            y: 0,
            width: 600,
            height: 400,
        });
        cfg.subplots.push(sub);

        let out = auto_layout(&cfg, &measure);
        // The overlay's band: its labels, the gap, its title and the padding.
        let axis = &out.overlays[0].y_axis;
        let gap = title_gap(axis.label_style.font_size);
        let padding = edge_padding(axis.title_option.text.font_size);
        assert_eq!(
            axis.out_margin,
            LABEL_GAP + DEFAULT_LABEL + gap + 13.0 + padding
        );
        let panel = out.overlay_panel(0).unwrap();
        assert!(title_clearance(&panel, Side::Right, &measure) >= gap - 1e-3);
        let outer = title_outer_gap(&panel, Side::Right, &measure);
        assert!(outer >= padding - 1.0, "{outer}");
        let chart = out.subplot_config(0).unwrap();
        assert!(title_clearance(&chart, Side::Left, &measure) >= GAP - 1e-3);
        assert_padded(&chart, &measure);
        assert_eq!(
            out.subplots[0].config.chart_area,
            cfg.subplots[0].config.chart_area
        );
        assert_eq!(auto_layout(&out, &measure), out);
    }

    /// An overlay with an X axis of its own gets its top band fitted like
    /// its Y axis' right band, and its title clears its labels there; with
    /// its Y linked it gets no right band of its own. A second pass changes
    /// nothing.
    #[test]
    fn auto_layout_fits_an_overlay_x_band_on_the_top() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        cfg.chart_title.visible = false;
        let mut overlay = crate::defaults::overlay_layer();
        overlay.link_x = false;
        overlay.link_y = true;
        overlay.x_axis.title_option.visible = true;
        overlay.x_axis.title_option.text.segments = crate::text::rich_segments_from_text("t");
        overlay.y_axis.out_margin = 300.0;
        cfg.overlays.push(overlay);
        let out = auto_layout(&cfg, &measure);
        let axis = &out.overlays[0].x_axis;
        let gap = title_gap(axis.label_style.font_size);
        assert!(axis.out_margin > LABEL_GAP + gap, "{}", axis.out_margin);
        let panel = out.overlay_panel(0).unwrap();
        assert!(title_clearance(&panel, Side::Top, &measure) >= gap - 1e-3);
        // The linked Y axis is not drawn: its stored band does not count.
        assert!(out.margins().right < 300.0);
        assert_eq!(out.overlays[0].y_axis.out_margin, 300.0);
        assert_eq!(auto_layout(&out, &measure), out);
    }

    /// A cell of a grid at `(x, y)`, 480 x 420 like the `mixed_axes`
    /// example's, titled `title` (none when empty) with a left title.
    fn grid_cell(x: u32, y: u32, title: &str) -> Config {
        let mut cfg = with_titles("y");
        cfg.chart_area = ChartArea(Rect {
            x,
            y,
            width: 480,
            height: 420,
        });
        cfg.chart_title.text.segments = crate::text::rich_segments_from_text(title);
        cfg
    }

    /// The plot of every cell of `doc`: its own chart's, then each
    /// subplot's.
    fn plots(doc: &Config) -> Vec<Rect> {
        let mut out = vec![doc.data_area().expect("a data area").0];
        for index in 0..doc.subplots.len() {
            let chart = doc.subplot_config(index).expect("a subplot");
            out.push(chart.data_area().expect("a data area").0);
        }
        out
    }

    // A 2 x 2 grid shaped like the `mixed_axes` example, and a lone cell
    // beside it. The upper-right cell has no chart title; the lower-left one
    // five-digit labels and no x title; the lower-right one an overlay with
    // a titled axis on the right. Fitted alone, the cells' plots differ;
    // aligned, each column shares its left and right plot edges (the
    // overlay's band included) and each row its top and bottom edges, a
    // title in a widened band sits the gap past its labels, the lone cell
    // keeps its own fit and a second call changes nothing.
    #[test]
    fn auto_layout_lines_up_the_plots_of_a_grid() {
        let measure = Labels;
        let mut doc = grid_cell(0, 0, "Temperature");
        let untitled = grid_cell(480, 0, "");
        let mut wide = grid_cell(0, 420, "Humidity");
        five_digit_labels(&mut wide.left_y);
        wide.bottom_x.title_option.text.segments.clear();
        let mut twin = grid_cell(480, 420, "Daytime");
        let mut overlay = crate::defaults::overlay_layer();
        overlay.y_axis.title_option.visible = true;
        overlay.y_axis.title_option.text.segments = crate::text::rich_segments_from_text("%");
        twin.overlays.push(overlay);
        let mut lone = grid_cell(960, 100, "Lone");
        lone.chart_area.0.height = 300;
        for config in [untitled, wide, twin, lone] {
            let mut sub = crate::defaults::subplot();
            sub.config = config;
            doc.subplots.push(sub);
        }

        // Each cell fitted alone, as before the alignment.
        let mut alone = vec![auto_layout(&doc_without_subplots(&doc), &measure)];
        alone.extend(
            doc.subplots
                .iter()
                .map(|s| auto_layout(&s.config, &measure)),
        );
        let fitted: Vec<Rect> = alone.iter().map(|c| c.data_area().unwrap().0).collect();
        assert_ne!(
            fitted[0].x, fitted[2].x,
            "the left edges differ when fitted"
        );
        assert_ne!(
            fitted[1].x + fitted[1].width,
            fitted[3].x + fitted[3].width,
            "the right edges differ when fitted"
        );
        assert_ne!(fitted[0].y, fitted[1].y, "the top edges differ when fitted");
        assert_ne!(
            fitted[2].y + fitted[2].height,
            fitted[3].y + fitted[3].height,
            "the bottom edges differ when fitted"
        );

        let out = auto_layout(&doc, &measure);
        let plot = plots(&out);
        for (a, b) in [(0, 2), (1, 3)] {
            assert_eq!(plot[a].x, plot[b].x, "column {a} {b}: left edge");
            assert_eq!(plot[a].width, plot[b].width, "column {a} {b}: right edge");
        }
        for (a, b) in [(0, 1), (2, 3)] {
            assert_eq!(plot[a].y, plot[b].y, "row {a} {b}: top edge");
            assert_eq!(plot[a].height, plot[b].height, "row {a} {b}: bottom edge");
        }
        // Every plot of the grid is the same size.
        for p in &plot[1..4] {
            assert_eq!((p.width, p.height), (plot[0].width, plot[0].height));
        }
        // The titled cells of a row share the chart title band; an
        // untitled one gets none and a deeper top axis band instead.
        assert!(out.chart_title.top_margin > 0.0);
        assert_eq!(out.subplots[0].config.chart_title.top_margin, 0.0);
        assert!(out.subplots[0].config.top_x.out_margin > out.top_x.out_margin);
        assert_eq!(
            out.subplots[1].config.chart_title.top_margin,
            out.subplots[2].config.chart_title.top_margin
        );
        // The lone cell keeps its own fit.
        assert_eq!(plot[4], fitted[4]);
        assert_eq!(out.subplots[3].config, alone[4]);
        // The parent's left band, widened to the lower-left cell's, keeps
        // its title the gap past its labels; the overlay's title clears its
        // labels in the right band of its group.
        assert!(out.left_y.out_margin > alone[0].left_y.out_margin);
        let clearance = title_clearance(&out, Side::Left, &measure);
        assert!((GAP..GAP + 1.0).contains(&clearance), "{clearance}");
        let twin = out.subplot_config(2).unwrap().overlay_panel(0).unwrap();
        let clearance = title_clearance(&twin, Side::Right, &measure);
        assert!(clearance >= GAP - 1e-3, "{clearance}");
        assert_eq!(auto_layout(&out, &measure), out, "idempotent");
    }

    /// `doc` without its subplots: its own chart alone.
    fn doc_without_subplots(doc: &Config) -> Config {
        let mut alone = doc.clone();
        alone.subplots.clear();
        alone
    }

    // The chart title's band is its measured height (13 px) and 0.3 em of
    // its font above and below, at least the edge padding, rounded up; the
    // title is centred in it, so the gap above it is at least the padding.
    // A hidden or empty title gets no band, which validation accepts, and
    // the top axis band then holds the padding.
    #[test]
    fn auto_layout_pads_the_chart_title_and_drops_a_missing_one() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        cfg.chart_title.text.segments = crate::text::rich_segments_from_text("Spectrum");
        let out = auto_layout(&cfg, &measure);
        // 28 px font: 8.4 px each side.
        assert_eq!(out.chart_title.top_margin, (13.0f32 + 2.0 * 8.4).ceil());
        let title = rects(&out, Element::ChartTitle, &measure)[0];
        let ca = RectF::from_rect(&out.chart_area);
        assert!(title.y - ca.y >= 8.4, "{title:?}");
        let below = ca.y + out.chart_title.top_margin - (title.y + title.height);
        assert!(below >= 8.4, "{below}");
        assert_padded(&out, &measure);
        // A small font keeps the edge padding; a large one pads by 0.3 em.
        cfg.chart_title.text.font_size = 12.0;
        assert_eq!(
            auto_layout(&cfg, &measure).chart_title.top_margin,
            13.0 + 2.0 * AUTO_LAYOUT_EDGE_PADDING
        );
        cfg.chart_title.text.font_size = 60.0;
        assert_eq!(
            auto_layout(&cfg, &measure).chart_title.top_margin,
            13.0 + 2.0 * 18.0
        );

        let mut hidden = cfg.clone();
        hidden.chart_title.visible = false;
        let mut empty = cfg.clone();
        empty.chart_title.text.segments.clear();
        for cfg in [hidden, empty] {
            let out = auto_layout(&cfg, &measure);
            assert_eq!(out.chart_title.top_margin, 0.0);
            assert!(out.validate_chart().is_ok());
            assert_padded(&out, &measure);
            let top = rects(&out, Element::AxisLabel(Side::Left), &measure)
                .iter()
                .map(|r| r.y)
                .fold(f32::INFINITY, f32::min);
            assert!(top - out.chart_area.y as f32 >= LABEL_PADDING);
        }
    }

    // The ends of the perpendicular axes' labels that run past the plot are
    // held by the band on that side: the top y label by the top band, the
    // last x label by the right band, each the padding inside the chart edge.
    #[test]
    fn auto_layout_holds_the_overhang_of_the_perpendicular_labels() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        // No tick on the empty sides, so the whole overhang is in the band.
        cfg.top_x.major_tick_length = 0.0;
        cfg.right_y.major_tick_length = 0.0;
        let out = auto_layout(&cfg, &measure);
        let ca = RectF::from_rect(&out.chart_area);
        let top = rects(&out, Element::AxisLabel(Side::Left), &measure)
            .iter()
            .map(|r| r.y)
            .fold(f32::INFINITY, f32::min);
        let right = rects(&out, Element::AxisLabel(Side::Bottom), &measure)
            .iter()
            .map(|r| r.x + r.width)
            .fold(f32::NEG_INFINITY, f32::max);
        let da = RectF::from_rect(&out.data_area().unwrap());
        assert!(
            top < da.y && right > da.x + da.width,
            "they run past the plot"
        );
        for gap in [top - ca.y, ca.x + ca.width - right] {
            assert!((LABEL_PADDING..LABEL_PADDING + 1.0).contains(&gap), "{gap}");
        }
        assert_eq!(
            out.right_y.out_margin,
            (DEFAULT_LABEL * 0.5 + LABEL_PADDING).ceil()
        );
        assert_padded(&out, &measure);
    }

    // The group headers of a categorical x axis sit past its labels in the
    // bottom band, which holds them: the x title goes the gap past the
    // headers, not over them, and the band is the padding past the title.
    #[test]
    fn auto_layout_holds_the_category_group_headers() {
        use crate::config::{Category, CategoryGroup, CategoryGroups};
        let measure = Labels;
        let group = |label: &str, first: &str, last: &str| CategoryGroup {
            label: crate::text::RichText::plain(label, Color::BLACK, 12.0, ""),
            first: first.into(),
            last: last.into(),
            bracket: true,
        };
        let mut cfg = with_titles("y");
        cfg.bottom_x.categories = Some(
            ["a", "b", "c", "d"]
                .into_iter()
                .map(|c| Category::new(c, c))
                .collect(),
        );
        let plain = auto_layout(&cfg, &measure);
        cfg.bottom_x.category_groups = Some(CategoryGroups {
            groups: vec![group("Left", "a", "b"), group("Right", "c", "d")],
            offset: [0.0; 2],
        });
        let out = auto_layout(&cfg, &measure);
        assert!(out.bottom_x.out_margin > plain.bottom_x.out_margin);
        let headers = rects(&out, Element::CategoryGroupLabel(Side::Bottom), &measure);
        assert!(!headers.is_empty());
        let header_bottom = headers
            .iter()
            .map(|r| r.y + r.height)
            .fold(f32::NEG_INFINITY, f32::max);
        let title = rects(&out, Element::AxisTitle(Side::Bottom), &measure)[0];
        assert!(title.y >= header_bottom + GAP - 1e-3, "{title:?}");
        assert_padded(&out, &measure);
        assert_eq!(auto_layout(&out, &measure), out);
    }

    // The gap between an axis title and its labels and the padding at a
    // band's outer edge are half the font (#270): 12 px text keeps the least
    // gap (6 px, over 4) and padding (8 px), 40 px text 20 px of each. The
    // top band holds the frame and pads like the labels.
    #[test]
    fn auto_layout_scales_the_title_gap_and_the_edge_padding_with_the_font() {
        let measure = Labels;
        let mut paddings = Vec::new();
        for size in [12.0f32, 18.0, 40.0] {
            let mut cfg = with_titles("y");
            for side in SIDES {
                let axis = axis_mut(&mut cfg, &side);
                axis.label_style.font_size = size;
                axis.title_option.text.font_size = size;
            }
            let out = auto_layout(&cfg, &measure);
            let gap = LABEL_GAP.max(AUTO_LAYOUT_TITLE_GAP_EMS * size);
            let padding = AUTO_LAYOUT_EDGE_PADDING.max(AUTO_LAYOUT_EDGE_PADDING_EMS * size);
            assert_eq!((gap, padding), (title_gap(size), edge_padding(size)));
            for side in [Side::Left, Side::Bottom] {
                let clearance = title_clearance(&out, side.clone(), &measure);
                assert!(
                    (gap..gap + 1.0).contains(&clearance),
                    "{size} {side:?}: {clearance}"
                );
                let outer = title_outer_gap(&out, side.clone(), &measure);
                assert!(
                    (padding - 1.0..padding + 1.0).contains(&outer),
                    "{size} {side:?}: {outer}"
                );
            }
            let (top, _) = edge_gap(&out, Side::Top, &measure);
            assert!((padding..padding + 1.0).contains(&top), "{size}: {top}");
            assert_padded(&out, &measure);
            assert_eq!(auto_layout(&out, &measure), out, "{size}: idempotent");
            paddings.push(top);
        }
        assert!(paddings[0] < paddings[1] && paddings[1] < paddings[2]);
    }

    // A chart with a hidden frame (no lines, ticks or labels) holds nothing
    // in its bands: each is the edge padding, and with no chart title the
    // top band pads the top.
    #[test]
    fn auto_layout_pads_a_hidden_frame_on_every_side() {
        let measure = Labels;
        let mut cfg = crate::defaults::config();
        cfg.apply_axis_preset(crate::preset::AxisPreset::Hidden);
        let out = auto_layout(&cfg, &measure);
        for side in SIDES {
            assert_eq!(
                axis_ref(&out, &side).out_margin,
                AUTO_LAYOUT_EDGE_PADDING,
                "{side:?}"
            );
        }
        assert_eq!(out.chart_title.top_margin, 0.0);
        assert_eq!(auto_layout(&out, &measure), out);
    }

    // A ternary chart keeps the room for its edge labels and titles inside
    // the data area, around the triangle: the cartesian bands hold none of
    // it and shrink to the edge padding.
    #[test]
    fn auto_layout_shrinks_the_bands_of_a_ternary_chart() {
        let measure = Labels;
        let mut cfg = with_titles("B");
        cfg.right_y.title_option.text.segments = crate::text::rich_segments_from_text("C");
        cfg.set_ternary(Some(crate::defaults::ternary_options()));
        let out = auto_layout(&cfg, &measure);
        for side in SIDES {
            assert_eq!(
                axis_ref(&out, &side).out_margin,
                AUTO_LAYOUT_EDGE_PADDING,
                "{side:?}"
            );
            // The titles stay where the triangle places them.
            assert_eq!(
                axis_ref(&out, &side).title_option.offset_y,
                axis_ref(&cfg, &side).title_option.offset_y
            );
        }
        assert!(out.ternary_geometry().unwrap().side > cfg.ternary_geometry().unwrap().side);
        assert_eq!(auto_layout(&out, &measure), out);
    }
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
