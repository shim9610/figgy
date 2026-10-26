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
    if config.polar.is_some() {
        return None;
    }
    let (axis, perpendicular) = match side {
        Side::Top => (&config.top_x, &config.left_y),
        Side::Bottom => (&config.bottom_x, &config.left_y),
        Side::Left => (&config.left_y, &config.bottom_x),
        Side::Right => (&config.right_y, &config.bottom_x),
    };
    let value = axis.cross_at?;
    let t = match perpendicular.scale {
        _ if perpendicular.categories.is_some() => {
            let (min, max) = perpendicular.data_range();
            span_fraction(value, min, max, false)
        }
        AxisScale::Linear => span_fraction(value, perpendicular.min, perpendicular.max, false),
        AxisScale::Logarithmic => {
            let (min, max) = guarded_log_range(perpendicular.min, perpendicular.max);
            span_fraction(value, min, max, true)
        }
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
pub fn axis_fraction(value: f64, axis: &AxisOptions) -> f32 {
    let t = match axis.scale {
        // A categorical axis is linear over its category range.
        _ if axis.categories.is_some() => {
            let (min, max) = axis.data_range();
            span_fraction(value, min, max, false)
        }
        AxisScale::Linear => span_fraction(value, axis.min, axis.max, false),
        AxisScale::Logarithmic => {
            let (min, max) = guarded_log_range(axis.min, axis.max);
            span_fraction(value, min, max, true)
        }
    }
    .unwrap_or(0.0);
    let t = if axis.inverted { 1.0 - t } else { t };
    t as f32
}

/// Where `value` falls between `min` and `max`, linearly or by decades: the
/// formula under [`axis_fraction`] and the annotation space. `None` when the
/// value has no position: a non-finite value, an empty or non-finite span,
/// or a non-positive value or bound on a logarithmic span.
pub fn span_fraction(value: f64, min: f64, max: f64, logarithmic: bool) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let (value, min, max) = if logarithmic {
        if value <= 0.0 || min <= 0.0 || max <= 0.0 {
            return None;
        }
        (value.log10(), min.log10(), max.log10())
    } else {
        (value, min, max)
    };
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
    /// Axis-aligned screen rectangle of the placed text envelope.
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
            _ => local,
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
pub fn colorbar_title_placement(
    side: Side,
    strip: &RectF,
    axis: &AxisOptions,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    let tick = axis.major_tick_length.max(0.0);
    let out_margin = axis.out_margin.max(0.0);
    match side {
        Side::Top | Side::Bottom => {
            let band_top = if matches!(side, Side::Top) {
                strip.y - tick - out_margin
            } else {
                strip.y + strip.height + tick
            };
            let origin = (
                strip.x + strip.width * 0.5 - extents.width * 0.5 + offset.0,
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
                    strip.x - tick - out_margin * 0.5
                } else {
                    strip.x + strip.width + tick + out_margin * 0.5
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

    /// The band the labels and a centred title need: the title's thickness
    /// plus twice the labels' reach and the clearance, so the centred title
    /// clears the labels by the clearance; the reach and the clearance alone
    /// without a title. `None` for a band with neither.
    fn required(&self) -> Option<f32> {
        let reach = self.reach.unwrap_or(0.0);
        match self.title {
            Some((near, far)) => Some(far - near + 2.0 * (reach + AUTO_LAYOUT_CLEARANCE)),
            None => self.reach.map(|r| r + AUTO_LAYOUT_CLEARANCE),
        }
    }

    /// The outward title offset, within the band `band` wide, at which the
    /// title clears the labels and stays inside the band by the clearance,
    /// for a title now offset outward by `offset`; `None` when `offset`
    /// already does, or no offset can. A new offset is a whole pixel on the
    /// safe side.
    fn clamped_offset(&self, band: f32, offset: f32) -> Option<f32> {
        let (near, far) = self.title?;
        let thickness = far - near;
        let reach = self.reach.unwrap_or(0.0);
        let low = reach + AUTO_LAYOUT_CLEARANCE + thickness * 0.5 - band * 0.5;
        let high = band * 0.5 - thickness * 0.5 - AUTO_LAYOUT_CLEARANCE;
        if !(low.ceil() <= high.floor()) {
            return None;
        }
        if offset < low {
            Some(low.ceil())
        } else if offset > high {
            Some(high.floor())
        } else {
            None
        }
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
/// ([`MeasureText::emit`] for tick labels, which places the labels the
/// renderer draws after its pruning), on the panels the renderer draws:
///
/// - **Label format.** An axis whose labels are `Decimal` or `Plain` and not
///   categorical switches to `LabelFormat::Power` when a drawn label prints
///   more than five digits (`0.00300`) or is wider than five ems of its font
///   size. Other formats are left alone, and a format never switches back.
/// - **`out_margin`.** The band of each side grows so its centred title
///   clears the labels: the title's thickness plus twice the labels' reach
///   past the tick and [`LABEL_GAP`], or the reach and the gap when the side
///   has no title. A band that already holds them is kept as it is, a side
///   without labels or a title keeps its margin, and a band is never shrunk.
///   A growth that would leave no data area is not made.
/// - **Title offsets.** A title offset of zero stays zero. A title already
///   offset across its band (`title_option.offset_y`) is moved, by whole
///   pixels, only as far as it takes to clear its labels and stay inside its
///   band by [`LABEL_GAP`].
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
    let labels = if panel.polar.is_some() {
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
/// leaves no data area; then move a title that is offset across the band
/// only as far as it must. Returns whether anything changed.
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
        let offset = axis.title_option.offset_y;
        if offset != 0.0
            && let Some(outward) = band.clamped_offset(width, sign * offset)
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
        config.polar = Some(crate::polar::PolarOptions::default());
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
                colorbar_title_placement(side.clone(), &strip, &axis, (0.0, 0.0), m).rect(m);
            let after = colorbar_title_placement(side, &moved, &axis, (0.0, 0.0), m).rect(m);
            assert_eq!(after, before.translated(13.0, -9.0));
        }

        let right = colorbar_title_placement(Side::Right, &strip, &axis, (0.0, 0.0), m).rect(m);
        assert_eq!(right.y + right.height * 0.5, strip.y + strip.height * 0.5);

        let top = colorbar_title_placement(Side::Top, &strip, &axis, (0.0, 0.0), m).rect(m);
        assert_eq!(top.x + top.width * 0.5, strip.x + strip.width * 0.5);
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

    /// A measure that lays out three tick labels per visible axis, at both
    /// ends and the middle of the data-area edge, as the renderer places
    /// them (`label_origin`, offsets and `line_offset` included), and the
    /// colourbar's three labels along the strip. A `Decimal` or `Plain`
    /// label prints `decimal`, any other format `3×10`.
    struct Labels {
        decimal: &'static str,
    }

    impl Labels {
        fn text(&self, axis: &AxisOptions) -> &'static str {
            match axis.label_style.format {
                LabelFormat::Decimal | LabelFormat::Plain(_) => self.decimal,
                _ => "3×10",
            }
        }

        fn place(&self, axis: &AxisOptions, side: Side, at: [(f32, f32); 3], sink: &mut dyn Sink) {
            if !(axis.label_style.visible && axis.label_style.label_visible) {
                return;
            }
            let text = self.text(axis);
            let extents = TextExtents {
                width: text.chars().count() as f32 * GLYPH,
                ascent: 10.0,
                descent: 3.0,
            };
            let o = axis_offset(side.clone(), axis.line_offset);
            for p in at {
                let origin = label_origin(
                    side.clone(),
                    (p.0 + o.0, p.1 + o.1),
                    axis.major_tick_length,
                    (
                        axis.label_style.label_offset_x,
                        axis.label_style.label_offset_y,
                    ),
                    extents,
                );
                sink.text(
                    Slot::Above,
                    &PlacedText {
                        text: TextRef::Plain {
                            text,
                            family: "",
                            size: axis.label_style.font_size,
                            color: Color::BLACK,
                        },
                        origin,
                        extents,
                        rotation: None,
                    },
                );
            }
        }
    }

    fn ends_and_middle(side: &Side, rect: &RectF) -> [(f32, f32); 3] {
        [0.0, 0.5, 1.0].map(|t| point_on_rect_side(t, side, rect))
    }

    impl MeasureText for Labels {
        fn measure_rich(&self, rt: &crate::text::RichText) -> TextExtents {
            TextExtents {
                width: rt.segments.len() as f32 * GLYPH,
                ascent: 10.0,
                descent: 3.0,
            }
        }

        fn emit(&self, cfg: &Config, element: &Element, sink: &mut dyn Sink) -> bool {
            let Ok(da) = cfg.data_area() else {
                return true;
            };
            let plot = RectF::from_rect(&da);
            match element {
                Element::AxisLabel(side) => {
                    let axis = super::super::axis_ref(cfg, side);
                    self.place(axis, side.clone(), ends_and_middle(side, &plot), sink);
                }
                Element::ColorBarLabel => {
                    if let Some((bar, strip)) = visible_colorbar_rect(cfg) {
                        let at = ends_and_middle(&bar.side, &strip);
                        self.place(&bar.axis, bar.side.clone(), at, sink);
                    }
                }
                _ => {}
            }
            true
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
        let cfg = default_config();
        let measure = Labels { decimal: "0.00300" };
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
        let mut titled = default_config();
        titled.left_y.title_option.text.segments = crate::text::rich_segments_from_text("y");
        let title = Probe::of(&titled, &Element::AxisTitle(Side::Left), &measure);
        let inner = band_inner_edge(&Side::Left, RectF::from_rect(&da), 5.0);
        let band = BandMeasure::of(&Side::Left, inner, &probe, &title);
        assert_eq!(band.reach, Some(LABEL_GAP + 56.0));
        assert_eq!(band.title, Some((55.0 - 6.5, 55.0 + 6.5)));
    }

    // A centred title clears labels reaching `R` in a band of
    // `t + 2 (R + gap)`; an offset title is clamped to the whole pixels at
    // which it clears them and stays inside the band.
    #[test]
    fn a_band_needs_room_for_its_labels_and_a_centred_title() {
        let both = BandMeasure {
            reach: Some(30.0),
            title: Some((40.0, 53.0)),
        };
        assert_eq!(both.required(), Some(13.0 + 2.0 * (30.0 + LABEL_GAP)));
        let labels = BandMeasure {
            reach: Some(30.0),
            title: None,
        };
        assert_eq!(labels.required(), Some(30.0 + LABEL_GAP));
        assert_eq!(BandMeasure::default().required(), None);
        // In a 110 px band the title may move from 30 + 4 + 6.5 - 55 =
        // -14.5 (inward) to 55 - 6.5 - 4 = 44.5 (outward).
        assert_eq!(both.clamped_offset(110.0, 0.0), None);
        assert_eq!(both.clamped_offset(110.0, -14.5), None);
        assert_eq!(both.clamped_offset(110.0, -20.0), Some(-14.0));
        assert_eq!(both.clamped_offset(110.0, 50.0), Some(44.0));
        assert_eq!(labels.clamped_offset(110.0, -20.0), None, "no title");
        assert_eq!(both.clamped_offset(40.0, -20.0), None, "no room at all");
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
        let measure = Labels { decimal: "0.5" };
        let cfg = with_titles("y");
        assert!(title_clearance(&cfg, Side::Left, &measure) >= LABEL_GAP);
        assert_eq!(auto_layout(&cfg, &measure), cfg);
        assert_eq!(auto_layout(&default_config(), &measure), default_config());
    }

    // `-1.2345` labels (56 px) reach into the centred title of the default
    // 110 px left band. The band grows to the title's thickness plus twice the
    // labels' reach and the gap, after which the centred title clears them by
    // the gap; the input is untouched and a second call changes nothing.
    #[test]
    fn auto_layout_grows_a_band_until_its_centred_title_clears_the_labels() {
        // Five digits: long, but not too long for `Decimal`.
        let measure = Labels { decimal: "-1.2345" };
        let cfg = with_titles("Optical depth");
        let input = cfg.clone();
        assert!(
            title_clearance(&cfg, Side::Left, &measure) < 0.0,
            "overlaps as laid out"
        );

        let out = auto_layout(&cfg, &measure);
        assert_eq!(cfg, input, "the input is not modified");
        // Past the tick: the gap and 56 px.
        let reach = LABEL_GAP + 56.0;
        let title = 13.0;
        assert_eq!(
            out.left_y.out_margin,
            (title + 2.0 * (reach + LABEL_GAP)).ceil()
        );
        assert!(title_clearance(&out, Side::Left, &measure) >= LABEL_GAP - 1e-3);
        assert_eq!(
            out.left_y.title_option.offset_y, 0.0,
            "a centred title stays centred"
        );
        // Everything else is the input's.
        let mut back = out.clone();
        back.left_y.out_margin = cfg.left_y.out_margin;
        back.bottom_x.out_margin = cfg.bottom_x.out_margin;
        assert_eq!(back, cfg);
        assert_eq!(auto_layout(&out, &measure), out, "idempotent");
    }

    // A side with no labels and no title keeps its margin, however small,
    // and a side with labels but no title only needs room for the labels.
    #[test]
    fn auto_layout_sizes_a_band_without_a_title_for_its_labels_alone() {
        let measure = Labels { decimal: "-1.2345" };
        let mut cfg = default_config();
        cfg.left_y.out_margin = 20.0;
        cfg.right_y.out_margin = 1.0;
        let out = auto_layout(&cfg, &measure);
        let reach = LABEL_GAP + 56.0;
        assert_eq!(out.left_y.out_margin, (reach + LABEL_GAP).ceil());
        assert_eq!(out.right_y.out_margin, 1.0);
        assert_eq!(out.top_x.out_margin, cfg.top_x.out_margin);
    }

    // A title the host offset into its labels is moved out of them, by whole
    // pixels and no further than it must; a title offset the other way, but
    // still clear and inside its band, is left where it is.
    #[test]
    fn auto_layout_moves_an_offset_title_only_as_far_as_it_must() {
        let measure = Labels { decimal: "0.5" };
        let mut cfg = with_titles("y");
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
        let long = Labels { decimal: "0.00300" };
        let out = auto_layout(&default_config(), &long);
        assert_eq!(out.left_y.label_style.format, LabelFormat::Power);
        assert_eq!(out.bottom_x.label_style.format, LabelFormat::Power);
        // Hidden labels are not drawn, so they are never too long.
        assert_eq!(out.top_x.label_style.format, LabelFormat::Decimal);
        assert_eq!(auto_layout(&out, &long), out);

        let short = Labels { decimal: "0.003" };
        assert_eq!(auto_layout(&default_config(), &short), default_config());

        let mut kept = default_config();
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
        let measure = Labels { decimal: "-1.2345" };
        let mut cfg = with_titles("Optical depth");
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 180,
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
        let measure = Labels { decimal: "-1.2345" };
        let mut cfg = default_config();
        let mut bar = crate::default::default_colorbar_options();
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
        assert_eq!(
            bar.axis.out_margin,
            (13.0 + 2.0 * (reach + LABEL_GAP)).ceil()
        );
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
        let measure = Labels { decimal: "-1.2345" };
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
