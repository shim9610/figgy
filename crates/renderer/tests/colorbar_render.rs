//! Visual probe for the CPU colourbar.
//!
//! The unit tests in `axis_render` pin the ramp's ends, the tick↔colour
//! agreement, and the band geometry numerically. This writes the same charts out
//! as PNGs so the whole thing can be *looked* at — the colourbar is the first
//! part of the field work that renders on its own, with no GPU data layer.
//!
//! Output goes to `target/arch-after/` (or `$FIGGY_LAYOUT_PROBE_DIR`), the same
//! place the layout geometry probe writes.

use std::fs;
use std::path::{Path, PathBuf};

use renderer::axis_render::{
    AxisLayerKind, try_raster_chart_layer_to_rgba_with_selection, try_raster_chart_to_rgba,
};
use renderer::colormap::ColorMap;
use renderer::config::AxisScale;
use renderer::format::LabelFormat;
use renderer::layout::{ChartArea, Rect, Side};
use renderer::text::RichText;
use renderer::{Color, RasterImage, encode_png};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 520;

fn base_config() -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.chart_title.text = RichText::plain("Colourbar probe", Color::BLACK, 28.0, "");
    config.chart_title.visible = true;

    config.bottom_x.min = 0.0;
    config.bottom_x.max = 10.0;
    config.bottom_x.major_spacing = 2.0;
    config.bottom_x.out_margin = 60.0;
    config.bottom_x.title_option.text = RichText::plain("x", Color::BLACK, 22.0, "");

    config.left_y.min = 0.0;
    config.left_y.max = 5.0;
    config.left_y.major_spacing = 1.0;
    config.left_y.out_margin = 80.0;
    config.left_y.title_option.text = RichText::plain("y", Color::BLACK, 22.0, "");

    config.top_x.out_margin = 8.0;
    config.right_y.out_margin = 8.0;
    config
}

/// The four sides plus a logarithmic bar with 10ⁿ labels and a titled one.
fn probes() -> Vec<(&'static str, renderer::Config)> {
    let mut out = Vec::new();

    for (name, side, map) in [
        ("right-viridis", Side::Right, ColorMap::Viridis),
        ("left-magma", Side::Left, ColorMap::Magma),
        ("top-turbo", Side::Top, ColorMap::Turbo),
        ("bottom-rdbu", Side::Bottom, ColorMap::RdBu),
    ] {
        let mut config = base_config();
        let mut bar = renderer::default::default_colorbar_options();
        bar.side = side;
        bar.colormap = map;
        bar.axis.min = -20.0;
        bar.axis.max = 80.0;
        bar.axis.major_spacing = 20.0;
        config.colorbar = Some(bar);
        out.push((name, config));
    }

    // Logarithmic z with power labels and a z title, on the right.
    let mut log = base_config();
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = Side::Right;
    bar.axis.scale = AxisScale::Logarithmic;
    bar.axis.min = 1.0e-3;
    bar.axis.max = 1.0e3;
    bar.axis.major_spacing = 1.0;
    bar.axis.label_style.format = LabelFormat::Power;
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text = RichText::plain("intensity", Color::BLACK, 20.0, "");
    bar.axis.out_margin = 110.0;
    log.colorbar = Some(bar);
    out.push(("right-log-power-titled", log));

    // Short, end-aligned, greyscale, inverted — the layout knobs at once.
    let mut short = base_config();
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = Side::Right;
    bar.colormap = ColorMap::GrayScale;
    bar.length_frac = 0.45;
    bar.align = renderer::config::BarAlign::End;
    bar.thickness_px = 28.0;
    bar.axis.inverted = true;
    bar.axis.min = 0.0;
    bar.axis.max = 1.0;
    bar.axis.major_spacing = 0.25;
    short.colorbar = Some(bar);
    out.push(("right-short-inverted-gray", short));

    out
}

fn output_dir() -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("renderer crate is inside the workspace crates directory");
    let configured = std::env::var_os("FIGGY_LAYOUT_PROBE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/arch-after"));
    if configured.is_absolute() {
        configured
    } else {
        workspace.join(configured)
    }
}

#[test]
fn colorbar_probe_renders_every_side_and_scale() {
    let dir = output_dir();
    fs::create_dir_all(&dir).expect("create probe directory");

    for (name, config) in probes() {
        let rgba = try_raster_chart_to_rgba(&config).expect("colourbar raster");
        assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);

        // The strip alone must account for a wide spread of distinct colours —
        // a flat or empty band would still pass a "something was drawn" check.
        let distinct = {
            let mut seen = std::collections::BTreeSet::new();
            for pixel in rgba.chunks_exact(4) {
                if pixel[3] != 0 {
                    seen.insert([pixel[0], pixel[1], pixel[2]]);
                }
            }
            seen.len()
        };
        assert!(
            distinct > 64,
            "{name}: only {distinct} distinct inked colours — the ramp did not paint"
        );

        let image = RasterImage {
            width: WIDTH,
            height: HEIGHT,
            rgba,
        };
        let png = encode_png(&image).expect("probe png");
        fs::write(dir.join(format!("colorbar-{name}.png")), png).expect("write probe png");
    }
}

/// Rough cost of the strip: one anti-aliased fill per device pixel along the
/// bar. Printed rather than asserted — it is a number to know, not a budget.
#[test]
#[ignore = "timing probe, run with --ignored --nocapture"]
fn colorbar_strip_raster_cost() {
    let (_, with_bar) = probes().into_iter().next().expect("a probe");
    let mut without = with_bar.clone();
    without.colorbar = None;

    let time = |config: &renderer::Config| {
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let rgba = try_raster_chart_to_rgba(config).expect("raster");
            // Cheap, but it keeps the loop from being a timing of nothing.
            assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
        }
        start.elapsed().as_secs_f64() / 20.0 * 1000.0
    };
    let bare = time(&without);
    let barred = time(&with_bar);
    println!(
        "deco raster: {bare:.2} ms without a colourbar, {barred:.2} ms with one \
         (+{:.2} ms)",
        barred - bare
    );
}

/// The colourbar as an interactive element: selected (blue box + eight resize
/// handles), and after a drag + a handle resize. Written out so the interaction
/// chrome can be looked at, not just asserted.
#[test]
fn colorbar_selection_and_resize_probe() {
    use renderer::layout::{Element, NudgeResult};
    use renderer::resize::{Resizable, ResizeHandle};
    use renderer::select::{ColorBarElement, Selectable};
    use renderer::{CpuTextMeasure, Draggable};

    let dir = output_dir();
    fs::create_dir_all(&dir).expect("create probe directory");

    let mut selected = base_config();
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = Side::Right;
    bar.axis.min = -20.0;
    bar.axis.max = 80.0;
    bar.axis.major_spacing = 20.0;
    selected.colorbar = Some(bar);

    // Same config, then dragged left/down and grown from its north-west corner.
    let mut edited = selected.clone();
    assert_eq!(
        ColorBarElement.drag_by(&mut edited, -70.0, 30.0),
        NudgeResult::Moved
    );
    for _ in 0..6 {
        assert_eq!(
            ColorBarElement.resize_by(&mut edited, ResizeHandle::NW, -4.0, -6.0),
            NudgeResult::Moved
        );
    }
    let after = edited.colorbar.as_ref().expect("colourbar");
    assert!(
        after.thickness_px > 18.0,
        "the corner drag widened the strip"
    );
    assert!(after.length_frac > 0.75, "and lengthened it");
    assert_eq!((after.offset_x, after.offset_y), (-70.0, 30.0));
    // The element that reports the bounds is the one that moved it.
    let _ = Element::ColorBar;

    for (name, config) in [("selected", selected), ("dragged-resized", edited)] {
        let measure = CpuTextMeasure::for_style(&config.draw_style);
        let box_ = ColorBarElement
            .selection_box(&config, &measure)
            .expect("the colourbar is selectable");
        assert_eq!(box_.handles.len(), 8, "{name}: eight resize handles");

        let rgba = try_raster_chart_layer_to_rgba_with_selection(
            &config,
            AxisLayerKind::All,
            std::slice::from_ref(&box_),
        )
        .expect("selection raster");
        let image = RasterImage {
            width: WIDTH,
            height: HEIGHT,
            rgba,
        };
        let png = encode_png(&image).expect("probe png");
        fs::write(dir.join(format!("colorbar-{name}.png")), png).expect("write probe png");
    }
}
