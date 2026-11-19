//! The default colour cycle and the palette presets end to end (#357): the
//! colours written where the series are built are the colours the chart
//! draws, picks and highlights, the legend shows, every export carries and
//! a saved document keeps. Nothing recolours a loaded document.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;
use test_gpu::BlockOn as _;

use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::layout::Rect;
use renderer::theme::{ChartKind, ThemeFamily};
use renderer::{
    Chart, Color, ColorCycle, DataRenderType, ExportOptions, ExportSource, RasterImage, Renderer,
    SeriesConfig,
};

const SERIES: usize = 6;
const N: usize = 50;

fn chart_and_series(r: &mut Renderer) -> (Chart, Vec<SeriesConfig>) {
    let x: Vec<f64> = (0..N).map(|i| i as f64 / (N - 1) as f64 * 10.0).collect();
    r.add_column("x", &x).unwrap();
    let mut series = Vec::new();
    for k in 0..SERIES {
        // Flat lines, one per band, so each series has pixels of its own.
        let y = vec![k as f64 + 0.5; N];
        r.add_column(format!("y{k}"), &y).unwrap();
        series.push(
            SeriesConfig::scatter_line(format!("s{k}"), "x", format!("y{k}"))
                .label(&format!("series {k}"))
                .width(3.0),
        );
    }
    ColorCycle::DEFAULT.color_unset(&mut series);
    let mut chart = Chart::sized(640, 480)
        .with_title("Six series")
        .with_legend_from(&series);
    chart.set_x_range(-0.5, 10.5);
    chart.set_y_range(0.0, SERIES as f64);
    chart.fit_layout();
    (chart, series)
}

fn line_color(series: &SeriesConfig) -> Color {
    match &series.render_type {
        DataRenderType::Xy(xy) => xy.line.as_ref().unwrap().line_color,
        _ => unreachable!("x-y series"),
    }
}

fn rgb(color: Color) -> [u8; 3] {
    [color.r, color.g, color.b].map(|c| (c * 255.0).round() as u8)
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba_blocking(
        ExportSource::chart(chart, series),
        ExportOptions::panel(scale).with_clear(Color::WHITE),
    )
    .unwrap()
}

fn has_color(img: &RasterImage, color: Color) -> usize {
    let target = rgb(color).map(i32::from);
    img.rgba
        .chunks_exact(4)
        .filter(|p| (0..3).all(|i| (i32::from(p[i]) - target[i]).abs() <= 6))
        .count()
}

/// Data y → panel pixel y.
fn y_px(chart: &Chart, y: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().unwrap();
    let t = (y - cfg.left_y.min) / (cfg.left_y.max - cfg.left_y.min);
    (da.y + da.height) as f32 - t as f32 * da.height as f32
}

/// Data x → panel pixel x.
fn x_px(chart: &Chart, x: f64) -> f32 {
    let cfg = chart.config();
    let da = cfg.data_area().unwrap();
    let t = (x - cfg.bottom_x.min) / (cfg.bottom_x.max - cfg.bottom_x.min);
    da.x as f32 + t as f32 * da.width as f32
}

// The series take the default cycle in order, and draw in exactly those
// colours at 1x and 2x, beside their legend swatches.
#[test]
fn uncoloured_series_draw_the_default_cycle_at_1x_and_2x() {
    let mut r = Renderer::headless_blocking().expect("headless renderer");
    let (chart, series) = chart_and_series(&mut r);
    for (k, item) in series.iter().enumerate() {
        assert_eq!(line_color(item), ColorCycle::DEFAULT.color(k), "series {k}");
    }
    // The legend symbols carry the series colours: one segment per series
    // in that colour.
    let legend = &chart.config().legend.content.segments;
    for (k, item) in series.iter().enumerate() {
        let color = line_color(item);
        assert!(
            legend.iter().any(|segment| segment.color == color),
            "series {k}: no legend symbol in its colour"
        );
    }
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        for (k, item) in series.iter().enumerate() {
            let count = has_color(&img, line_color(item));
            assert!(count > 100, "{scale}x: series {k} drew {count} pixels");
        }
        // No series is black any more: only text and the frame are dark,
        // and those are thin.
        let black = has_color(&img, Color::BLACK);
        assert!(
            black < img.rgba.len() / 4 / 20,
            "{scale}x: {black} black pixels"
        );
    }
}

// Picks, the picked highlight and stepping work on cycled series as on
// any other: the pick names the series under the cursor, the highlight
// draws, and step_selection moves along the series.
#[test]
fn cycled_series_pick_highlight_and_step() {
    let mut r = Renderer::headless_blocking().expect("headless renderer");
    let (mut chart, series) = chart_and_series(&mut r);
    let _ = r.enable_gpu_picking();
    let id = r
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let rect = chart.config().chart_area.0;
    let index = 20;
    let x = index as f64 / (N - 1) as f64 * 10.0;
    let at = [x_px(&chart, x), y_px(&chart, 2.5)];
    let request = renderer::GpuPickRequest {
        canvas_position_px: at,
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: rect.width,
            height: rect.height,
        },
        display_scale: 1.0,
        max_distance_px: 4.0,
    };
    let picked = r.pick_chart_data(id, request).unwrap().resolve().block_on();
    r.remove_chart(id).unwrap();
    match picked.unwrap() {
        Some(renderer::PickedData::Point {
            series_id,
            point_index,
            ..
        }) => {
            assert_eq!(series_id, "s2");
            assert_eq!(point_index, index);
        }
        other => panic!("expected series s2's point: {other:?}"),
    }

    let plain = export(&mut r, &chart, &series, 1.0);
    let magenta = Color::new(1.0, 0.0, 1.0, 1.0);
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        visible: true,
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "s2".into(),
            point_index: index,
        }],
        highlight_color: magenta,
        outline_width_px: 3.0,
        point_radius_extra_px: 4.0,
        contour_width_extra_px: 0.0,
    });
    let highlighted = export(&mut r, &chart, &series, 1.0);
    assert_eq!(has_color(&plain, magenta), 0);
    assert!(has_color(&highlighted, magenta) > 10, "the highlight draws");

    let stepped = chart
        .config_mut()
        .step_selection(&series, 0, 1, |_| Some(N))
        .unwrap();
    assert!(stepped);
    let refs = &chart.config().picked_data.as_ref().unwrap().refs;
    assert!(
        matches!(&refs[0], PickedDataRef::Point { series_id, .. } if series_id != "s2"),
        "{refs:?}"
    );
}

// SVG and PDF carry the cycle colours: every legend swatch is a vector
// shape in its series colour.
#[test]
fn vector_exports_carry_the_cycle_colours() {
    let mut r = Renderer::headless_blocking().expect("headless renderer");
    let (chart, series) = chart_and_series(&mut r);
    let options = ExportOptions::document(1.0);
    let svg = r
        .export_svg_blocking(ExportSource::chart(&chart, &series), options)
        .unwrap();
    for (k, item) in series.iter().enumerate() {
        let [red, green, blue] = rgb(line_color(item));
        let hex = format!("#{red:02x}{green:02x}{blue:02x}");
        assert!(svg.contains(&hex), "series {k}: {hex} not in the SVG");
    }
    let pdf = r
        .export_pdf_blocking(ExportSource::chart(&chart, &series), options)
        .unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
}

// The colours are plain values in the document: a save and a load keep
// them and draw the same image, and a document that stored black stays
// black, since loading never recolours a series.
#[cfg(feature = "serde")]
#[test]
fn a_saved_document_keeps_its_colours() {
    let mut r = Renderer::headless_blocking().expect("headless renderer");
    let (chart, series) = chart_and_series(&mut r);
    let json = serde_json::to_string(&series).unwrap();
    let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, series);
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    for scale in [1.0f32, 2.0] {
        assert_eq!(
            export(&mut r, &chart, &series, scale).rgba,
            export(&mut r, &chart, &back, scale).rgba,
            "{scale}x"
        );
    }
    // A document whose series were stored black, as earlier releases
    // wrote them.
    let mut black = series.clone();
    for item in &mut black {
        *item = item.clone().color(Color::BLACK);
    }
    let json = serde_json::to_string(&black).unwrap();
    let loaded: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
    assert!(loaded.iter().all(|item| line_color(item) == Color::BLACK));
    let img = export(&mut r, &chart, &loaded, 1.0);
    assert_eq!(has_color(&img, ColorCycle::DEFAULT.color(1)), 0);
}

// A family's look with a preset's colours: the gallery line chart in
// ColorBrewer Dark2 draws its series in Dark2's colours.
#[test]
fn a_theme_takes_a_palette_preset() {
    let mut r = Renderer::headless_blocking().expect("headless renderer");
    let (mut chart, mut series) = chart_and_series(&mut r);
    let theme = ThemeFamily::Gallery
        .theme(ChartKind::Line)
        .with_palette(ColorCycle::Dark2);
    chart.apply_custom_theme(&theme, &mut series).unwrap();
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        for (k, item) in series.iter().enumerate() {
            assert_eq!(line_color(item), ColorCycle::Dark2.color(k));
            assert!(
                has_color(&img, ColorCycle::Dark2.color(k)) > 100,
                "{scale}x {k}"
            );
        }
    }
}
