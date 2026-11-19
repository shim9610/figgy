//! auto_layout: fitting the margin bands to what they hold (#44, #270).

use super::*;

// ---------------------------------------------------------------------------
// auto_layout measurement (#44)
// ---------------------------------------------------------------------------

/// The least padding [`auto_layout`] keeps between what a fitted band
/// holds and the band's outer edge, in pixels: twice [`LABEL_GAP`]. A band
/// with nothing to hold is this padding alone; one that holds text or a
/// drawn axis pads by [`AUTO_LAYOUT_EDGE_PADDING_EMS`] of its font when that
/// is more ([`edge_padding`]).
pub(super) const AUTO_LAYOUT_EDGE_PADDING: f32 = 8.0;

/// The padding [`auto_layout`] keeps between what a fitted band holds and
/// the band's outer edge, as a fraction of the font size of the text at
/// that edge (the band's title, else the labels it holds, else the labels
/// of the axis whose line or ticks it holds): 0.5 em, never less than
/// [`AUTO_LAYOUT_EDGE_PADDING`] (9 px at the default 18 px labels, 11 px
/// beside the default 22 px axis title) (#270).
pub(super) const AUTO_LAYOUT_EDGE_PADDING_EMS: f32 = 0.5;

/// The gap [`auto_layout`] keeps between an axis or colourbar title and its
/// labels, as a fraction of the labels' font size: 0.5 em, never less than
/// [`LABEL_GAP`] (8 px at 16 px labels) (#270). The colourbar title's
/// placement keeps the proportional part alone
/// ([`colorbar_title_placement`]), so a scaled export keeps the 1x
/// proportions.
pub(super) const AUTO_LAYOUT_TITLE_GAP_EMS: f32 = 0.5;

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
pub(super) const AUTO_LAYOUT_TITLE_PADDING_EMS: f32 = 0.3;

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
///   size. Other formats are left alone (`Auto` already switches to an
///   exponent form by its own limits, #357), and a format never switches
///   back.
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
    // Every pass below reads a subplot's text at the sizes it is drawn
    // with (#292): they are written into the working copy, which then
    // draws them as they are, and the stored sizes are put back at the end.
    for (index, subplot) in out.subplots.iter_mut().enumerate() {
        subplot
            .config
            .scale_text_sizes(None, cfg.subplot_text_factor(index));
        subplot.text_scale = crate::layer::SubplotTextScale::Fixed;
    }
    lay_out_chart(&mut out, measure);
    for index in 0..out.subplots.len() {
        let Some(mut chart) = out.subplot_config(index) else {
            continue;
        };
        lay_out_chart(&mut chart, measure);
        adopt_chart_layout(&mut out.subplots[index].config, &chart);
    }
    align_cells(&mut out, measure);
    for (subplot, stored) in out.subplots.iter_mut().zip(&cfg.subplots) {
        subplot.config.scale_text_sizes(Some(&stored.config), 1.0);
        subplot.text_scale = stored.text_scale;
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
pub(super) mod tests {
    use super::*;
    use crate::layout::Rect;

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
        // The subplot is fitted at the text sizes its cell draws (#292).
        let chart = out.subplot_config(0).unwrap();
        let scale = out.subplot_text_factor(0);
        assert!(scale < 1.0, "{scale}");
        let gap = title_gap(chart.left_y.label_style.font_size);
        assert!((gap - GAP * scale).abs() < 1e-3, "{gap}");
        assert!(title_clearance(&chart, Side::Left, &measure) >= gap - 1e-3);
        assert_padded(&chart, &measure);
        assert_eq!(
            out.subplots[0].config.chart_area,
            cfg.subplots[0].config.chart_area
        );
        assert_eq!(auto_layout(&out, &measure), out);
    }

    /// A measure in ems: every glyph half its text's size wide, 0.8 up
    /// and 0.2 down.
    struct Ems;
    impl MeasureText for Ems {
        fn measure_rich(&self, rt: &crate::text::RichText) -> TextExtents {
            TextExtents {
                width: rt.segments.len() as f32 * rt.font_size * 0.5,
                ascent: rt.font_size * 0.8,
                descent: rt.font_size * 0.2,
            }
        }
    }

    // A small cell's bands are fitted to the text it draws (#292): narrower
    // than the same cell's with `Fixed` sizes, each title the scaled gap
    // past its labels and inside the cell, the stored text sizes untouched
    // and a second call changing nothing. The same at an export scale.
    #[test]
    fn auto_layout_fits_a_small_cell_to_its_scaled_text() {
        let measure = Ems;
        let mut cfg = with_titles("Optical depth");
        cfg.chart_title.text.segments = crate::text::rich_segments_from_text("Parent");
        let mut sub = crate::defaults::subplot();
        sub.config = with_titles("Optical depth");
        sub.config.chart_title.text.segments = crate::text::rich_segments_from_text("Cell");
        long_labels(&mut sub.config.left_y);
        let parent = cfg.chart_area.0;
        sub.config.chart_area = ChartArea(Rect {
            x: parent.x + parent.width,
            y: parent.y,
            width: parent.width / 2,
            height: parent.height / 2,
        });
        cfg.subplots.push(sub);
        let mut fixed = cfg.clone();
        fixed.subplots[0].text_scale = crate::layer::SubplotTextScale::Fixed;

        for export in [1.0, 2.0] {
            let (cfg, fixed) = (cfg.scaled(export), fixed.scaled(export));
            let scale = cfg.subplot_text_factor(0);
            assert!((scale - 0.5f32.cbrt()).abs() < 1e-6);
            let out = auto_layout(&cfg, &measure);
            let plain = auto_layout(&fixed, &measure);
            let (cell, wide) = (&out.subplots[0].config, &plain.subplots[0].config);
            for (side, band, fixed_band) in [
                ("left", cell.left_y.out_margin, wide.left_y.out_margin),
                ("bottom", cell.bottom_x.out_margin, wide.bottom_x.out_margin),
                (
                    "title",
                    cell.chart_title.top_margin,
                    wide.chart_title.top_margin,
                ),
            ] {
                assert!(
                    band > 0.0 && band < fixed_band,
                    "{side}: {band} {fixed_band}"
                );
            }
            // The left band holds the scaled labels, gap, title and padding.
            let chart = out.subplot_config(0).unwrap();
            let axis = &chart.left_y;
            let stored = &cfg.subplots[0].config.left_y;
            assert_eq!(
                axis.label_style.font_size,
                stored.label_style.font_size * scale
            );
            let gap = title_gap(axis.label_style.font_size);
            for side in [Side::Left, Side::Bottom] {
                let clearance = title_clearance(&chart, side.clone(), &measure);
                assert!(clearance >= gap - 1e-3, "{side:?}: {clearance}");
                assert!(clearance < gap + 1.0 * export, "{side:?}: {clearance}");
            }
            assert_padded(&chart, &measure);
            // Nothing but the layout fields changed in the document.
            assert_eq!(
                cell.chart_title.text,
                cfg.subplots[0].config.chart_title.text
            );
            assert_eq!(
                cell.left_y.label_style.font_size,
                stored.label_style.font_size
            );
            assert_eq!(cell.left_y.title_option.text, stored.title_option.text);
            assert_eq!(out.subplots[0].text_scale, cfg.subplots[0].text_scale);
            assert_eq!(plain.subplots[0].text_scale, fixed.subplots[0].text_scale);
            assert_eq!(auto_layout(&out, &measure), out, "idempotent");
        }
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
    pub(in super::super) fn grid_cell(x: u32, y: u32, title: &str) -> Config {
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

        // Each cell fitted alone, as before the alignment, at the text
        // sizes it draws (#292: the lone cell is lower than the parent).
        let mut alone = vec![auto_layout(&doc_without_subplots(&doc), &measure)];
        alone.extend(
            (0..doc.subplots.len()).map(|i| auto_layout(&doc.subplot_config(i).unwrap(), &measure)),
        );
        assert_eq!(doc.subplot_text_factor(2), 1.0);
        assert!(doc.subplot_text_factor(3) < 1.0);
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
        assert_eq!(out.subplot_config(3).unwrap(), alone[4]);
        // Its stored text sizes are the ones it was written with.
        assert_eq!(
            out.subplots[3].config.chart_title.text,
            doc.subplots[3].config.chart_title.text
        );
        assert_eq!(
            out.subplots[3].config.left_y.label_style.font_size,
            doc.subplots[3].config.left_y.label_style.font_size
        );
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
