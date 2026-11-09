//! Subplots: a document of several charts, each with its own axes, laid out
//! by their chart areas.
//! - `small_multiples` writes `gallery-subplots.png`: four sensors in a 2 x 2
//!   grid; the three subplots link their X axis to the parent chart
//!   (`CellRef::Parent`) and keep their own Y ranges, exported as one
//!   document.
//! - `mixed_axes` writes `mixed-axes.png`: a 2 x 2 grid where only the
//!   upper-right chart links its X axis, the lower-left one plots humidity
//!   against temperature, and the lower-right one has its own X range and a
//!   twin Y axis from an overlay layer inside the subplot.
//!
//! cargo run --locked -p figgy-renderer --example subplots -- target/subplots
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    CellRef, Chart, Color, Config, ExportOptions, ExportSource, HitMap, LayerRef, OverlayLayer,
    Renderer, SeriesConfig, Subplot,
    data_config::{DataRenderType, XySeries},
    text::RichText,
};
use std::path::Path;
use styled::{chrome, rgb, save};

/// A line series' fill in a colour of its own (`fill_to` fills in the
/// line's colour at a quarter of its opacity).
fn fill_color(mut series: SeriesConfig, color: Color) -> SeriesConfig {
    if let DataRenderType::Xy(XySeries {
        line: Some(line), ..
    }) = &mut series.render_type
    {
        line.fill_color = Some(color);
    }
    series
}

/// A chart of `width` x `height` with its top-left corner at `(x, y)` in
/// the document.
fn placed(x: u32, y: u32, width: u32, height: u32) -> Chart {
    let mut chart = Chart::sized(width, height);
    let area = &mut chart.config_mut().chart_area.0;
    (area.x, area.y) = (x, y);
    chart
}

// Small multiples: four sensors in a 2 x 2 grid of subplots sharing the hour
// axis, each with its own range and colour.
fn small_multiples(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(247, 248, 253);
    let ink = rgb(52, 57, 85);
    let grid = rgb(228, 231, 240);
    let hour: Vec<f64> = (0..=240).map(|i| f64::from(i) / 10.).collect();
    r.add_column("h", &hour)?;
    let cell = |x: u32, y: u32, title: &str, y_title: &str| -> Config {
        let mut chart = placed(x, y, 600, 400);
        let config = chart.config_mut();
        chrome(config, bg, ink, grid);
        config.chart_title.text.font_size = 26.;
        config.chart_title.top_margin = 60.;
        config.left_y.out_margin = 95.;
        config.right_y.out_margin = 40.;
        config.bottom_x.out_margin = 70.;
        config.top_x.out_margin = 20.;
        config.legend.visible = false;
        config.bottom_x.major_spacing = 6.;
        for a in [&mut config.left_y, &mut config.bottom_x] {
            a.label_style.font_size = 18.;
            a.title_option.text.font_size = 19.;
        }
        chart
            .with_title(title)
            .with_x_title("Hour")
            .with_y_title(y_title)
            .config()
            .clone()
    };
    // id, title, unit, colour, mean, daily swing, phase, noise, axis start
    let sensors = [
        (
            "temperature",
            "Temperature",
            "°C",
            rgb(232, 104, 84),
            18.,
            7.,
            -1.2,
            2.,
            8.,
        ),
        (
            "humidity",
            "Humidity",
            "%",
            rgb(37, 150, 160),
            55.,
            20.,
            1.9,
            10.,
            30.,
        ),
        (
            "co2",
            "CO₂",
            "ppm",
            rgb(112, 92, 212),
            620.,
            260.,
            0.4,
            60.,
            300.,
        ),
        (
            "light",
            "Daylight",
            "lux",
            rgb(228, 170, 50),
            400.,
            380.,
            -1.4,
            60.,
            0.,
        ),
    ];
    let mut specs = Vec::new();
    for (id, _, _, color, base, swing, phase, noise, floor) in sensors {
        let data: Vec<f64> = hour
            .iter()
            .map(|h| {
                let daily = (h / 24. * std::f64::consts::TAU + phase).sin();
                (base
                    + swing * daily
                    + noise * 0.12 * (h * 2.3).sin()
                    + noise * 0.07 * (h * 5.1).cos())
                .max(0.)
            })
            .collect();
        r.add_column(id, &data)?;
        specs.push(fill_color(
            SeriesConfig::line(id, "h", id)
                .color(color)
                .width(3.)
                .fill_to(floor),
            Color::from_rgba(color.r, color.g, color.b, 0.1),
        ));
    }
    let mut config = cell(0, 0, sensors[0].1, sensors[0].2);
    config.subplots = [(1usize, 600u32, 0u32), (2, 0, 400), (3, 600, 400)]
        .into_iter()
        .map(|(k, x, y)| Subplot {
            config: cell(x, y, sensors[k].1, sensors[k].2),
            series: vec![sensors[k].0.to_string()],
            link_x: Some(CellRef::Parent),
            ..Default::default()
        })
        .collect();
    let mut chart = Chart::new(config);
    chart.set_x_range(0., 24.);
    chart.set_y_range(8., 28.);
    chart.set_subplot_y_range(0, 30., 80.);
    chart.set_subplot_y_range(1, 300., 950.);
    chart.set_subplot_y_range(2, 0., 850.);
    chart.with_axis_range_change(|config| {
        config.left_y.major_spacing = 5.;
        config.subplots[0].config.left_y.major_spacing = 10.;
        config.subplots[1].config.left_y.major_spacing = 150.;
        config.subplots[2].config.left_y.major_spacing = 200.;
    });
    // Subplot 0 follows the parent's X range; every Y range is its own.
    let document = chart.config();
    let linked = document.subplot_config(0).ok_or("subplot 0 is missing")?;
    assert_eq!(linked.bottom_x.max, document.bottom_x.max);
    // Pointer interaction addresses one chart at a time; a subplot answers
    // with its own elements under `subplot_<n>:` ids.
    let mut interactive = document.clone();
    interactive.set_active_layer(LayerRef::Subplot(2));
    assert!(
        HitMap::for_active_layer(&interactive)
            .find("subplot_2:data_area")
            .is_some()
    );
    let image = r.export_rgba_blocking(
        ExportSource::chart(&chart, &specs),
        ExportOptions::document(1.).with_clear(bg),
    )?;
    save(&image, bg, &out.join("gallery-subplots.png"))
}

// Mixed axes: one linked X axis, one chart with a different X column, one with
// its own X range and a twin Y axis of its own.
fn mixed_axes(r: &mut Renderer, out: &Path) -> Result {
    // One day of hourly readings in three units.
    let hour: Vec<f64> = (0..=24).map(f64::from).collect();
    let wave = |phase: f64| {
        hour.iter()
            .map(move |h| (h / 24.0 * std::f64::consts::TAU + phase).sin())
            .collect::<Vec<f64>>()
    };
    let temperature: Vec<f64> = wave(-1.2).iter().map(|v| 18.0 + 8.0 * v).collect();
    let pressure: Vec<f64> = wave(0.0).iter().map(|v| 1008.0 + 6.0 * v).collect();
    let humidity: Vec<f64> = wave(2.0).iter().map(|v| 60.0 + 25.0 * v).collect();
    for (id, data) in [
        ("hour", hour.clone()),
        ("air_temperature", temperature),
        ("pressure", pressure),
        ("air_humidity", humidity),
    ] {
        r.add_column(id, &data)?;
    }

    // One 480x420 cell: a chart with its own axes, titled.
    let cell = |x: u32, y: u32, title: &str, x_title: &str, y_title: &str| -> Config {
        let mut chart = placed(x, y, 480, 420);
        let config = chart.config_mut();
        config.left_y.out_margin = 100.0;
        config.right_y.out_margin = 40.0;
        config.bottom_x.out_margin = 70.0;
        config.top_x.out_margin = 16.0;
        chart
            .with_title(title)
            .with_x_title(x_title)
            .with_y_title(y_title)
            .config()
            .clone()
    };
    let subplot = |config: Config, series: &[&str]| Subplot {
        config,
        series: series.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    };

    // The document's own chart is the upper-left cell; three subplots fill
    // the grid. Only the upper-right one links an axis (X, to the parent).
    let mut config = cell(0, 0, "Temperature", "Hour", "°C");
    let mut pressure_by_hour = subplot(cell(480, 0, "Pressure", "Hour", "hPa"), &["pressure"]);
    pressure_by_hour.link_x = Some(CellRef::Parent);
    let humidity_by_temperature = subplot(
        cell(0, 420, "Humidity against temperature", "°C", "%"),
        &["humidity_t"],
    );
    let pressure_color = Color::from_rgb8(176, 64, 32);
    let humidity_color = Color::from_rgb8(40, 140, 90);
    let mut daytime = subplot(cell(480, 420, "Daytime", "Hour", "hPa"), &["daytime"]);
    daytime.config.right_y.out_margin = 100.0;
    let mut twin = OverlayLayer::default();
    twin.series = vec!["daytime_humidity".into()];
    twin.y_axis.out_margin = 100.0;
    twin.y_axis.title_option.visible = true;
    twin.y_axis.title_option.text = RichText::plain("%", humidity_color, 22.0, "");
    twin.y_axis.label_style.color = humidity_color;
    daytime.config.overlays.push(twin);
    config.subplots = vec![pressure_by_hour, humidity_by_temperature, daytime];

    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 24.0);
    chart.auto_fit_y(r.pool(), "air_temperature", 0.1)?;
    // Subplot 0's X follows the parent; its Y is its own.
    chart.auto_fit_subplot_y_union(r.pool(), 0, &["pressure"], 0.1)?;
    chart.auto_fit_subplot_x_union(r.pool(), 1, &["air_temperature"], 0.05)?;
    chart.auto_fit_subplot_y_union(r.pool(), 1, &["air_humidity"], 0.1)?;
    chart.set_subplot_x_range(2, 6.0, 18.0);
    chart.auto_fit_subplot_y_union(r.pool(), 2, &["pressure"], 0.1)?;
    chart.with_axis_range_change(|config| {
        let overlay = &mut config.subplots[2].config.overlays[0].y_axis;
        overlay.min = 30.0;
        overlay.max = 90.0;
        overlay.major_spacing = 10.0;
    });

    let plain = |id: &str, x: &str, y: &str, color: Color| {
        SeriesConfig::line(id, x, y).color(color).width(2.5)
    };
    let specs = [
        plain(
            "temperature",
            "hour",
            "air_temperature",
            Color::from_rgb8(34, 104, 198),
        ),
        plain("pressure", "hour", "pressure", pressure_color),
        plain(
            "humidity_t",
            "air_temperature",
            "air_humidity",
            humidity_color,
        ),
        plain("daytime", "hour", "pressure", pressure_color),
        plain("daytime_humidity", "hour", "air_humidity", humidity_color),
    ];

    // Every cell is a chart of its own with its own data area.
    let document = chart.config();
    assert_eq!(document.document_area().width, 960);
    assert_eq!(document.document_area().height, 840);
    let linked = document.subplot_config(0).ok_or("subplot 0 is missing")?;
    assert_eq!(linked.bottom_x.max, document.bottom_x.max);
    // The twin axis inside subplot 2 answers under its own id while that
    // overlay is the active layer.
    let mut interactive = document.clone();
    interactive.set_active_layer(LayerRef::SubplotOverlay(2, 0));
    assert!(
        HitMap::for_active_layer(&interactive)
            .find("subplot_2:overlay_0:axis_right")
            .is_some()
    );
    let path = out.join("mixed-axes.png");
    r.save_blocking(
        ExportSource::chart(&chart, &specs),
        ExportOptions::document(1.0).with_clear(Color::WHITE),
        &path,
    )?;
    println!("{}", path.display());
    Ok(())
}

fn main() -> Result {
    let out = gallery::output_directory("target/subplots")?;
    gallery::with_renderer(|r| {
        small_multiples(r, &out)?;
        mixed_axes(r, &out)
    })
}
