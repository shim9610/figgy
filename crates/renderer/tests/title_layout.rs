//! The title helpers' band fitting and multi-line alignment, checked through
//! the public raster and selection APIs.

use renderer::axis_render::try_raster_chart_to_rgba;
use renderer::config::TickVisibility;
use renderer::layout::{ChartArea, Rect, RectF};
use renderer::text_render::{FontPolicy, measure_rich_text};
use renderer::{Chart, ChartTitleElement, Config, CpuTextMeasure, Selectable, TextAlign};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
const THREE_LINES: &str = "First line\nSecond line\nThird line";

/// A chart whose only ink is the chart title: no axis chrome, labels, axis
/// titles or grid, so every inked pixel belongs to the title.
fn title_only_config() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    for axis in [
        &mut cfg.top_x,
        &mut cfg.bottom_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.label_style.visible = false;
        axis.label_style.label_visible = false;
        axis.title_option.visible = false;
    }
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    cfg
}

fn inked(rgba: &[u8], x: u32, y: u32) -> bool {
    rgba[((y * WIDTH + x) * 4 + 3) as usize] > 0
}

/// Bounding box of all ink in rows `rows`, as whole pixels.
fn ink_bounds(rgba: &[u8], rows: std::ops::Range<u32>) -> Option<RectF> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in rows {
        for x in 0..WIDTH {
            if inked(rgba, x, y) {
                let (x0, y0, x1, y1) = found.unwrap_or((x, y, x, y));
                found = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
            }
        }
    }
    found.map(|(x0, y0, x1, y1)| RectF {
        x: x0 as f32,
        y: y0 as f32,
        width: (x1 - x0 + 1) as f32,
        height: (y1 - y0 + 1) as f32,
    })
}

// A single default-size line already fits the default band, so `with_title`
// leaves the band — and with it the data area and every pixel — as it was.
#[test]
fn with_title_keeps_a_single_default_line_in_the_default_band() {
    let default_band = renderer::default::default_config().chart_title.top_margin;
    for title in ["A sine wave", "Typography", "Cross-section σ(E) — 3.5 px"] {
        let chart = Chart::new(renderer::default::default_config()).with_title(title);
        assert_eq!(
            chart.config().chart_title.top_margin,
            default_band,
            "{title:?}"
        );
    }
}

// The issue: a three-line title (~84 px) centred in the 32 px default band
// started above the canvas and lost the top of its first line. `with_title`
// now grows the band, so the whole block is drawn below the canvas top.
#[test]
fn with_title_grows_the_band_so_a_multi_line_title_is_not_clipped() {
    let default_band = renderer::default::default_config().chart_title.top_margin;
    let mut chart = Chart::new(title_only_config()).with_title(THREE_LINES);
    let cfg = chart.config().clone();
    let block = measure_rich_text(&cfg.chart_title.text, FontPolicy::Standard);
    assert!(block.height() > default_band);
    assert!(cfg.chart_title.top_margin >= block.height());
    assert!(cfg.data_area().is_ok());

    let rgba = try_raster_chart_to_rgba(&cfg).expect("raster");
    assert!(
        (0..WIDTH).all(|x| !inked(&rgba, x, 0)),
        "nothing may be cut off at the canvas top"
    );
    let ink = ink_bounds(&rgba, 0..HEIGHT).expect("title ink");
    assert!(ink.y >= 1.0);
    assert!(
        ink.y + ink.height <= cfg.chart_title.top_margin,
        "the block fits its band: ink to {} in a {} px band",
        ink.y + ink.height,
        cfg.chart_title.top_margin
    );

    // Fitting is idempotent and grow-only.
    assert!(!chart.fit_chart_title_margin());
    chart.config_mut().chart_title.top_margin = 200.0;
    assert!(!chart.fit_chart_title_margin());
    assert_eq!(chart.config().chart_title.top_margin, 200.0);
}

// Direct SSoT injection is the host's to own: a title set through the config,
// not through the helper, keeps the band it was given and is placed exactly as
// before (centred in the band, even when that starts it above the chart).
#[test]
fn a_title_injected_through_the_config_keeps_its_band() {
    let mut cfg = title_only_config();
    cfg.chart_title.text.segments = renderer::text::rich_segments_from_text(THREE_LINES);
    let band = cfg.chart_title.top_margin;
    let chart = Chart::new(cfg.clone());
    assert_eq!(chart.config().chart_title.top_margin, band);

    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let block = measure_rich_text(&cfg.chart_title.text, FontPolicy::Standard);
    let bounds = ChartTitleElement
        .bounds(chart.config(), &measure)
        .expect("title bounds");
    assert_eq!(bounds.y, (band - block.height()) * 0.5);
}

// A single-line title through the helper draws exactly what the same title
// injected through the config draws: the helper only acts on clipped titles.
#[test]
fn with_title_draws_a_fitting_title_exactly_as_before() {
    for title in ["A sine wave", "Typography", "Tgy"] {
        let helper = Chart::new(title_only_config()).with_title(title);
        let mut injected = title_only_config();
        injected.chart_title.text.segments = renderer::text::rich_segments_from_text(title);
        assert_eq!(helper.config(), &injected, "{title:?}");
        assert_eq!(
            try_raster_chart_to_rgba(helper.config()).expect("raster"),
            try_raster_chart_to_rgba(&injected).expect("raster"),
            "{title:?}"
        );
    }
}

// Center alignment centres every line of a multi-line title on the chart;
// Left starts every line at the block's left edge. The measured block (and
// with it the hit box) is the same either way.
#[test]
fn title_alignment_positions_each_line_inside_the_block() {
    let render = |align: TextAlign| {
        let mut chart = Chart::new(title_only_config());
        chart.config_mut().chart_title.text.align = align;
        let chart = chart.with_title("A much wider first line\nab");
        let cfg = chart.config().clone();
        let measure = CpuTextMeasure::for_style(&cfg.draw_style);
        let bounds = ChartTitleElement.bounds(&cfg, &measure).expect("bounds");
        // `ascent` is the first line's rise. That line has no descenders, so
        // a row just below its baseline separates it from the second line.
        let block = measure_rich_text(&cfg.chart_title.text, FontPolicy::Standard);
        let split = (bounds.y + block.ascent + 4.0) as u32;
        let rgba = try_raster_chart_to_rgba(&cfg).expect("raster");
        let line1 = ink_bounds(&rgba, 0..split).expect("first line ink");
        let line2 = ink_bounds(&rgba, split..HEIGHT).expect("second line ink");
        (bounds, line1, line2)
    };

    let (left_bounds, left1, left2) = render(TextAlign::Left);
    let (center_bounds, center1, center2) = render(TextAlign::Center);
    assert_eq!(
        left_bounds, center_bounds,
        "alignment never moves the block"
    );
    assert_eq!(left1, center1, "the widest line has no slack to spend");

    let chart_centre = WIDTH as f32 * 0.5;
    let centre_of = |r: RectF| r.x + r.width * 0.5;
    assert!(
        (centre_of(center2) - chart_centre).abs() <= 2.0,
        "centred second line at {}, chart centre {chart_centre}",
        centre_of(center2)
    );
    assert!(
        (left2.x - left1.x).abs() <= 2.0,
        "left-aligned lines share the left edge: {} vs {}",
        left2.x,
        left1.x
    );
}
