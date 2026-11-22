use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, BarAlign, ColorBarOptions, Config, LegendCorner};
use crate::format::LabelFormat;
use crate::sink::{Border, DrawnPart, Parts, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, RichText, TextExtents};

use super::{
    ChartArea, DataArea, Element, RectF, Side, axis_mut, axis_ref, overlay_axis_on,
    right_out_margin_effective, tick_contribution, top_out_margin_effective,
};

mod auto_layout;
mod breaks;
mod colorbar;
mod titles;

pub use auto_layout::*;
pub use breaks::*;
pub use colorbar::*;
pub use titles::*;

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

/// Baseline origin for one measured tick label. `row` is how far the
/// labels of the row reach above and below their baseline (#357,
/// [`TICK_ROW_REFERENCE`] in the label face): a top label sits on the
/// row's one baseline, `row.descent` above the gap, and a bottom label
/// `row.ascent` below it, whatever its own letters; a label reaching
/// farther (a subscript or a superscript) moves away by its own extent.
pub fn label_origin(
    side: Side,
    tick_position: (f32, f32),
    outward: f32,
    offset: (f32, f32),
    extents: TextExtents,
    row: TextExtents,
) -> (f32, f32) {
    let (x, y) = match side {
        Side::Top => (
            tick_position.0 - extents.width * 0.5,
            tick_position.1 - outward - LABEL_GAP - extents.descent.max(row.descent),
        ),
        Side::Bottom => (
            tick_position.0 - extents.width * 0.5,
            tick_position.1 + outward + LABEL_GAP + extents.ascent.max(row.ascent),
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

/// The letters whose extents are a tick-label row's (#357): capitals,
/// digits, ascenders and round overshoots for the ascent, descenders for
/// the descent, measured in the label face, so every label of a row
/// shares one baseline.
pub const TICK_ROW_REFERENCE: &str = "0AHOSbdfhklgjpqy";

/// Pixel rectangle occupied by a label drawn at a baseline origin.
pub fn label_rect(origin: (f32, f32), extents: TextExtents) -> RectF {
    RectF {
        x: origin.0,
        y: origin.1 - extents.ascent,
        width: extents.width,
        height: extents.height(),
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

#[cfg(test)]
mod tests {
    use super::auto_layout::tests::grid_cell;
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
            let none = TextExtents {
                width: 0.0,
                ascent: 0.0,
                descent: 0.0,
            };
            let actual = label_origin(side, tick, 5.0, offset, m, none);
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
        let default_band = crate::defaults::config().chart_title.top_margin;
        let mut empty = crate::defaults::config();
        assert!(!fit_chart_title_margin(&mut empty, &LineMeasure));
        assert_eq!(empty, crate::defaults::config());

        let mut hidden = titled("one\ntwo\nthree");
        hidden.chart_title.visible = false;
        assert!(!fit_chart_title_margin(&mut hidden, &LineMeasure));
        assert_eq!(hidden.chart_title.top_margin, default_band);

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
        assert_eq!(short.chart_title.top_margin, default_band);
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
}
