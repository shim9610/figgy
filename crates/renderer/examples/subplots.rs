//! Subplots: a document of several charts, each with its own axes, laid out
//! by their chart areas.
//! - `small_multiples` writes `gallery-subplots.png`: four sensors in a 2 x 2
//!   grid. The document's own chart is the whole figure and draws only its
//!   background; the four cells are subplots, half its width and height, so
//!   each draws the theme's text sizes at its text scale (0.79). Subplots 1
//!   to 3 link their X axis to subplot 0 (`CellRef::Subplot(0)`) and keep
//!   their own Y ranges, exported as one document.
//! - `mixed_axes` writes `mixed-axes.png`: a 2 x 2 grid of the parent and
//!   three subplots of its size, which draw their text as stored. Only the
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
use renderer::theme::ChartKind;
use renderer::{
    CellRef, Chart, Color, Config, ExportOptions, ExportSource, HitMap, LayerRef, Renderer,
    SeriesConfig, Subplot, config::TickVisibility, text::RichText,
};
use std::path::Path;
use styled::{background, save, theme_named, themed};

// Small multiples: four sensors in a 2 x 2 grid of subplots sharing the hour
// axis, each with its own range and colour.
fn small_multiples(r: &mut Renderer, out: &Path) -> Result {
    // The theme's text sizes are the ones of a full-size chart; each cell
    // draws them at its own text scale (#292). The temperature is drawn in
    // coral, the humidity in teal.
    let mut theme = theme_named(ChartKind::Line, "cool-slate")?;
    theme.series.palette.swap(0, 1);
    let bg = background(&theme);
    let hour: Vec<f64> = (0..=240).map(|i| f64::from(i) / 10.).collect();
    r.add_column("h", &hour)?;
    let cell = |x: u32, y: u32, title: &str, y_title: &str| -> Config {
        let mut chart = Chart::placed(x, y, 600, 400);
        chart.config_mut().bottom_x.major_spacing = 6.;
        chart
            .with_title(title)
            .with_x_title("Hour")
            .with_y_title(y_title)
            .config()
            .clone()
    };
    // id, title, unit, mean, daily swing, phase, noise, axis start
    let sensors = [
        ("temperature", "Temperature", "°C", 18., 7., -1.2, 2., 8.),
        ("humidity", "Humidity", "%", 55., 20., 1.9, 10., 30.),
        ("co2", "CO₂", "ppm", 620., 260., 0.4, 60., 300.),
        ("light", "Daylight", "lux", 400., 380., -1.4, 60., 0.),
    ];
    let mut specs = Vec::new();
    for (id, _, _, base, swing, phase, noise, floor) in sensors {
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
        specs.push(SeriesConfig::line(id, "h", id).fill_to(floor));
    }
    // The document's own chart is the whole 1200 x 800 figure and draws
    // only its background; the four sensors are subplots in its quarters.
    // A cell is half the figure's width and height, so its text is drawn at
    // 0.79 of the theme's sizes. Subplots 1 to 3 follow subplot 0's X axis.
    let mut config = Chart::sized(1200, 800).config().clone();
    config.subplots = [(0u32, 0u32), (600, 0), (0, 400), (600, 400)]
        .into_iter()
        .zip(sensors)
        .enumerate()
        .map(|(k, ((x, y), sensor))| Subplot {
            config: cell(x, y, sensor.1, sensor.2),
            series: vec![sensor.0.to_string()],
            link_x: (k > 0).then_some(CellRef::Subplot(0)),
            ..Default::default()
        })
        .collect();
    let mut chart = Chart::new(config);
    chart.set_subplot_x_range(0, 0., 24.);
    chart.set_subplot_y_range(0, 8., 28.);
    chart.set_subplot_y_range(1, 30., 80.);
    chart.set_subplot_y_range(2, 300., 950.);
    chart.set_subplot_y_range(3, 0., 850.);
    chart.with_axis_range_change(|config| {
        for (subplot, spacing) in config.subplots.iter_mut().zip([5., 10., 150., 200.]) {
            subplot.config.left_y.major_spacing = spacing;
        }
    });
    themed(&mut chart, &theme, &mut specs)?;
    // Nothing of the figure itself is drawn over its background.
    chart.with_axis_range_change(|config| {
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.line_visible = false;
            axis.tick = TickVisibility::None;
            axis.label_style.visible = false;
            axis.title_option.visible = false;
        }
        config.chart_title.visible = false;
        config.legend.visible = false;
        config.plot_background = None;
        config.grid.show_major_x = false;
        config.grid.show_major_y = false;
        config.grid.show_minor_x = false;
        config.grid.show_minor_y = false;
    });
    // Subplot 1 follows subplot 0's X range; every Y range is its own. Its
    // text is the stored size times the cell's factor.
    let document = chart.config();
    let linked = document.subplot_config(1).ok_or("subplot 1 is missing")?;
    assert_eq!(
        linked.bottom_x.max,
        document.subplots[0].config.bottom_x.max
    );
    let scale = document.subplot_text_scale(1);
    assert!((scale - 0.5f32.cbrt()).abs() < 1e-6);
    assert_eq!(
        linked.chart_title.text.font_size,
        document.subplots[1].config.chart_title.text.font_size * scale
    );
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
    let cell = |x: u32, y: u32, title: &str, x_title: &str, y_title: &str| -> Chart {
        Chart::placed(x, y, 480, 420)
            .with_title(title)
            .with_x_title(x_title)
            .with_y_title(y_title)
    };
    let subplot = |chart: Chart, series: &[&str]| Subplot {
        config: chart.config().clone(),
        series: series.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    };

    // The document's own chart is the upper-left cell; three subplots fill
    // the grid. Only the upper-right one links an axis (X, to the parent).
    let mut config = cell(0, 0, "Temperature", "Hour", "°C").config().clone();
    let mut pressure_by_hour = subplot(cell(480, 0, "Pressure", "Hour", "hPa"), &["pressure"]);
    pressure_by_hour.link_x = Some(CellRef::Parent);
    let humidity_by_temperature = subplot(
        cell(0, 420, "Humidity against temperature", "°C", "%"),
        &["humidity_t"],
    );
    let pressure_color = Color::from_rgb8(176, 64, 32);
    let humidity_color = Color::from_rgb8(40, 140, 90);
    let mut twin = renderer::default::default_axis_options_y();
    twin.title_option.visible = true;
    twin.title_option.text = RichText::plain("%", humidity_color, 22.0, "");
    twin.label_style.color = humidity_color;
    let daytime = subplot(
        cell(480, 420, "Daytime", "Hour", "hPa").with_overlay(twin, &["daytime_humidity"]),
        &["daytime"],
    );
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
    chart.fit_layout();
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
