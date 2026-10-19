//! Eight designed scenes for the README gallery, one per Cartesian feature
//! that the feature examples show plainly: a wind field over a heatmap,
//! ocean currents coloured by speed, a colour-mapped scatter, a bubble
//! chart, a forecast with uncertainty bands, a thermostat schedule as step
//! lines, twin axes and small multiples. Existing public APIs only; all data
//! is synthetic. PNGs are rendered at 1200 x 800 without post-processing.
//! cargo run --locked -p figgy-renderer --example gallery_showcase -- target/showcase

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::{Result, column, line, scatter, series};
use renderer::{
    CellRef, Chart, Color, ColorMap, Config, OverlayLayer, RasterImage, Renderer, RendererDevice,
    Subplot,
    config::{AxisScale, LegendCorner, LegendEntryKind, TickVisibility},
    data_config::{
        DataRenderType, FieldFillConfig, FillMode, GridLayout, LineFill, LineInterpolation,
        MatrixOrientation, MatrixRef, PointColorMap, PointSizeMap, Shading, SizeScale,
        VectorColorMap, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    },
    data_render::{create_instance, request_adapter, request_device},
    encode_png,
    layout::{ChartArea, Rect, Side},
    line::LineStylePreset,
    text::RichText,
};
use std::{path::Path, sync::Arc};

const SIZE: (u32, u32) = (1200, 800);
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba(
        f32::from(r) / 255.,
        f32::from(g) / 255.,
        f32::from(b) / 255.,
        a,
    )
}
fn extent(values: &[f64]) -> (f64, f64) {
    values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        })
}

/// Clear color is part of each scene, including the area behind axes and labels.
fn save(image: &RasterImage, background: Color, path: &Path) -> Result {
    assert_eq!((image.width, image.height), SIZE);
    assert!(image.rgba.chunks_exact(4).all(|p| p[3] == 255));
    let bg = [background.r, background.g, background.b].map(|c| (c * 255.).round() as u8);
    let colored = image
        .rgba
        .chunks_exact(4)
        .filter(|p| {
            let distance: u16 = (0..3).map(|i| p[i].abs_diff(bg[i]) as u16).sum();
            distance > 80 && p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40
        })
        .count();
    assert!(colored > 2000, "missing chart colors: {colored}");
    std::fs::write(path, encode_png(image)?)?;
    println!("{}: {colored} colored foreground pixels", path.display());
    Ok(())
}

/// The shared chrome: no axis lines or ticks, horizontal grid only, a quiet
/// legend in the background colour.
fn chrome(config: &mut Config, bg: Color, ink: Color, grid: Color) {
    config.chart_title.text.font_size = 34.;
    config.chart_title.text.color = ink;
    config.chart_title.top_margin = 85.;
    config.left_y.out_margin = 110.;
    config.right_y.out_margin = 65.;
    config.bottom_x.out_margin = 95.;
    config.top_x.out_margin = 45.;
    config.grid.major_y_color = grid;
    config.grid.major_x_color = grid;
    config.grid.show_major_x = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for a in [
        &mut config.left_y,
        &mut config.right_y,
        &mut config.bottom_x,
        &mut config.top_x,
    ] {
        a.line_visible = false;
        a.tick = TickVisibility::None;
        a.label_style.color = ink;
        a.label_style.font_size = 21.;
        a.title_option.text.color = ink;
        a.title_option.text.font_size = 23.;
    }
    config.legend.visible = true;
    config.legend.corner = LegendCorner::TopLeft;
    config.legend.bg_color = bg;
    config.legend.border_color = bg;
    config.legend.content.color = ink;
    config.legend.content.font_size = 22.;
    config.legend.padding = 15.;
}

fn cartesian(title: &str, x: &str, y: &str, bg: Color, ink: Color, grid: Color) -> Chart {
    let mut chart = gallery::chart(title, x, y, SIZE);
    chrome(chart.config_mut(), bg, ink, grid);
    chart
}

fn colorbar(config: &mut Config, ramp: ColorMap, min: f64, max: f64, spacing: f64, ink: Color) {
    let mut bar = renderer::default::default_colorbar_options();
    bar.side = Side::Right;
    bar.colormap = ramp;
    bar.axis.scale = AxisScale::Linear;
    bar.axis.min = min;
    bar.axis.max = max;
    bar.axis.major_spacing = spacing;
    bar.axis.label_style.color = ink;
    bar.axis.label_style.font_size = 18.;
    bar.axis.tick = TickVisibility::None;
    bar.border_color = Color::from_rgba(ink.r, ink.g, ink.b, 0.25);
    bar.border_width = 1.;
    bar.thickness_px = 16.;
    bar.length_frac = 0.6;
    config.colorbar = Some(bar);
}

fn export(
    r: &mut Renderer,
    chart: &Chart,
    specs: &[renderer::SeriesConfig],
    bg: Color,
) -> Result<RasterImage> {
    Ok(pollster::block_on(
        r.export_panel_rgba_with_clear_async(chart, specs, 1., bg),
    )?)
}

// 1. A sea breeze over a bay: wind speed as a smooth field, direction as
//    arrows on a coarser grid of the same coordinate space.
fn wind(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(14, 22, 38);
    let ink = rgb(182, 199, 222);
    let mut chart = cartesian(
        "Sea breeze / 15:00 local",
        "East (km)",
        "North (km)",
        bg,
        ink,
        bg,
    );
    chart.set_x_range(0., 12.);
    chart.set_y_range(0., 8.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_y = false;
    let field = |x: f64, y: f64| -> (f64, f64) {
        // A vortex off the headland, a second weaker one inland and a
        // steady onshore drift.
        let eddy = |cx: f64, cy: f64, strength: f64, width: f64| {
            let (dx, dy) = (x - cx, y - cy);
            let r2 = dx * dx + dy * dy;
            let gain = strength / (1. + r2 / width);
            (-dy * gain, dx * gain)
        };
        let (u1, v1) = eddy(4., 5.2, 1.4, 2.2);
        let (u2, v2) = eddy(8.6, 2.6, -0.9, 1.6);
        (u1 + u2 + 0.9, v1 + v2 + 0.35)
    };
    let axis = |n: usize, span: f64| -> Vec<f64> {
        (0..n).map(|i| span * i as f64 / (n - 1) as f64).collect()
    };
    let (fx, fy) = (axis(73, 12.), axis(49, 8.));
    r.add_column("wx", &column(fx.clone()))?;
    r.add_column("wy", &column(fy.clone()))?;
    let mut speed_ids = Vec::new();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (i, x) in fx.iter().enumerate() {
        let id = format!("wind-speed-{i}");
        let speed: Vec<f64> = fy
            .iter()
            .map(|y| {
                let (u, v) = field(*x, *y);
                u.hypot(v)
            })
            .collect();
        let (a, b) = extent(&speed);
        lo = lo.min(a);
        hi = hi.max(b);
        r.add_column(id.clone(), &column(speed))?;
        speed_ids.push(id);
    }
    let (ax, ay) = (axis(25, 12.), axis(17, 8.));
    r.add_column("ax", &column(ax.clone()))?;
    r.add_column("ay", &column(ay.clone()))?;
    let (mut u_ids, mut v_ids) = (Vec::new(), Vec::new());
    for (i, x) in ax.iter().enumerate() {
        let u: Vec<f64> = ay.iter().map(|y| field(*x, *y).0).collect();
        let v: Vec<f64> = ay.iter().map(|y| field(*x, *y).1).collect();
        let (uid, vid) = (format!("wind-u-{i}"), format!("wind-v-{i}"));
        r.add_column(uid.clone(), &column(u))?;
        r.add_column(vid.clone(), &column(v))?;
        u_ids.push(uid);
        v_ids.push(vid);
    }
    let grid = |columns: &Vec<String>| MatrixRef {
        columns: columns.clone(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    };
    let specs = [
        series(
            "speed",
            "wx",
            "wy",
            DataRenderType::Heatmap {
                matrix: grid(&speed_ids),
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Interpolated,
                    opacity: 1.,
                },
            },
        ),
        series(
            "wind",
            "ax",
            "ay",
            DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: grid(&u_ids),
                    v: grid(&v_ids),
                },
                style: VectorStyle {
                    color: rgba(240, 246, 255, 0.9),
                    width_px: 1.8,
                    scale: VectorScale::Pixels { px_per_unit: 16. },
                    pivot: VectorPivot::Middle,
                    head_length_px: 7.,
                    head_width_px: 6.,
                    min_length_px: 3.,
                    color_map: None,
                },
            },
        ),
    ];
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                bg,
                rgb(25, 62, 96),
                rgb(33, 122, 142),
                rgb(72, 190, 170),
                rgb(214, 240, 196),
            ],
        },
        0.,
        (hi * 2.).round() / 2.,
        0.5,
        ink,
    );
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 2.;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("showcase-wind.png"))
}

// 2. Surface currents: arrows at scattered buoys, length and colour by speed.
fn currents(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(9, 30, 40);
    let ink = rgb(170, 205, 205);
    let mut chart = cartesian(
        "Surface current / drifter array",
        "Longitude",
        "Latitude",
        bg,
        ink,
        rgb(19, 47, 58),
    );
    chart.set_x_range(0., 10.);
    chart.set_y_range(0., 6.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    let (mut x, mut y, mut u, mut v, mut speed) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (nx, ny) = (26u32, 16u32);
    for j in 0..ny {
        for i in 0..nx {
            // A deterministic jitter keeps the buoys off a strict lattice.
            let h = f64::from(i * 7 + j * 13) * 0.618;
            let jx = (h.sin() * 43758.5453).fract() - 0.5;
            let jy = (h.cos() * 43758.5453).fract() - 0.5;
            let px = 0.3 + 9.4 * f64::from(i) / f64::from(nx - 1) + 0.18 * jx;
            let py = 0.3 + 5.4 * f64::from(j) / f64::from(ny - 1) + 0.18 * jy;
            // A meandering jet with slower return flow on either side.
            let centre = 3. + 1.1 * (px * 0.9).sin();
            let d = (py - centre) / 0.9;
            let jet = (-d * d).exp();
            let cu = 0.25 + 1.3 * jet;
            let cv = 1.0 * (px * 0.9).cos() * jet - 0.15 * d * (1. - jet);
            x.push(px);
            y.push(py);
            u.push(cu);
            v.push(cv);
            speed.push(cu.hypot(cv));
        }
    }
    let (lo, hi) = extent(&speed);
    for (id, data) in [
        ("cx", x),
        ("cy", y),
        ("cu", u.clone()),
        ("cv", v.clone()),
        ("cspeed", speed),
    ] {
        r.add_column(id, &column(data))?;
    }
    let mut style = VectorStyle {
        color: ink,
        width_px: 2.2,
        scale: VectorScale::Pixels { px_per_unit: 1. },
        pivot: VectorPivot::Middle,
        head_length_px: 8.,
        head_width_px: 7.,
        min_length_px: 0.,
        color_map: Some(VectorColorMap::Column("cspeed".into())),
    };
    style.fit_scale(extent(&u), extent(&v), 44.);
    let specs = [series(
        "current",
        "cx",
        "cy",
        DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "cu".into(),
                v: "cv".into(),
            },
            style,
        },
    )];
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(36, 92, 120),
                rgb(70, 170, 160),
                rgb(173, 226, 150),
                rgb(250, 230, 120),
            ],
        },
        (lo * 10.).floor() / 10.,
        (hi * 10.).ceil() / 10.,
        0.4,
        ink,
    );
    chart.config_mut().bottom_x.major_spacing = 2.;
    chart.config_mut().left_y.major_spacing = 1.;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("showcase-currents.png"))
}

// 3. A colour-mapped scatter: samples along two spiral arms, coloured by a
//    third column through the colourbar.
fn colormap_scatter(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(250, 250, 252);
    let ink = rgb(52, 57, 85);
    let mut chart = cartesian(
        "Particle ages / two arms",
        "x (kpc)",
        "y (kpc)",
        bg,
        ink,
        rgb(232, 234, 242),
    );
    chart.set_x_range(-10., 10.);
    chart.set_y_range(-7., 7.);
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_x = true;
    let (mut x, mut y, mut age) = (Vec::new(), Vec::new(), Vec::new());
    let n = 1400u32;
    for i in 0..n {
        let t = f64::from(i) / f64::from(n - 1);
        let h = f64::from(i) * 0.618;
        let j1 = (h.sin() * 43758.5453).fract() - 0.5;
        let j2 = (h.cos() * 43758.5453).fract() - 0.5;
        let arm = if i % 2 == 0 { 0. } else { std::f64::consts::PI };
        let theta = arm + 3.6 * t;
        let radius = 0.6 + 8.6 * t;
        let spread = 0.25 + 0.9 * t;
        x.push(radius * theta.cos() + spread * j1 * 2.);
        y.push(0.7 * radius * theta.sin() + spread * j2 * 1.4);
        age.push(100. * (1. - t) + 14. * j1);
    }
    for (id, data) in [("sx", x), ("sy", y), ("sage", age)] {
        r.add_column(id, &column(data))?;
    }
    let mut points = scatter(ink, 5.5);
    points.point_color_map = Some(PointColorMap {
        column: "sage".into(),
    });
    let specs = [series(
        "stars",
        "sx",
        "sy",
        DataRenderType::Scatter { scatter: points },
    )];
    colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(244, 190, 72),
                rgb(238, 110, 92),
                rgb(170, 70, 160),
                rgb(70, 60, 160),
                rgb(24, 32, 96),
            ],
        },
        0.,
        100.,
        25.,
        ink,
    );
    chart
        .config_mut()
        .colorbar
        .as_mut()
        .unwrap()
        .axis
        .title_option
        .visible = true;
    chart
        .config_mut()
        .colorbar
        .as_mut()
        .unwrap()
        .axis
        .title_option
        .text = RichText::plain("Age (Myr)", ink, 19., "");
    chart.config_mut().bottom_x.major_spacing = 5.;
    chart.config_mut().left_y.major_spacing = 3.5;
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("showcase-colormap.png"))
}

// 4. A bubble chart: three regions, marker area by population over one
//    shared domain, translucent fills so overlaps stay readable.
fn bubbles(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(252, 249, 244);
    let ink = rgb(70, 60, 56);
    let mut chart = cartesian(
        "Income and life expectancy / 54 cities",
        "Income per person (USD thousands)",
        "Life expectancy (years)",
        bg,
        ink,
        rgb(235, 228, 216),
    );
    chart.set_x_range(0., 70.);
    chart.set_y_range(58., 86.);
    chart.config_mut().legend.corner = LegendCorner::BottomRight;
    chart.config_mut().bottom_x.major_spacing = 10.;
    chart.config_mut().left_y.major_spacing = 4.;
    let mut specs = Vec::new();
    for (k, (id, label, color, x0, slope, pop)) in [
        ("east", "East", rgb(222, 101, 83), 8., 0.42, 0.55),
        ("north", "North", rgb(45, 140, 135), 28., 0.24, 0.35),
        ("south", "South", rgb(118, 92, 196), 16., 0.33, 0.8),
    ]
    .into_iter()
    .enumerate()
    {
        let (mut x, mut y, mut p) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..18u32 {
            let h = f64::from(i * 31 + k as u32 * 97) * 0.618;
            let j1 = (h.sin() * 43758.5453).fract() - 0.5;
            let j2 = (h.cos() * 43758.5453).fract() - 0.5;
            let j3 = ((h * 1.7).sin() * 43758.5453).fract();
            let income = x0 + 30. * (f64::from(i) / 17.) + 7. * j1;
            x.push(income);
            y.push(60. + slope * income + 20. * (1. - (-income / 25.).exp()) * 0.4 + 2.5 * j2);
            p.push(0.4 + 9.6 * j3 * j3 * pop);
        }
        for (suffix, data) in [("x", x), ("y", y), ("pop", p)] {
            r.add_column(format!("{id}-{suffix}"), &column(data))?;
        }
        let mut marker = scatter(Color::from_rgba(color.r, color.g, color.b, 0.62), 8.);
        let mut size = PointSizeMap {
            column: format!("{id}-pop"),
            domain_min: 0.,
            domain_max: 1.,
            min_px: 5.,
            max_px: 34.,
            scale: SizeScale::Area,
        };
        size.fit_domain(0.4, 10.);
        marker.point_size_map = Some(size);
        specs.push(series(
            id,
            &format!("{id}-x"),
            &format!("{id}-y"),
            DataRenderType::Scatter { scatter: marker },
        ));
        chart = chart.with_legend_entry(label, color, 2., LegendEntryKind::Scatter);
    }
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("showcase-bubbles.png"))
}

// 5. A forecast: the history as a solid line, the forecast dashed, with 80%
//    and 95% bands filled between their edge series.
fn forecast(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(247, 248, 253);
    let ink = rgb(52, 57, 85);
    let teal = rgb(37, 150, 160);
    let mut chart = cartesian(
        "Demand forecast / next three weeks",
        "Day",
        "Orders per day",
        bg,
        ink,
        rgb(228, 231, 240),
    );
    chart.set_x_range(0., 49.);
    chart.set_y_range(0., 1600.);
    chart.config_mut().bottom_x.major_spacing = 7.;
    chart.config_mut().left_y.major_spacing = 400.;
    let day: Vec<f64> = (0..=49).map(f64::from).collect();
    let trend = |d: f64| 520. + 11. * d + 160. * ((d - 3.) / 7. * std::f64::consts::TAU).sin();
    let history: Vec<f64> = day
        .iter()
        .map(|d| {
            if *d <= 28. {
                trend(*d) + 35. * (d * 1.9).sin() + 22. * (d * 4.3).cos()
            } else {
                f64::NAN
            }
        })
        .collect();
    let forecast: Vec<f64> = day
        .iter()
        .map(|d| if *d >= 28. { trend(*d) } else { f64::NAN })
        .collect();
    let band = |width: f64| -> (Vec<f64>, Vec<f64>) {
        let upper = day
            .iter()
            .map(|d| {
                if *d >= 28. {
                    trend(*d) + width * (d - 28. + 2.).sqrt()
                } else {
                    f64::NAN
                }
            })
            .collect();
        let lower = day
            .iter()
            .map(|d| {
                if *d >= 28. {
                    trend(*d) - width * (d - 28. + 2.).sqrt()
                } else {
                    f64::NAN
                }
            })
            .collect();
        (upper, lower)
    };
    let (up80, lo80) = band(28.);
    let (up95, lo95) = band(52.);
    for (id, data) in [
        ("day", day),
        ("history", history),
        ("forecast", forecast),
        ("up80", up80),
        ("lo80", lo80),
        ("up95", up95),
        ("lo95", lo95),
    ] {
        r.add_column(id, &column(data))?;
    }
    let edge = |id: &str, to: Option<&str>, alpha: f32| {
        let mut style = line(Color::from_rgba(teal.r, teal.g, teal.b, 0.0), 0.5);
        style.fill = to.map(|s| LineFill::ToSeries(s.into()));
        style.fill_color = Some(Color::from_rgba(teal.r, teal.g, teal.b, alpha));
        series(id, "day", id, DataRenderType::Line { line: style })
    };
    let mut dashed = line(teal, 3.);
    dashed.line_style = LineStylePreset::Dash;
    let specs = [
        edge("up95", Some("lo95"), 0.14),
        edge("lo95", None, 0.),
        edge("up80", Some("lo80"), 0.22),
        edge("lo80", None, 0.),
        series(
            "history",
            "day",
            "history",
            DataRenderType::Line {
                line: line(ink, 3.),
            },
        ),
        series(
            "forecast",
            "day",
            "forecast",
            DataRenderType::Line { line: dashed },
        ),
    ];
    chart = chart
        .with_legend_entry("Actual", ink, 3., LegendEntryKind::Line)
        .with_legend_entry(
            "Forecast, 80% and 95% bands",
            teal,
            3.,
            LegendEntryKind::Line,
        );
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("showcase-forecast.png"))
}

// 6. A thermostat schedule: the setpoint as a step line with a filled area,
//    the measured room temperature as a smooth line over it.
fn thermostat(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(248, 247, 252);
    let ink = rgb(58, 54, 82);
    let violet = rgb(112, 92, 212);
    let coral = rgb(232, 104, 84);
    let mut chart = cartesian(
        "Thermostat / a winter day",
        "Hour",
        "Temperature (°C)",
        bg,
        ink,
        rgb(230, 228, 240),
    );
    chart.set_x_range(0., 24.);
    chart.set_y_range(14., 24.);
    chart.config_mut().bottom_x.major_spacing = 3.;
    chart.config_mut().left_y.major_spacing = 2.;
    let hours: Vec<f64> = (0..=24).map(f64::from).collect();
    let setpoint: Vec<f64> = hours
        .iter()
        .map(|h| match *h as u32 {
            0..=5 => 16.,
            6..=8 => 21.,
            9..=16 => 18.5,
            17..=21 => 21.5,
            _ => 17.,
        })
        .collect();
    let minute: Vec<f64> = (0..=288).map(|i| f64::from(i) / 12.).collect();
    let room: Vec<f64> = minute
        .iter()
        .map(|t| {
            // The room lags the setpoint with a soft first-order response.
            let target = |h: f64| match h as u32 {
                0..=5 => 16.,
                6..=8 => 21.,
                9..=16 => 18.5,
                17..=21 => 21.5,
                _ => 17.,
            };
            let mut temp = 16.4;
            let mut h = 0.;
            while h < *t {
                temp += (target(h) - temp) * 0.045 + 0.02 * (h * 5.).sin();
                h += 1. / 12.;
            }
            temp
        })
        .collect();
    r.add_column("hour", &column(hours))?;
    r.add_column("setpoint", &column(setpoint))?;
    r.add_column("minute", &column(minute))?;
    r.add_column("room", &column(room))?;
    let mut step = line(violet, 2.5);
    step.interpolation = LineInterpolation::StepPost;
    step.fill = Some(LineFill::ToBaseline(14.));
    step.fill_color = Some(Color::from_rgba(violet.r, violet.g, violet.b, 0.16));
    let specs = [
        series(
            "setpoint",
            "hour",
            "setpoint",
            DataRenderType::Line { line: step },
        ),
        series(
            "room",
            "minute",
            "room",
            DataRenderType::Line {
                line: line(coral, 3.5),
            },
        ),
    ];
    chart = chart
        .with_legend_entry("Setpoint schedule", violet, 2.5, LegendEntryKind::Line)
        .with_legend_entry("Room temperature", coral, 3.5, LegendEntryKind::Line);
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("showcase-thermostat.png"))
}

// 7. Twin axes: a battery discharge with the voltage on the left axis and the
//    current on an overlay layer's right axis.
fn twin_axes(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(18, 20, 34);
    let ink = rgb(190, 196, 224);
    let mint = rgb(80, 220, 190);
    let lavender = rgb(150, 160, 255);
    let mut chart = cartesian(
        "Battery discharge / one cycle",
        "Minutes",
        "Cell voltage (V)",
        bg,
        ink,
        rgb(38, 42, 66),
    );
    chart.set_x_range(0., 90.);
    chart.set_y_range(3.0, 4.3);
    chart.config_mut().bottom_x.major_spacing = 15.;
    chart.config_mut().left_y.major_spacing = 0.25;
    chart.config_mut().right_y.line_visible = false;
    chart.config_mut().right_y.tick = TickVisibility::None;
    chart.config_mut().left_y.title_option.text.color = mint;
    chart.config_mut().left_y.label_style.color = mint;
    let minutes: Vec<f64> = (0..=360).map(|i| f64::from(i) / 4.).collect();
    let current: Vec<f64> = minutes
        .iter()
        .map(|m| {
            let base = if *m < 30. {
                1.2
            } else if *m < 55. {
                2.6
            } else {
                0.9
            };
            base + 0.12 * (m * 1.3).sin() + 0.05 * (m * 4.7).cos()
        })
        .collect();
    let mut charge = 1.0f64;
    let voltage: Vec<f64> = minutes
        .iter()
        .zip(&current)
        .map(|(_, i)| {
            charge -= i / 60. / 4. / 3.2;
            let open = 3.3 + 0.9 * charge.clamp(0., 1.).powf(0.6);
            open - 0.08 * i
        })
        .collect();
    r.add_column("minutes", &column(minutes))?;
    r.add_column("voltage", &column(voltage))?;
    r.add_column("current", &column(current))?;
    let mut overlay = OverlayLayer::default();
    overlay.series = vec!["current".into()];
    overlay.y_axis.out_margin = 110.;
    overlay.y_axis.line_visible = false;
    overlay.y_axis.tick = TickVisibility::None;
    overlay.y_axis.min = 0.;
    overlay.y_axis.max = 3.5;
    overlay.y_axis.major_spacing = 0.5;
    overlay.y_axis.label_style.color = lavender;
    overlay.y_axis.label_style.font_size = 21.;
    overlay.y_axis.title_option.visible = true;
    overlay.y_axis.title_option.text = RichText::plain("Current (A)", lavender, 23., "");
    // Layers are expanded when the chart is built, so the overlay joins the
    // config before the chart that draws it exists.
    let mut config = chart.config().clone();
    config.overlays.push(overlay);
    let mut chart = Chart::new(config);
    chart.set_x_range(0., 90.);
    chart.set_y_range(3.0, 4.3);
    chart.set_overlay_y_range(0, 0., 3.5);
    chart.with_axis_range_change(|config| {
        config.bottom_x.major_spacing = 15.;
        config.left_y.major_spacing = 0.25;
        config.overlays[0].y_axis.major_spacing = 0.5;
    });
    let mut filled = line(mint, 3.5);
    filled.fill = Some(LineFill::ToBaseline(3.0));
    filled.fill_color = Some(Color::from_rgba(mint.r, mint.g, mint.b, 0.12));
    let mut stepped = line(lavender, 2.5);
    stepped.interpolation = LineInterpolation::Linear;
    let specs = [
        series(
            "voltage",
            "minutes",
            "voltage",
            DataRenderType::Line { line: filled },
        ),
        series(
            "current",
            "minutes",
            "current",
            DataRenderType::Line { line: stepped },
        ),
    ];
    chart = chart
        .with_legend_entry("Voltage", mint, 3.5, LegendEntryKind::Line)
        .with_legend_entry("Current", lavender, 2.5, LegendEntryKind::Line);
    chart.config_mut().legend.corner = LegendCorner::TopRight;
    // Overlay layers are part of the document, not the base panel.
    let image =
        pollster::block_on(r.export_document_rgba_with_clear_async(&chart, &specs, 1., bg))?;
    save(&image, bg, &out.join("showcase-twin-axes.png"))
}

// 8. Small multiples: four sensors in a 2 x 2 grid of subplots sharing the
//    hour axis, each with its own range and colour.
fn small_multiples(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(247, 248, 253);
    let ink = rgb(52, 57, 85);
    let grid = rgb(228, 231, 240);
    let hour: Vec<f64> = (0..=240).map(|i| f64::from(i) / 10.).collect();
    r.add_column("h", &column(hour.clone()))?;
    let cell = |x: u32, y: u32, title: &str, y_title: &str| -> Config {
        let mut config = renderer::default::default_config();
        config.chart_area = ChartArea(Rect {
            x,
            y,
            width: 600,
            height: 400,
        });
        chrome(&mut config, bg, ink, grid);
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
        Chart::new(config)
            .with_title(title)
            .with_x_title("Hour")
            .with_y_title(y_title)
            .config()
            .clone()
    };
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
        ),
        ("co2", "CO₂", "ppm", rgb(112, 92, 212), 620., 260., 0.4, 60.),
        (
            "light",
            "Daylight",
            "lux",
            rgb(228, 170, 50),
            400.,
            380.,
            -1.4,
            60.,
        ),
    ];
    let mut specs = Vec::new();
    for (id, _, _, color, base, swing, phase, noise) in sensors {
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
        r.add_column(id, &column(data))?;
        let mut style = line(color, 3.);
        style.fill = Some(LineFill::ToBaseline(-1e4));
        style.fill_color = Some(Color::from_rgba(color.r, color.g, color.b, 0.1));
        specs.push(series(id, "h", id, DataRenderType::Line { line: style }));
    }
    let mut config = cell(0, 0, sensors[0].1, sensors[0].2);
    config.subplots = [(1usize, 600u32, 0u32), (2, 0, 400), (3, 600, 400)]
        .into_iter()
        .map(|(k, x, y)| {
            let mut subplot = Subplot {
                config: cell(x, y, sensors[k].1, sensors[k].2),
                series: vec![sensors[k].0.to_string()],
                link_x: Some(CellRef::Parent),
                link_y: None,
            };
            subplot.config.legend.visible = false;
            subplot
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
    let image =
        pollster::block_on(r.export_document_rgba_with_clear_async(&chart, &specs, 1., bg))?;
    save(&image, bg, &out.join("showcase-small-multiples.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/showcase")?;
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut r = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, 16 * 1024 * 1024)?;
    wind(&mut r, &out)?;
    currents(&mut r, &out)?;
    colormap_scatter(&mut r, &out)?;
    bubbles(&mut r, &out)?;
    forecast(&mut r, &out)?;
    thermostat(&mut r, &out)?;
    twin_axes(&mut r, &out)?;
    small_multiples(&mut r, &out)?;
    r.wait_idle();
    Ok(())
}
