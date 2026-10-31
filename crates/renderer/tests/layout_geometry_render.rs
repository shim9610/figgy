//! Pixel test of the fixed layout: four axes of different formats, a title and
//! a legend in each corner. The axes frame the data area the layout computes,
//! and the legend's pixels fall inside the data area in its corner's quadrant.
//! Each chart is written out as a PNG, to `target/arch-after/` (or
//! `$FIGGY_LAYOUT_PROBE_DIR`, which `ci/verify.py` points at the CI image
//! directory), so the layout can be looked at too.

use std::fs;
use std::path::{Path, PathBuf};

use renderer::axis_render::try_raster_chart_to_rgba;
use renderer::config::{AxisScale, LegendCorner};
use renderer::format::{LabelFormat, TimestampLabelFormat};
use renderer::layout::{ChartArea, Rect};
use renderer::text::RichText;
use renderer::{Color, RasterImage, encode_png};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 640;

fn probe_config(corner: LegendCorner) -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });

    config.chart_title.text = RichText::plain("Layout geometry probe", Color::BLACK, 28.0, "");
    config.chart_title.visible = true;

    config.bottom_x.min = -1.25;
    config.bottom_x.max = 3.75;
    config.bottom_x.major_spacing = 0.5;
    config.bottom_x.label_style.format = LabelFormat::Decimal;
    config.bottom_x.title_option.text =
        RichText::plain("Decimal bottom axis", Color::BLACK, 22.0, "");

    config.top_x.min = 0.001;
    config.top_x.max = 1000.0;
    config.top_x.major_spacing = 1.0;
    config.top_x.scale = AxisScale::Logarithmic;
    config.top_x.label_style.visible = true;
    config.top_x.label_style.label_visible = true;
    config.top_x.label_style.format = LabelFormat::Power;
    config.top_x.title_option.visible = true;
    config.top_x.title_option.text = RichText::plain("Power top axis", Color::BLACK, 22.0, "");
    config.top_x.out_margin = 80.0;

    config.left_y.min = 1_704_067_200.0;
    config.left_y.max = 1_704_153_600.0;
    config.left_y.major_spacing = 14_400.0;
    config.left_y.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat::default());
    config.left_y.title_option.text =
        RichText::plain("Timestamp left axis", Color::BLACK, 22.0, "");
    config.left_y.out_margin = 230.0;

    config.right_y.min = -50.0;
    config.right_y.max = 150.0;
    config.right_y.major_spacing = 25.0;
    config.right_y.label_style.visible = true;
    config.right_y.label_style.label_visible = true;
    config.right_y.label_style.format = LabelFormat::Decimal;
    config.right_y.title_option.visible = true;
    config.right_y.title_option.text =
        RichText::plain("Decimal right axis", Color::BLACK, 22.0, "");
    config.right_y.out_margin = 110.0;

    config.legend.visible = true;
    config.legend.corner = corner;
    config.legend.content = RichText::plain(
        "solid series\ndashed series\npoint series",
        Color::BLACK,
        14.0,
        "",
    );
    config
}

fn output_dir() -> PathBuf {
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let configured = std::env::var_os("FIGGY_LAYOUT_PROBE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/arch-after"));
    if configured.is_absolute() {
        configured
    } else {
        package_root.join(configured)
    }
}

fn raster(config: &renderer::Config) -> Vec<u8> {
    let rgba = try_raster_chart_to_rgba(config).expect("layout raster");
    assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
    rgba
}

/// Whether the pixel column `x` (or row `y`) carries ink along most of the
/// data area's edge: an axis line.
fn inked_along(rgba: &[u8], xs: impl Iterator<Item = u32>, ys: impl Iterator<Item = u32>) -> f64 {
    let ys: Vec<u32> = ys.collect();
    let mut inked = 0usize;
    let mut total = 0usize;
    for x in xs {
        for y in &ys {
            total += 1;
            if rgba[((y * WIDTH + x) * 4 + 3) as usize] != 0 {
                inked += 1;
            }
        }
    }
    inked as f64 / total.max(1) as f64
}

#[test]
fn the_axes_frame_the_data_area_and_the_legend_sits_in_its_corner() {
    let dir = output_dir();
    fs::create_dir_all(&dir).expect("create image directory");

    // Without the legend: the baseline every corner is compared against.
    let mut bare = probe_config(LegendCorner::TopLeft);
    bare.legend.visible = false;
    let baseline = raster(&bare);
    let da = bare.data_area().expect("data area");
    // Each edge of the data area carries an axis line within a pixel or two.
    for (what, xs, ys) in [
        (
            "left",
            da.x.saturating_sub(2)..da.x + 2,
            da.y..da.y + da.height,
        ),
        (
            "right",
            da.x + da.width - 2..da.x + da.width + 2,
            da.y..da.y + da.height,
        ),
        (
            "top",
            da.x..da.x + da.width,
            da.y.saturating_sub(2)..da.y + 2,
        ),
        (
            "bottom",
            da.x..da.x + da.width,
            da.y + da.height - 2..da.y + da.height + 2,
        ),
    ] {
        let share = inked_along(&baseline, xs, ys);
        assert!(
            share > 0.2,
            "{what} edge of the data area at ({}, {}) {} x {}: {share:.2} of the band is inked",
            da.x,
            da.y,
            da.width,
            da.height
        );
    }

    for (name, corner) in [
        ("top-left", LegendCorner::TopLeft),
        ("top-right", LegendCorner::TopRight),
        ("bottom-left", LegendCorner::BottomLeft),
        ("bottom-right", LegendCorner::BottomRight),
    ] {
        let rgba = raster(&probe_config(corner));

        // The legend is the only difference, and it lies inside the data area
        // in its corner's quadrant.
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut changed = 0u32;
        for (i, (a, b)) in rgba
            .chunks_exact(4)
            .zip(baseline.chunks_exact(4))
            .enumerate()
        {
            if a == b {
                continue;
            }
            changed += 1;
            let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
            assert!(
                x >= da.x && x < da.x + da.width && y >= da.y && y < da.y + da.height,
                "{name}: the legend changed pixel ({x}, {y}) outside the data area"
            );
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        assert!(
            changed > 200,
            "{name}: the legend changed only {changed} pixels"
        );
        let (cx, cy) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
        let (mid_x, mid_y) = (
            da.x as f64 + da.width as f64 / 2.0,
            da.y as f64 + da.height as f64 / 2.0,
        );
        let (left, top) = match corner {
            LegendCorner::TopLeft => (true, true),
            LegendCorner::TopRight => (false, true),
            LegendCorner::BottomLeft => (true, false),
            LegendCorner::BottomRight => (false, false),
        };
        assert_eq!(
            (cx < mid_x, cy < mid_y),
            (left, top),
            "{name}: the legend's ink is centred at ({cx:.0}, {cy:.0}) in a data area \
             centred at ({mid_x:.0}, {mid_y:.0})"
        );

        let image = RasterImage {
            width: WIDTH,
            height: HEIGHT,
            rgba,
        };
        let png = encode_png(&image).expect("layout png");
        fs::write(dir.join(format!("layout-{name}.png")), png).expect("write layout png");
    }
}
