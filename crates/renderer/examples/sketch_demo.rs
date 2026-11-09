//! Headless sketch-mode demo — exports PNGs comparing precise vs sketch.
//!
//! Run: `cargo run -p figgy-renderer --example sketch_demo`
//! Output: `target/sketch_demo/{precise,sketch,sketch_tour}.png`

use renderer::color::Color;
use renderer::config::{DrawStyle, LegendEntryKind, SketchOptions};
use renderer::data_config::{DataRenderType, ScatterShape, SeriesConfig, XySeries};
use renderer::line::LineStylePreset;
use renderer::{Chart, ExportOptions, ExportSource, Renderer};

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], path: &str) {
    // Exported over white for viewing.
    let options = ExportOptions::panel(1.0).with_clear(Color::WHITE);
    r.save_blocking(ExportSource::chart(chart, series), options, path)
        .expect("export");
    println!("wrote {path}");
}

fn main() {
    let mut r = Renderer::headless_blocking().expect("renderer");

    std::fs::create_dir_all("target/sketch_demo").expect("mkdir");

    // ── Star-history-like S-curve (sparse points → visible markers) ──
    let n = 44;
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let stars: Vec<f64> = xs
        .iter()
        .map(|&x| {
            3.0 + 40.0 / (1.0 + (-26.0 * (x - 0.27)).exp())
                + 34.0 / (1.0 + (-8.5 * (x - 0.62)).exp())
        })
        .collect();
    r.add_column("x", &xs).unwrap();
    r.add_column("stars", &stars).unwrap();

    let accent = Color::from_rgb8(214, 86, 54);
    let star_series = [SeriesConfig::scatter_line("stars", "x", "stars")
        .label("figgy/sketch")
        .color(accent)
        .width(3.0)
        .marker(ScatterShape::CircleFilled, 6.5)];

    let star_chart = |style: DrawStyle| {
        let mut chart = Chart::sized(920, 600);
        chart.config_mut().draw_style = style;
        let mut chart = chart
            .with_title("Star history")
            .with_x_title("Date")
            .with_y_title("GitHub Stars")
            .with_legend_from(&star_series);
        chart.set_x_range(-0.02, 1.02);
        chart.set_y_range(0.0, 85.0);
        chart
    };

    export(
        &mut r,
        &star_chart(DrawStyle::Precise),
        &star_series,
        "target/sketch_demo/precise.png",
    );
    export(
        &mut r,
        &star_chart(DrawStyle::Sketch(SketchOptions {
            amplitude_px: 2.2,
            wavelength_px: 55.0,
            seed: 1,
        })),
        &star_series,
        "target/sketch_demo/sketch.png",
    );

    // ── Tour: solid+markers / dashed line / errorbar series + dot grid ──
    let m = 36;
    let txs: Vec<f64> = (0..m).map(|i| i as f64 / (m - 1) as f64).collect();
    let ya: Vec<f64> = txs.iter().map(|&x| 60.0 + 24.0 * (x * 6.0).sin()).collect();
    let yb: Vec<f64> = txs
        .iter()
        .map(|&x| 36.0 + 22.0 * x - 8.0 * (x * 9.0).cos())
        .collect();
    let yc: Vec<f64> = txs
        .iter()
        .map(|&x| 12.0 + 9.0 * (x * 4.5 + 1.2).sin())
        .collect();
    let err: Vec<f64> = txs
        .iter()
        .map(|&x| 2.2 + 1.3 * (x * 7.0).cos().abs())
        .collect();
    r.add_column("t", &txs).unwrap();
    r.add_column("ya", &ya).unwrap();
    r.add_column("yb", &yb).unwrap();
    r.add_column("yc", &yc).unwrap();
    r.add_column("err", &err).unwrap();

    let blue = Color::from_rgb8(38, 104, 214);
    let green = Color::from_rgb8(44, 140, 80);
    let mut meas = SeriesConfig::scatter("meas", "t", "yc")
        .color(green)
        .marker(ScatterShape::Diamond, 5.0)
        .y_errors("err");
    if let DataRenderType::Xy(XySeries {
        errors: Some(errors),
        ..
    }) = &mut meas.render_type
    {
        errors.style.error_bar_width = 1.6;
        errors.style.cap_width = 1.6;
    }
    let tour_series = [
        SeriesConfig::scatter_line("wave", "t", "ya")
            .color(accent)
            .width(2.5)
            .marker(ScatterShape::CircleFilled, 5.5),
        SeriesConfig::line("trend", "t", "yb")
            .color(blue)
            .width(2.5)
            .dash(LineStylePreset::Dash),
        meas,
    ];

    let mut chart = Chart::sized(920, 600);
    let config = chart.config_mut();
    config.grid.show_major_x = true;
    config.grid.show_major_y = true;
    config.grid.show_minor_x = true;
    config.grid.show_minor_y = true;
    config.grid.minor_x_style = LineStylePreset::Dot;
    config.grid.minor_y_style = LineStylePreset::Dot;
    config.draw_style = DrawStyle::Sketch(SketchOptions {
        amplitude_px: 2.5,
        wavelength_px: 45.0,
        seed: 3,
    });
    // The legend marks are hand-picked: a solid rule for the dashed trend
    // and a circle for the diamond markers, unlike the series' own marks.
    let mut chart = chart
        .with_title("figgy draw_style tour — sketch")
        .with_x_title("t")
        .with_y_title("value")
        .with_legend_entry("solid + markers", accent, 2.5, LegendEntryKind::LineScatter)
        .with_legend_entry("dashed line", blue, 2.5, LegendEntryKind::Line)
        .with_legend_entry("scatter + errorbar", green, 2.5, LegendEntryKind::Scatter);
    chart.set_x_range(-0.03, 1.03);
    chart.set_y_range(0.0, 95.0);

    export(
        &mut r,
        &chart,
        &tour_series,
        "target/sketch_demo/sketch_tour.png",
    );
}
