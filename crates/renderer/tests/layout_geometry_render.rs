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
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("renderer crate is inside workspace crates directory");
    let configured = std::env::var_os("FIGGY_LAYOUT_PROBE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/arch-after"));
    if configured.is_absolute() {
        configured
    } else {
        workspace.join(configured)
    }
}

fn write_probe(dir: &Path, name: &str, corner: LegendCorner) {
    let rgba = try_raster_chart_to_rgba(&probe_config(corner)).expect("layout raster");
    assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
    assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));

    let image = RasterImage {
        width: WIDTH,
        height: HEIGHT,
        rgba: rgba.clone(),
    };
    let png = encode_png(&image).expect("layout png");
    fs::write(dir.join(format!("layout-{name}.rgba")), rgba).expect("write layout rgba");
    fs::write(dir.join(format!("layout-{name}.png")), png).expect("write layout png");
}

#[test]
fn fixed_layout_geometry_probe() {
    let dir = output_dir();
    fs::create_dir_all(&dir).expect("create layout probe directory");

    for (name, corner) in [
        ("top-left", LegendCorner::TopLeft),
        ("top-right", LegendCorner::TopRight),
        ("bottom-left", LegendCorner::BottomLeft),
        ("bottom-right", LegendCorner::BottomRight),
    ] {
        write_probe(&dir, name, corner);
    }
}
