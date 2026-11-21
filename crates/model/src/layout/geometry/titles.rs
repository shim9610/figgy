//! Chart and axis title placement and the chart title band.

use super::*;

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
    pub(super) fn above(&self) -> f32 {
        self.padding() + self.ascent()
    }

    /// The room the title takes in its band below its first baseline:
    /// [`Self::descent`] and the padding.
    pub(super) fn below(&self) -> f32 {
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
pub(super) fn centred_baseline(band: f32, above: f32, below: f32) -> f32 {
    (band - above - below) * 0.5 + above
}

/// Room the default title band leaves around one default-size title line: the
/// default `top_margin` minus that line's nominal height, its font size (the
/// em box the renderer also gives a line without ink). Derived rather than
/// restated, so fitted bands follow the defaults if they change.
pub(super) fn chart_title_band_padding() -> f32 {
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
