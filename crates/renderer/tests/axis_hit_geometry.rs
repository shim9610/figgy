//! Axis lines and ticks are selected where the raster strokes them. A
//! Cartesian or colourbar axis hits inside its selection box, which fits its
//! drawn line and ticks; the polar radius ray hits on each stroke, every hit
//! within the grab slack of its ink. Covers a thick line, ticks on both
//! sides, detached axes and ticks longer than the ray's minimum grab strip.

use renderer::axis_render::try_raster_chart_to_rgba;
use renderer::config::TickVisibility;
use renderer::layout::{ChartArea, Rect, Side};
use renderer::{AxisElement, ColorBarAxisElement, Config, CpuTextMeasure, Selectable};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// A chart with no labels, titles or grid and every axis hidden, so the only
/// ink an axis adds is its own line and ticks.
fn bare_config() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    cfg.chart_title.visible = false;
    for axis in [
        &mut cfg.top_x,
        &mut cfg.bottom_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.label_style.label_visible = false;
        axis.title_option.visible = false;
    }
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    cfg
}

/// The pixels the axis adds: the raster of `cfg` against `hidden`.
fn axis_ink(cfg: &Config, hidden: &Config) -> Vec<u8> {
    let shown = try_raster_chart_to_rgba(cfg).unwrap();
    let without = try_raster_chart_to_rgba(hidden).unwrap();
    shown
        .chunks_exact(4)
        .zip(without.chunks_exact(4))
        .map(|(a, b)| (0..4).map(|k| a[k].abs_diff(b[k])).max().unwrap())
        .collect()
}

/// How an element's hit region relates to its ink.
enum Rule {
    /// The hit region is the selection box, which fits the ink: every point
    /// inside it hits, every point a pixel outside does not, and each edge
    /// lies within the padding (and anti-aliasing) of the ink's extent.
    Box,
    /// The hit region follows each stroke: every hit lies within `slack` of
    /// ink (the grab strip plus anti-aliasing).
    Strokes { slack: f32 },
}

/// The same config with the axis painted fully transparent: the layout,
/// which the axis' visibility feeds, stays the same.
fn transparent(mut cfg: Config, paint: impl FnOnce(&mut Config)) -> Config {
    paint(&mut cfg);
    cfg
}

/// Check one axis element against its ink.
fn check(name: &str, cfg: &Config, hidden: &Config, element: &dyn Selectable, rule: Rule) {
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let ink = axis_ink(cfg, hidden);
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let selection = element
        .selection_box(cfg, &measure)
        .unwrap_or_else(|| panic!("{name}: no selection box"))
        .rect
        .expanded(1.0);
    let mut strong = 0;
    for y in 0..h {
        for x in 0..w {
            let d = ink[y * w + x];
            if d == 0 {
                continue;
            }
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            assert!(
                selection.contains(px, py),
                "{name}: ink at ({px}, {py}) outside the box {selection:?}"
            );
            if d >= 128 {
                strong += 1;
                assert!(
                    element.contains(cfg, &measure, px, py),
                    "{name}: drawn at ({px}, {py}) but not hit"
                );
            }
        }
    }
    assert!(strong > 0, "{name} drew nothing");
    let slack = match rule {
        Rule::Box => {
            let b = selection.expanded(-1.0);
            let ink_box = ink_extent(&ink, w, h);
            for (edge, inked) in [
                (b.x, ink_box.0),
                (b.y, ink_box.1),
                (b.x + b.width, ink_box.2),
                (b.y + b.height, ink_box.3),
            ] {
                assert!(
                    (edge - inked).abs() <= 3.5,
                    "{name}: box {b:?} does not fit the ink {ink_box:?}"
                );
            }
            let (cx, cy) = (b.x + b.width * 0.5, b.y + b.height * 0.5);
            for (x, y) in [
                (b.x + 0.5, cy),
                (b.x + b.width - 0.5, cy),
                (cx, b.y + 0.5),
                (cx, b.y + b.height - 0.5),
            ] {
                assert!(
                    element.contains(cfg, &measure, x, y),
                    "{name}: ({x}, {y}) in the box"
                );
            }
            for (x, y) in [
                (b.x - 1.0, cy),
                (b.x + b.width + 1.0, cy),
                (cx, b.y - 1.0),
                (cx, b.y + b.height + 1.0),
            ] {
                assert!(
                    !element.contains(cfg, &measure, x, y),
                    "{name}: ({x}, {y}) off the box"
                );
            }
            return;
        }
        Rule::Strokes { slack } => slack,
    };
    let reach = slack.ceil() as i64;
    let inked_near = |px: f32, py: f32| {
        let (cx, cy) = (px as i64, py as i64);
        (cy - reach..=cy + reach).any(|y| {
            (cx - reach..=cx + reach).any(|x| {
                (0..w as i64).contains(&x)
                    && (0..h as i64).contains(&y)
                    && ink[y as usize * w + x as usize] > 0
            })
        })
    };
    let mut y = 0.5;
    while y < HEIGHT as f32 {
        let mut x = 0.5;
        while x < WIDTH as f32 {
            if element.contains(cfg, &measure, x, y) {
                assert!(
                    inked_near(x, y),
                    "{name}: ({x}, {y}) hits with no ink nearby"
                );
            }
            x += 2.0;
        }
        y += 2.0;
    }
}

#[test]
fn cartesian_axes_hit_where_they_are_stroked() {
    for (name, side, tick, width, offset) in [
        (
            "bottom, outside ticks",
            Side::Bottom,
            TickVisibility::Outside,
            1.0,
            0.0,
        ),
        (
            "left, both ticks, thick",
            Side::Left,
            TickVisibility::Both,
            3.0,
            0.0,
        ),
        (
            "top, inside ticks, detached",
            Side::Top,
            TickVisibility::Inside,
            1.0,
            -14.0,
        ),
        (
            "right, outside ticks, detached",
            Side::Right,
            TickVisibility::Outside,
            2.0,
            18.0,
        ),
    ] {
        let mut cfg = bare_config();
        let axis = match side {
            Side::Top => &mut cfg.top_x,
            Side::Bottom => &mut cfg.bottom_x,
            Side::Left => &mut cfg.left_y,
            Side::Right => &mut cfg.right_y,
        };
        axis.line_visible = true;
        axis.tick = tick;
        axis.line_width = width;
        axis.major_tick_length = 8.0;
        axis.line_offset = offset;
        let hidden = transparent(cfg.clone(), |c| {
            for axis in [&mut c.top_x, &mut c.bottom_x, &mut c.left_y, &mut c.right_y] {
                axis.line_color.a = 0.0;
            }
        });
        check(name, &cfg, &hidden, &AxisElement { side }, Rule::Box);
    }
}

#[test]
fn the_colourbar_axis_hits_where_it_is_stroked() {
    let mut cfg = bare_config();
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.label_style.label_visible = false;
    bar.axis.title_option.visible = false;
    bar.axis.line_visible = true;
    bar.axis.tick = TickVisibility::Both;
    bar.axis.major_tick_length = 7.0;
    bar.axis.line_offset = 6.0;
    cfg.colorbar = Some(bar);
    let hidden = transparent(cfg.clone(), |c| {
        c.colorbar.as_mut().unwrap().axis.line_color.a = 0.0;
    });
    check(
        "colourbar axis",
        &cfg,
        &hidden,
        &ColorBarAxisElement,
        Rule::Box,
    );
}

// The polar ray is grabbed at least 4 px either side, and on every tick:
// ticks 12 px long reach far past that strip and must hit too.
#[test]
fn the_polar_radius_ray_hits_on_its_line_and_ticks() {
    let mut cfg = bare_config();
    cfg.polar = Some(renderer::polar::PolarOptions::default());
    cfg.left_y.line_visible = true;
    cfg.left_y.tick = TickVisibility::Both;
    cfg.left_y.major_tick_length = 12.0;
    cfg.left_y.line_width = 1.5;
    let hidden = transparent(cfg.clone(), |c| c.left_y.line_color.a = 0.0);
    check(
        "polar ray",
        &cfg,
        &hidden,
        &AxisElement { side: Side::Left },
        Rule::Strokes { slack: 4.5 },
    );
}

/// The extent of all ink, as `(x0, y0, x1, y1)` in pixel edges.
fn ink_extent(ink: &[u8], w: usize, h: usize) -> (f32, f32, f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if ink[y * w + x] > 0 {
                (x0, y0) = (x0.min(x), y0.min(y));
                (x1, y1) = (x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x0 as f32, y0 as f32, x1 as f32, y1 as f32)
}

// In a document with an overlay whose axis band is wider than the parent's,
// the base panel the renderer draws raises the parent's right margin to the
// group band, which moves the right axis title outward. The title is
// selected where that panel draws it.
#[test]
fn an_axis_title_is_hit_where_its_panel_draws_it() {
    use renderer::AxisTitleElement;

    let mut cfg = bare_config();
    cfg.right_y.title_option.visible = true;
    cfg.right_y.title_option.text =
        renderer::text::RichText::plain("Right", renderer::Color::BLACK, 14., "");
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.out_margin = 140.0;
    overlay.y_axis.line_visible = false;
    overlay.y_axis.tick = TickVisibility::None;
    overlay.y_axis.label_style.label_visible = false;
    overlay.y_axis.title_option.visible = false;
    cfg.overlays.push(overlay);

    let base = cfg.expand().unwrap().panels[0].config.clone();
    let mut untitled = base.clone();
    untitled.right_y.title_option.visible = false;
    let ink = axis_ink(&base, &untitled);
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let element = AxisTitleElement { side: Side::Right };
    let selection = element
        .selection_box(&cfg, &measure)
        .unwrap()
        .rect
        .expanded(1.0);
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut strong = 0;
    for y in 0..h {
        for x in 0..w {
            let d = ink[y * w + x];
            if d == 0 {
                continue;
            }
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            assert!(
                selection.contains(px, py),
                "title ink at ({px}, {py}) outside {selection:?}"
            );
            if d >= 128 {
                strong += 1;
                assert!(
                    element.contains(&cfg, &measure, px, py),
                    "title drawn at ({px}, {py}) but not hit"
                );
            }
        }
    }
    assert!(strong > 0, "the title drew nothing");
}
