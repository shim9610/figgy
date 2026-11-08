//! Polar plots: ordinary line and scatter series drawn in (angle, radius)
//! through `Config.polar`. Each series' `x_column` holds the angle in
//! `polar.angle_unit` and its `y_column` the radius; the renderer projects
//! them, and the angle and radius axes keep their ticks, labels and grid as
//! spokes and rings. `set_y_range` sets the radius axis. `set_x_range` only
//! chooses which angle ticks, spokes and labels are drawn: 360° (or 2π) is
//! always one full turn, and `sector` limits the drawn part of the circle.
//! A heatmap draws its cells as annular sectors between its angle and
//! radius boundaries, with its contours and their labels (#208).
//!
//! Besides the numbered plain checks it writes two designed 1200 x 800
//! feature scenes in the shared gallery chrome (`support/styled.rs`):
//! - `wave_spectrum` writes `gallery-polar-heatmap.png`: a directional wave
//!   spectrum as flat annular cells (direction around, frequency outward)
//!   with labelled energy contours.
//! - `wind_rose` writes `gallery-rose.png`: a wind rose, category bars on a
//!   categorical angle axis stacked by speed band.
//!
//! cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;

use renderer::{
    Color, ColorMap, DataRenderType, Renderer,
    config::{Category, append_legend_entry, series_symbol_segments},
    data_config::{
        BarGroups, CategoryBarMode, CategoryBarStyle, ContourConfig, ContourLabelConfig,
        FieldFillConfig, FieldSeries, FillMode, GridLayout, LineFill, MatrixOrientation, MatrixRef,
        Shading,
    },
    format::{LabelFormat, PlainNumberFormat},
    line::LineStylePreset,
    polar::{AngleUnit, PolarInterpolation, PolarOptions},
    text::RichText,
};
use std::{f64::consts::PI, path::Path};
use styled::{colorbar, export, rgb, rgba, save};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = gallery::output_directory("target/polar-plot")?;
    gallery::with_renderer(|r| {
        let size = (960, 640);
        let degrees: Vec<f64> = (0..=360).map(f64::from).collect();
        r.add_column("deg", &gallery::column(degrees.clone()))?;

        // 1) An antenna pattern: a line filled to the centre, degrees around
        //    the ring with a degree sign, rings every 0.25.
        let pattern: Vec<f64> = degrees
            .iter()
            .map(|d| {
                let t = d.to_radians();
                (0.15 + 0.85 * (2.0 * t).cos().abs()) * (0.6 + 0.4 * t.cos().max(0.0))
            })
            .collect();
        r.add_column("pattern", &gallery::column(pattern))?;
        let mut chart = gallery::chart("Antenna pattern", "Azimuth", "Gain", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, 1.0);
        let mut line = gallery::line(Color::from_rgb8(30, 110, 210), 2.5);
        line.fill = Some(LineFill::ToBaseline(0.0));
        line.fill_color = Some(Color::new(0.12, 0.43, 0.82, 0.25));
        let series = vec![gallery::series(
            "gain",
            "deg",
            "pattern",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(line),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions::default()));
            config.bottom_x.major_spacing = 30.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 0.25;
        }
        gallery::save(r, &chart, &series, &out.join("01-antenna-pattern.png"))?;

        // 2) A compass rose: clockwise from the top, markers on a three-leaf
        //    rose with arc segments between the samples.
        let samples: Vec<f64> = (0..72).map(|i| f64::from(i) * 5.0).collect();
        let rose: Vec<f64> = samples
            .iter()
            .map(|d| 2.0 + (3.0 * d.to_radians()).cos())
            .collect();
        r.add_column("rose_deg", &gallery::column(samples))?;
        r.add_column("rose", &gallery::column(rose))?;
        let mut chart = gallery::chart("Compass rose", "Bearing", "Strength", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, 3.0);
        let series = vec![gallery::series(
            "rose",
            "rose_deg",
            "rose",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(gallery::scatter(Color::from_rgb8(200, 60, 40), 7.0)),
                line: Some(gallery::line(Color::from_rgb8(200, 60, 40), 1.5)),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                interpolation: PolarInterpolation::Arc { pieces: 6 },
                radial_label_angle_degrees: 45.0,
                ..PolarOptions::default()
            }));
            config.bottom_x.major_spacing = 45.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 1.0;
        }
        gallery::save(r, &chart, &series, &out.join("02-compass-rose.png"))?;

        // 3) A dashed spiral from 24 sparse samples over two turns: the arc
        //    interpolation bends every segment along the data, and the dash
        //    pattern walks the bent path.
        let turns: Vec<f64> = (0..24).map(|i| f64::from(i) * 30.0).collect();
        let spiral: Vec<f64> = turns.iter().map(|d| d / 720.0 * 5.0).collect();
        r.add_column("spiral_deg", &gallery::column(turns))?;
        r.add_column("spiral", &gallery::column(spiral))?;
        let mut chart = gallery::chart("Spiral, arc interpolation", "Angle", "Radius", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, 5.0);
        let mut dashed = gallery::line(Color::from_rgb8(40, 150, 90), 3.0);
        dashed.line_style = LineStylePreset::Dash;
        let series = vec![gallery::series(
            "spiral",
            "spiral_deg",
            "spiral",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(gallery::scatter(Color::from_rgb8(40, 150, 90), 6.0)),
                line: Some(dashed),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                interpolation: PolarInterpolation::Arc { pieces: 16 },
                ..PolarOptions::default()
            }));
            config.bottom_x.major_spacing = 45.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 1.0;
        }
        gallery::save(r, &chart, &series, &out.join("03-spiral-arcs.png"))?;

        // 4) A half-circle sector in radians with a hole in the middle: a
        //    phase response over -π/2..π/2, labelled in multiples of π.
        let phase: Vec<f64> = (0..=60)
            .map(|i| -PI / 2.0 + PI * f64::from(i) / 60.0)
            .collect();
        let response: Vec<f64> = phase
            .iter()
            .map(|p| 1.0 + 0.8 * (2.0 * p).cos().powi(2) + 0.2 * (6.0 * p).sin())
            .collect();
        r.add_column("phase", &gallery::column(phase))?;
        r.add_column("response", &gallery::column(response))?;
        let mut chart = gallery::chart("Sector in radians", "Phase", "Response", size);
        chart.set_x_range(-PI / 2.0, PI / 2.0);
        chart.set_y_range(0.0, 2.0);
        let mut line = gallery::line(Color::from_rgb8(150, 60, 170), 3.0);
        line.fill = Some(LineFill::ToBaseline(0.0));
        line.fill_color = Some(Color::new(0.59, 0.24, 0.67, 0.2));
        let series = vec![gallery::series(
            "response",
            "phase",
            "response",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(line),
                errors: None,
                stem: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                angle_unit: AngleUnit::Radians,
                sector: Some([-PI / 2.0, PI / 2.0]),
                inner_radius_fraction: 0.2,
                radial_label_angle_degrees: 90.0,
                ..PolarOptions::default()
            }));
            config.bottom_x.major_spacing = PI / 4.0;
            config.bottom_x.label_style.format = LabelFormat::Radians;
            config.left_y.major_spacing = 0.5;
        }
        gallery::save(r, &chart, &series, &out.join("04-sector-radians.png"))?;

        // 5) A heatmap on the polar chart (#208): an antenna's gain in dBi
        //    over azimuth (5° cells) and frequency (rings from 1 to 6 GHz),
        //    the main lobe narrowing as the frequency rises, with labelled
        //    contours along the -3 dB and -10 dB lines.
        let azimuths = 72usize;
        let bands = 25usize;
        // Sample centres every 5°, and the first again one turn on: naming
        // its column twice closes the interpolated seam at 0°.
        let centres: Vec<f64> = (0..=azimuths).map(|i| i as f64 * 5.0 + 2.5).collect();
        let frequencies: Vec<f64> = (0..bands)
            .map(|j| 1.0 + j as f64 * 5.0 / (bands - 1) as f64)
            .collect();
        r.add_column("gain_az", &gallery::column(centres))?;
        r.add_column("gain_ghz", &gallery::column(frequencies.clone()))?;
        let mut ids: Vec<String> = (0..azimuths).map(|i| format!("gain_z{i}")).collect();
        for (i, id) in ids.iter().enumerate() {
            let az = (i as f64 * 5.0 + 2.5).to_radians();
            let gain: Vec<f64> = frequencies
                .iter()
                .map(|f| {
                    // A cosine-power main lobe whose power grows with the
                    // frequency, a back lobe 20 dB down and a floor.
                    let lobe = (0.5 * (1.0 + az.cos())).powf(1.5 * f);
                    let back = 0.01 * (0.5 * (1.0 - az.cos())).powi(4);
                    10.0 * (lobe + back + 1e-3).log10() + 6.0
                })
                .collect();
            r.add_column(id, &gallery::column(gain))?;
        }
        ids.push(ids[0].clone());
        let mut chart = gallery::chart("Gain over azimuth and frequency", "Azimuth", "GHz", size);
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(1.0, 6.0);
        let series = vec![gallery::series(
            "gain",
            "gain_az",
            "gain_ghz",
            DataRenderType::Field(renderer::data_config::FieldSeries {
                matrix: renderer::data_config::MatrixRef {
                    columns: ids,
                    orientation: renderer::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: renderer::data_config::GridLayout::Centers,
                },
                fill: Some(renderer::data_config::FieldFillConfig {
                    mode: renderer::data_config::FillMode::Continuous,
                    shading: renderer::data_config::Shading::Interpolated,
                    opacity: 1.0,
                }),
                contour: Some(renderer::data_config::ContourConfig {
                    levels: vec![-4.0, 3.0],
                    line: gallery::line(Color::from_rgb8(25, 25, 25), 1.2),
                    per_level_color: None,
                    labels: Some(renderer::data_config::ContourLabelConfig {
                        visible: true,
                        font_size: 12.0,
                        color: Color::from_rgb8(25, 25, 25),
                        format: LabelFormat::Decimal,
                        significant_digits: 1,
                        spacing_px: 180.0,
                        anchors: Vec::new(),
                        bg_color: None,
                        bg_padding_px: 2.0,
                    }),
                }),
                surface: None,
            }),
        )];
        {
            let config = chart.config_mut();
            config.set_polar(Some(PolarOptions {
                zero_angle_degrees: 90.0,
                clockwise: true,
                inner_radius_fraction: 0.08,
                radial_label_angle_degrees: 90.0,
                ..PolarOptions::default()
            }));
            config.bottom_x.major_spacing = 30.0;
            config.bottom_x.label_style.format = LabelFormat::Degrees;
            config.left_y.major_spacing = 1.0;
            config.grid.show_major_x = false;
            config.grid.show_major_y = false;
            let mut bar = renderer::default::default_colorbar_options();
            bar.axis.min = -24.0;
            bar.axis.max = 6.0;
            bar.axis.major_spacing = 6.0;
            config.colorbar = Some(bar);
        }
        gallery::save(r, &chart, &series, &out.join("05-gain-heatmap.png"))?;
        wave_spectrum(r, &out)?;
        wind_rose(r, &out)
    })
}

/// A deterministic hash in -1..1 for the synthetic measurement noise.
fn noise(seed: u64, i: usize) -> f64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64) * 2.0 - 1.0
}

/// The compass points of a categorical angle axis, clockwise from north.
const COMPASS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW",
    "NNW",
];

// A directional wave spectrum from an offshore buoy: energy over the
// direction the waves come from (around) and their frequency (outward), as
// flat annular cells with labelled contours. A long swell from the
// south-west, a younger wind sea from the north-east and a weak secondary
// swell from the south-east. The host computes the spectrum in decibels
// below its peak; the renderer maps and outlines the supplied cells.
fn wave_spectrum(r: &mut Renderer, out: &Path) -> gallery::Result {
    let bg = rgb(249, 250, 252);
    let ink = rgb(44, 56, 78);
    let grid = rgb(214, 222, 232);
    let mut chart = gallery::chart(
        "Directional wave spectrum / rings in Hz",
        "Direction",
        "Frequency (Hz)",
        styled::SIZE,
    );
    styled::chrome(chart.config_mut(), bg, ink, grid);
    chart.config_mut().legend.visible = false;
    // The compass labels and the ring labels name the two coordinates.
    chart.config_mut().bottom_x.title_option.visible = false;
    chart.config_mut().left_y.title_option.visible = false;

    // Eight compass points; the angle column holds category units, one
    // per 45°, so a cell boundary every 7.5° is a step of 1/6.
    let directions = 48usize;
    let rings = 21usize;
    let step = 8.0 / directions as f64;
    // One more boundary and the first column again past the turn: the
    // contours close across north.
    let angle_edges: Vec<f64> = (0..=directions + 1).map(|i| i as f64 * step).collect();
    let frequency_edges: Vec<f64> = (0..=rings).map(|j| 0.04 + j as f64 * 0.01).collect();
    let system = |f: f64, from_deg: f64, fp: f64, spread: f64, theta_deg: f64| {
        // A JONSWAP-like frequency shape normalised to its peak, times a
        // cos^2s spreading about the mean direction.
        let shape = |f: f64| {
            let sigma = if f <= fp { 0.07 } else { 0.09 };
            let peak = 3.3f64.powf((-(f - fp).powi(2) / (2.0 * sigma * sigma * fp * fp)).exp());
            f.powi(-5) * (-1.25 * (fp / f).powi(4)).exp() * peak
        };
        let half = (theta_deg - from_deg).to_radians() / 2.0;
        shape(f) / shape(fp) * half.cos().powi(2).powf(spread)
    };
    let mut energy = Vec::with_capacity(directions * rings);
    for i in 0..directions {
        let theta = (i as f64 + 0.5) * 360.0 / directions as f64;
        for j in 0..rings {
            let f = 0.045 + j as f64 * 0.01;
            let e = 1.0 * system(f, 235.0, 0.07, 24.0, theta)
                + 0.45 * system(f, 62.0, 0.15, 10.0, theta)
                + 0.2 * system(f, 150.0, 0.1, 16.0, theta);
            energy.push(e * (1.0 + 0.05 * noise(17, i * rings + j)));
        }
    }
    let peak = energy.iter().copied().fold(0.0f64, f64::max);
    r.add_column("wave_dir", &gallery::column(angle_edges))?;
    r.add_column("wave_freq", &gallery::column(frequency_edges))?;
    let mut ids: Vec<String> = (0..directions).map(|i| format!("wave_e{i}")).collect();
    for (i, id) in ids.iter().enumerate() {
        let db: Vec<f64> = energy[i * rings..(i + 1) * rings]
            .iter()
            .map(|e| {
                let db = 10.0 * (e / peak).log10();
                // Below the buoy's noise floor nothing is reported.
                if db < -30.0 { f64::NAN } else { db }
            })
            .collect();
        r.add_column(id, &gallery::column(db))?;
    }
    ids.push(ids[0].clone());
    let series = vec![gallery::series(
        "spectrum",
        "wave_dir",
        "wave_freq",
        DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: ids,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Edges,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: Some(ContourConfig {
                levels: vec![-20.0, -10.0, -3.0],
                line: gallery::line(rgba(20, 28, 48, 0.85), 1.4),
                per_level_color: None,
                labels: Some(ContourLabelConfig {
                    visible: true,
                    font_size: 15.0,
                    color: rgb(20, 28, 48),
                    format: LabelFormat::Decimal,
                    significant_digits: 2,
                    spacing_px: 260.0,
                    anchors: Vec::new(),
                    bg_color: None,
                    bg_padding_px: 3.0,
                }),
            }),
            surface: None,
        }),
    )];
    chart.set_x_range(0.0, 8.0);
    chart.set_y_range(0.04, 0.25);
    {
        let config = chart.config_mut();
        config.bottom_x.categories = Some(
            COMPASS
                .iter()
                .step_by(2)
                .map(|d| Category::new(d.to_lowercase(), *d))
                .collect(),
        );
        config.set_polar(Some(PolarOptions {
            zero_angle_degrees: 90.0,
            clockwise: true,
            inner_radius_fraction: 0.1,
            // Up the empty north-north-west sector.
            radial_label_angle_degrees: 112.5,
            ..PolarOptions::default()
        }));
        config.left_y.major_spacing = 0.05;
        config.left_y.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        colorbar(
            config,
            ColorMap::Custom {
                stops: vec![
                    rgb(237, 248, 217),
                    rgb(199, 233, 180),
                    rgb(127, 205, 187),
                    rgb(65, 182, 196),
                    rgb(29, 145, 192),
                    rgb(34, 94, 168),
                    rgb(12, 44, 132),
                ],
            },
            -30.0,
            0.0,
            5.0,
            ink,
        );
        let bar = config.colorbar.as_mut().unwrap();
        bar.nan_color = Color::new(0.0, 0.0, 0.0, 0.0);
        // Closer to the disc than the plot area's right edge.
        bar.offset_x = -110.0;
        bar.axis.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
        bar.axis.title_option.visible = true;
        bar.axis.title_option.text = RichText::plain("Energy (dB)", ink, 19.0, "");
        // Clear of the tick labels: the title's local y runs away from the
        // strip on a right-hand bar.
        bar.axis.title_option.offset_y = -28.0;
    }
    let image = export(r, &chart, &series, bg)?;
    save(&image, bg, &out.join("gallery-polar-heatmap.png"))
}

// A wind rose: a year of hourly winds as the share of time from each of
// sixteen directions, stacked outward by speed band. Category bars on a
// categorical angle axis; the host counts the shares.
fn wind_rose(r: &mut Renderer, out: &Path) -> gallery::Result {
    let bg = rgb(252, 249, 244);
    let ink = rgb(70, 60, 56);
    let grid = rgb(229, 221, 209);
    let mut chart = gallery::chart(
        "Wind rose / share of a year's hours",
        "Direction",
        "Share of hours",
        styled::SIZE,
    );
    styled::chrome(chart.config_mut(), bg, ink, grid);
    chart.config_mut().bottom_x.title_option.visible = false;
    chart.config_mut().left_y.title_option.visible = false;

    // How often the wind blows from direction k: a prevailing
    // west-south-westerly and a secondary north-easterly. The prevailing
    // winds are also the strong ones.
    let shape = |k: usize| {
        let a = k as f64 * std::f64::consts::TAU / 16.0;
        let prevailing = (1.0 + (a - 4.32).cos()).powi(3);
        let secondary = 0.3 * (1.0 + (a - 0.79).cos()).powi(4);
        0.18 + prevailing + secondary
    };
    let total: f64 = (0..16).map(shape).sum();
    let bands = [
        ("rose_light", "Under 4 m/s", rgb(253, 212, 140)),
        ("rose_moderate", "4 to 8 m/s", rgb(250, 152, 72)),
        ("rose_fresh", "8 to 12 m/s", rgb(222, 88, 44)),
        ("rose_strong", "Over 12 m/s", rgb(140, 36, 30)),
    ];
    let share = |k: usize, band: usize| {
        let a = k as f64 * std::f64::consts::TAU / 16.0;
        let strength = 0.5 * (1.0 + (a - 4.32).cos());
        let weights = [
            0.46 - 0.16 * strength,
            0.33,
            0.15 + 0.09 * strength,
            0.06 + 0.07 * strength,
        ];
        let sum: f64 = weights.iter().sum();
        let jitter = 1.0 + 0.1 * noise(31 + band as u64, k);
        // A fraction of the year's hours, to 0.1 %.
        (1000.0 * shape(k) / total * weights[band] / sum * jitter).round() / 1000.0
    };
    r.add_column(
        "rose_dir",
        &gallery::column((0..16).map(|k| k as f64).collect()),
    )?;
    let mut series = Vec::with_capacity(bands.len());
    for (band, (id, label, color)) in bands.into_iter().enumerate() {
        r.add_column(
            id,
            &gallery::column((0..16).map(|k| share(k, band)).collect()),
        )?;
        let mut s = gallery::series(
            id,
            "rose_dir",
            id,
            DataRenderType::CategoryBars {
                color,
                style: CategoryBarStyle {
                    corner_radius: 3.0,
                    gloss: 0.0,
                    texture_strength: 0.0,
                    ..CategoryBarStyle::default()
                },
                labels: Default::default(),
                label_format: Default::default(),
                overrides: None,
            },
        );
        s.label = Some(RichText::plain(label, ink, 22.0, ""));
        append_legend_entry(
            &mut chart.config_mut().legend.content,
            series_symbol_segments(&s, None),
            label,
        );
        series.push(s);
    }
    chart.set_x_range(0.0, 16.0);
    chart.set_y_range(0.0, 0.12);
    {
        let config = chart.config_mut();
        config.bottom_x.categories = Some(
            COMPASS
                .iter()
                .map(|d| Category::new(d.to_lowercase(), *d))
                .collect(),
        );
        config.set_polar(Some(PolarOptions {
            zero_angle_degrees: 90.0,
            clockwise: true,
            // A calm circle at the centre.
            inner_radius_fraction: 0.07,
            // Along the south-south-east spoke: the labels sit on its left,
            // over the quiet south-east.
            radial_label_angle_degrees: -67.5,
            ..PolarOptions::default()
        }));
        config.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Stacked,
            group_width: 0.86,
            ..BarGroups::default()
        });
        config.left_y.major_spacing = 0.03;
        config.left_y.label_style.format = LabelFormat::Percent;
        config.left_y.label_style.significant_digits = 2;
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        config.legend.visible = true;
    }
    let image = export(r, &chart, &series, bg)?;
    save(&image, bg, &out.join("gallery-rose.png"))
}
