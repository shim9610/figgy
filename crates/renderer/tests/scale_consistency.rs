//! A chart's decorations keep their layout at every output scale: drawn at
//! 2x and 3x (the display config of a HiDPI surface or an export), every
//! element's selection bounds are the 1x bounds scaled, every axis draws the
//! same tick labels, and the image box-filtered back to 1x matches the 1x
//! image up to anti-aliasing and glyph hinting.

use renderer::axis_render::try_raster_chart_to_rgba;
use renderer::layout::{ChartArea, DrawnPart, Element, Rect, RectF, Side};
use renderer::sink::Parts;
use renderer::{Config, CpuTextMeasure, HitMap, MeasureText, SelectionGeometry};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// A chart with every decoration kind visible: chart and axis titles, tick
/// labels on all four axes, a legend and a colourbar with its labels and
/// title.
fn decorated_config() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    let text =
        |s: &str, size: f32| renderer::text::RichText::plain(s, renderer::Color::BLACK, size, "");
    cfg.chart_title.visible = true;
    cfg.chart_title.text = text("Scale consistency", 16.);
    for (axis, title) in [
        (&mut cfg.bottom_x, "Time (s)"),
        (&mut cfg.left_y, "Signal"),
        (&mut cfg.top_x, "Top"),
        (&mut cfg.right_y, "Right"),
    ] {
        axis.title_option.visible = true;
        axis.title_option.text = text(title, 12.);
        axis.label_style.visible = true;
        axis.label_style.label_visible = true;
        axis.line_visible = true;
    }
    cfg.bottom_x.min = -1250.0;
    cfg.bottom_x.max = 98_000.0;
    cfg.left_y.min = -0.004;
    cfg.left_y.max = 0.012;
    cfg.legend.visible = true;
    cfg.legend.content = text("first series\nsecond series", 11.);
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text = text("z", 12.);
    cfg.colorbar = Some(bar);
    cfg
}

const ELEMENTS: [&str; 15] = [
    "chart_title",
    "axis_title_bottom",
    "axis_title_left",
    "axis_title_top",
    "axis_title_right",
    "tick_labels_bottom",
    "tick_labels_left",
    "tick_labels_top",
    "tick_labels_right",
    "axis_bottom",
    "axis_left",
    "legend",
    "colorbar",
    "colorbar_tick_labels",
    "colorbar_title",
];

fn bounds(cfg: &Config, name: &str) -> RectF {
    let map = HitMap::standard_chart();
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let id = map
        .find(name)
        .unwrap_or_else(|| panic!("{name} is not registered"));
    map.get(id)
        .unwrap()
        .bounds(cfg, &measure)
        .unwrap_or_else(|| panic!("{name} has no bounds"))
}

/// How many tick labels the renderer draws for `element`.
fn drawn_labels(cfg: &Config, element: &Element) -> usize {
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let mut count = 0;
    assert!(measure.emit(
        cfg,
        element,
        &mut Parts(&mut |part| {
            if let DrawnPart::Text(_) = part {
                count += 1;
            }
        })
    ));
    count
}

#[test]
fn decorations_keep_their_layout_at_every_scale() {
    keeps_its_layout_at_every_scale(&decorated_config(), true);
}

/// The decorated chart with adaptive logarithmic axes inside one decade
/// and across decades, and an adaptive logarithmic colourbar (#42): the
/// demoted label ladders are the same at every scale.
#[test]
fn adaptive_log_labels_keep_their_layout_at_every_scale() {
    use renderer::config::{AxisOptions, AxisScale, LogTickPolicy};
    use renderer::format::LabelFormat;
    let mut cfg = decorated_config();
    let adaptive = |axis: &mut AxisOptions, min: f64, max: f64, format: LabelFormat| {
        axis.scale = AxisScale::Logarithmic;
        axis.min = min;
        axis.max = max;
        axis.minor_count = 8;
        axis.log_ticks = LogTickPolicy::Adaptive;
        axis.label_style.format = format;
        axis.label_style.significant_digits = 1;
    };
    adaptive(&mut cfg.bottom_x, 3e3, 8e5, LabelFormat::Scientific);
    adaptive(&mut cfg.top_x, 3e3, 8e5, LabelFormat::Scientific);
    adaptive(&mut cfg.left_y, 1e4, 8e4, LabelFormat::Power);
    adaptive(&mut cfg.right_y, 8e3, 1.2e4, LabelFormat::Scientific);
    let bar = &mut cfg.colorbar.as_mut().unwrap().axis;
    adaptive(bar, 1.0, 100.0, LabelFormat::Power);
    // Bounds and label sets only: the box-filtered image comparison is
    // calibrated on the decorated chart's text, and the more numerous log
    // labels at fractional pixel origins differ at their glyph edges by
    // hinting alone (the rasterizer the ladder does not touch). The
    // decoration oracle's I5 compares the adaptive LogAxes scene's 2x ink
    // with its 1x ink up to hinting.
    keeps_its_layout_at_every_scale(&cfg, false);
}

fn keeps_its_layout_at_every_scale(base: &Config, compare_pixels: bool) {
    let base = base.clone();
    let one = try_raster_chart_to_rgba(&base).unwrap();
    for scale in [2u32, 3] {
        let s = scale as f32;
        let cfg = base.scaled(s);

        // Bounds scale with the output, to a pixel at 1x.
        for name in ELEMENTS {
            let b1 = bounds(&base, name);
            let bs = bounds(&cfg, name);
            for (at_scale, at_one, edge) in [
                (bs.x / s, b1.x, "left"),
                (bs.y / s, b1.y, "top"),
                ((bs.x + bs.width) / s, b1.x + b1.width, "right"),
                ((bs.y + bs.height) / s, b1.y + b1.height, "bottom"),
            ] {
                assert!(
                    (at_scale - at_one).abs() <= 1.0,
                    "{scale}x {name}: {edge} edge at {at_scale} against {at_one} at 1x ({bs:?} / {b1:?})"
                );
            }
        }

        // The same labels survive pruning at every scale.
        for element in [
            Element::AxisLabel(Side::Bottom),
            Element::AxisLabel(Side::Left),
            Element::AxisLabel(Side::Top),
            Element::AxisLabel(Side::Right),
            Element::ColorBarLabel,
        ] {
            assert_eq!(
                drawn_labels(&cfg, &element),
                drawn_labels(&base, &element),
                "{scale}x: {element:?} draws a different label set"
            );
        }

        // Box-filtered back to 1x, the image matches the 1x image except for
        // anti-aliasing and hinting at glyph and stroke edges: no region
        // moved.
        if !compare_pixels {
            continue;
        }
        let big = try_raster_chart_to_rgba(&cfg).unwrap();
        let (w, h) = (WIDTH as usize, HEIGHT as usize);
        let mut differing = 0usize;
        let mut worst = 0u32;
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                for dy in 0..scale as usize {
                    for dx in 0..scale as usize {
                        let i = (((y * scale as usize + dy) * w * scale as usize)
                            + x * scale as usize
                            + dx)
                            * 4;
                        for k in 0..4 {
                            sum[k] += big[i + k] as u32;
                        }
                    }
                }
                let n = scale * scale;
                let i = (y * w + x) * 4;
                let diff = (0..4)
                    .map(|k| (sum[k] / n).abs_diff(one[i + k] as u32))
                    .max()
                    .unwrap();
                worst = worst.max(diff);
                if diff > 96 {
                    differing += 1;
                }
            }
        }
        let share = differing as f64 / (w * h) as f64;
        println!(
            "{scale}x: {differing} pixels ({:.3}%) differ by more than 96, worst {worst}",
            share * 100.0
        );
        assert!(
            share < 0.004,
            "{scale}x: {differing} pixels ({:.3}%) differ by more than 96 from 1x",
            share * 100.0
        );
    }
}
