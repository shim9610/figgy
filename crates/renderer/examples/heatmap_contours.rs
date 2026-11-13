//! Matrix fields: grids of registered columns drawn as cells. All inputs are
//! synthetic. `render` writes three images:
//! - `contours` writes `gallery-contours.png`: a sampled matrix with
//!   interpolated color, labeled isolines and a colorbar.
//! - `hexbin` writes `gallery-hexbin.png`: 100,000 samples binned by the host
//!   into counts on a hexagonal lattice (`GridLayout::Hexagonal`), empty
//!   cells left unpainted, in the shared 1200 x 800 gallery chrome.
//! - `imshow` writes `gallery-imshow.png`: a colour image
//!   (`DataRenderType::RgbImage`), three narrowband channels on one lattice,
//!   in the shared gallery chrome.
//!
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap_contours

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, chart, output_directory, save, with_renderer};
use renderer::{
    Color, ColorMap, Renderer, SeriesConfig,
    config::AxisScale,
    data_config::{ContourLabelConfig, MatrixRef, Shading},
    default,
    format::{LabelFormat, PlainNumberFormat},
    text::RichText,
};
use std::path::Path;
use styled::{rgb, rgba};

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    contours(r, out)?;
    hexbin(r, out)?;
    imshow(r, out)
}

fn contours(r: &mut Renderer, out: &Path) -> Result {
    let n = 81;
    let axis: Vec<f64> = (0..n)
        .map(|i| -3.0 + 6.0 * i as f64 / (n - 1) as f64)
        .collect();
    r.add_column("field-x", &axis)?;
    r.add_column("field-y", &axis)?;
    let mut columns = Vec::new();
    for (i, x) in axis.iter().enumerate() {
        let id = format!("field-z-{i}");
        let values: Vec<f64> = axis
            .iter()
            .map(|y| {
                1.1 * (-((x + 1.0).powi(2) / 1.3 + (y - 0.7).powi(2) / 0.8)).exp()
                    + 0.85 * (-((x - 1.1).powi(2) / 0.7 + (y + 0.7).powi(2) / 1.8)).exp()
            })
            .collect();
        r.add_column(id.clone(), &values)?;
        columns.push(id);
    }
    let levels = (1..=10).map(|i| i as f64 * 0.1).collect();
    let matrix = MatrixRef::columns_are_x(columns);
    // Under the isolines an interpolated fill; on them white labels.
    let specs = [
        SeriesConfig::contours("potential", "field-x", "field-y", matrix, levels)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.9))
            .width(1.3)
            .filled()
            .shading(Shading::Interpolated)
            .contour_labels(ContourLabelConfig {
                font_size: 13.0,
                color: Color::WHITE,
                spacing_px: 140.0,
                bg_padding_px: 3.0,
                ..Default::default()
            }),
    ];
    let mut c = chart("A smooth field, exact contour levels", "x", "y", (960, 640));
    c.set_x_range(-3.0, 3.0);
    c.set_y_range(-3.0, 3.0);
    let mut bar = default::default_colorbar_options();
    // The gallery image was rendered with this gap.
    bar.gap_px = 24.0;
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 1.2;
    bar.axis.major_spacing = 0.2;
    // The colourbar titled with the field it maps, in TeX, in the axis
    // titles' ink and size.
    let axis_title = &c.config().left_y.title_option.text;
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text =
        RichText::from_tex(r"$z = f(x, y)$", axis_title.color, axis_title.font_size, "")?;
    c.config_mut().colorbar = Some(bar);
    c.config_mut().grid.show_major_x = false;
    c.config_mut().grid.show_major_y = false;
    save(r, &mut c, &specs, &out.join("gallery-contours.png"))
}

/// The data area of the hexbin scene in pixels, measured on its export. The
/// scene sizes its ranges from it so a data unit spans as many pixels across
/// as up: the hexagons stay regular.
const HEXBIN_AREA_PX: (f64, f64) = (911.0, 564.0);

/// A deterministic hash in -1..1, the host's stand-in for random draws.
fn noise(seed: u64, i: usize) -> f64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64) * 2.0 - 1.0
}

/// A uniform draw in (0, 1).
fn uniform(seed: u64, i: usize) -> f64 {
    ((noise(seed, i) + 1.0) * 0.5).clamp(1e-12, 1.0 - 1e-12)
}

/// Two independent standard normal draws (Box-Muller).
fn normal_pair(seed: u64, i: usize) -> (f64, f64) {
    let radius = (-2.0 * uniform(seed, i).ln()).sqrt();
    let angle = std::f64::consts::TAU * uniform(seed + 1, i);
    (radius * angle.cos(), radius * angle.sin())
}

// A hexbin: 100,000 taxi pickups over a city, binned by the host into counts
// on a lattice of hexagons 400 m across the flats. Both axes are in
// kilometres, so the lattice is regular in the data metric. The renderer
// draws the supplied counts through a logarithmic colourbar; empty cells
// are NaN and unpainted.
fn hexbin(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(250, 250, 247);
    let ink = rgb(48, 52, 66);
    let mut chart = styled::cartesian(
        "Taxi pickups / 100,000 rides",
        "East (km)",
        "North (km)",
        bg,
        ink,
        rgb(234, 235, 238),
    );
    chart.config_mut().legend.visible = false;
    chart.config_mut().grid.show_major_y = false;
    // A kilometre spans as many pixels across as up.
    let (x_min, x_max) = (0.0, 16.0);
    let (y_min, y_max) = (0.0, 16.0 * HEXBIN_AREA_PX.1 / HEXBIN_AREA_PX.0);
    // Regular hexagons of circumradius `radius`: the column pitch is 1.5
    // radii and the row pitch √3 radii, odd columns raised half a row. The
    // lattice starts one half cell inside the ranges and keeps only cells
    // that fit whole, so no hexagon is cut by the frame.
    let radius = 0.2;
    let (px, py) = (1.5 * radius, 3f64.sqrt() * radius);
    let (x0, y0) = (x_min + radius, y_min + 0.5 * py);
    let cols = ((x_max - radius - x0) / px).floor() as usize + 1;
    let rows = ((y_max - 0.5 * py - y0) / py).floor() as usize + 1;
    let fits = |c: i64, row: i64| y0 + (row as f64 + (c & 1) as f64 * 0.5 + 0.5) * py <= y_max;
    let mut counts = vec![0.0f64; cols * rows];
    for i in 0..100_000 {
        let (g1, g2) = normal_pair(101, i);
        let pick = uniform(105, i);
        let (x, y) = if pick < 0.45 {
            // Downtown, stretched along the river.
            (6.2 + 0.95 * g1 + 0.3 * g2, 5.3 + 0.35 * g1 + 0.6 * g2)
        } else if pick < 0.71 {
            // The avenue from downtown to the north-east.
            let t = uniform(107, i);
            (6.6 + 4.6 * t + 0.3 * g1, 5.6 + 2.9 * t + 0.3 * g2)
        } else if pick < 0.84 {
            // The airport.
            (13.3 + 0.32 * g1, 2.2 + 0.22 * g2)
        } else if pick < 0.9 {
            // The stadium.
            (2.8 + 0.25 * g1, 2.4 + 0.25 * g2)
        } else {
            // The rest of the city.
            (7.6 + 3.0 * g1, 5.0 + 1.9 * g2)
        };
        // The host's binning: the nearest centre among the nearest
        // column's rows and their neighbours.
        let c0 = ((x - x0) / px).round() as i64;
        let r0 = ((y - y0) / py - (c0 & 1) as f64 * 0.5).round() as i64;
        let mut best = (i64::MIN, i64::MIN, f64::INFINITY);
        for c in c0 - 1..=c0 + 1 {
            for row in r0 - 1..=r0 + 1 {
                let cx = x0 + c as f64 * px;
                let cy = y0 + (row as f64 + (c & 1) as f64 * 0.5) * py;
                let d = (x - cx).powi(2) + (y - cy).powi(2);
                if d < best.2 {
                    best = (c, row, d);
                }
            }
        }
        if (0..cols as i64).contains(&best.0)
            && (0..rows as i64).contains(&best.1)
            && fits(best.0, best.1)
        {
            counts[best.0 as usize * rows + best.1 as usize] += 1.0;
        }
    }
    let peak = counts.iter().copied().fold(0.0f64, f64::max);
    let centres_x: Vec<f64> = (0..cols).map(|c| x0 + c as f64 * px).collect();
    let centres_y: Vec<f64> = (0..rows).map(|row| y0 + row as f64 * py).collect();
    r.add_column("hexbin-x", &centres_x)?;
    r.add_column("hexbin-y", &centres_y)?;
    let mut ids = Vec::with_capacity(cols);
    for c in 0..cols {
        let id = format!("hexbin-n-{c}");
        let cells: Vec<f64> = counts[c * rows..(c + 1) * rows]
            .iter()
            .map(|n| if *n > 0.0 { *n } else { f64::NAN })
            .collect();
        r.add_column(id.clone(), &cells)?;
        ids.push(id);
    }
    let specs = [SeriesConfig::hexbin(
        "pickups",
        "hexbin-x",
        "hexbin-y",
        MatrixRef::columns_are_x(ids),
    )];
    chart.set_x_range(x_min, x_max);
    chart.set_y_range(y_min, y_max);
    chart.config_mut().bottom_x.major_spacing = 2.0;
    chart.config_mut().left_y.major_spacing = 2.0;
    chart.config_mut().bottom_x.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    chart.config_mut().left_y.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    // The colourbar ends at the peak count rounded up to 1, 2 or 5 x 10^k.
    let decade = 10f64.powf(peak.log10().floor());
    let top = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * decade)
        .find(|v| *v >= peak)
        .unwrap_or(peak);
    styled::colorbar(
        chart.config_mut(),
        ColorMap::Custom {
            stops: vec![
                rgb(226, 238, 228),
                rgb(156, 210, 188),
                rgb(72, 166, 170),
                rgb(40, 106, 150),
                rgb(44, 50, 112),
                rgb(30, 16, 58),
            ],
        },
        1.0,
        top,
        1.0,
        ink,
    );
    let bar = chart.config_mut().colorbar.as_mut().unwrap();
    bar.axis.scale = AxisScale::Logarithmic;
    bar.axis.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    bar.nan_color = Color::new(0.0, 0.0, 0.0, 0.0);
    // Room for the title inside the frame.
    bar.offset_x = -26.0;
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text = RichText::plain("Pickups per cell", ink, 19.0, "");
    let image = styled::export(r, &mut chart, &specs, bg)?;
    styled::save(&image, bg, &out.join("gallery-hexbin.png"))
}

// A colour image: a planetary nebula through three narrowband filters,
// composited by the host into red, green and blue channel grids on one
// lattice of 1 arcsec pixels. The renderer maps each channel from its range
// and blends the samples; no colourbar is involved.
fn imshow(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(8, 10, 18);
    let ink = rgb(170, 180, 202);
    let mut chart = styled::cartesian(
        "A planetary nebula / three narrowband channels",
        "Right ascension offset (arcsec)",
        "Declination offset (arcsec)",
        bg,
        ink,
        bg,
    )
    .with_legend_entry(
        "Hα and [N II] in red",
        rgb(236, 88, 64),
        2.0,
        renderer::config::LegendEntryKind::Scatter,
    )
    .with_legend_entry(
        "[O III] in green and blue",
        rgb(60, 196, 214),
        2.0,
        renderer::config::LegendEntryKind::Scatter,
    );
    chart.config_mut().grid.show_major_y = false;
    chart.config_mut().legend.bg_color = rgba(8, 10, 18, 0.0);
    chart.config_mut().legend.border_color = rgba(8, 10, 18, 0.0);
    chart.config_mut().legend.padding = 4.0;
    chart.config_mut().legend.content.font_size = 19.0;
    // The legend in one row under the x title, off the image; the chart is
    // laid out with the image's width and a provisional height.
    let w = 280usize;
    chart.set_x_range(-(w as f64) / 2.0, w as f64 / 2.0);
    chart.set_y_range(-80.0, 80.0);
    plain_tick_labels(&mut chart);
    styled::legend_row_under_x_title(&mut chart, 1.2);
    // One image pixel per data unit, as many rows as the laid-out data
    // area's aspect asks for, so the pixels stay square.
    let area = chart
        .config()
        .data_area()
        .map_err(|e| format!("imshow layout: {e:?}"))?;
    let h = (w as f64 * f64::from(area.height) / f64::from(area.width)).round() as usize;
    let (half_w, half_h) = (w as f64 / 2.0, h as f64 / 2.0);
    let mut pixels = vec![[0.0f64; 3]; w * h];
    let tilt = 28f64.to_radians();
    for c in 0..w {
        for row in 0..h {
            let (x, y) = (c as f64 + 0.5 - half_w, row as f64 + 0.5 - half_h);
            let xr = x * tilt.cos() + y * tilt.sin();
            let yr = -x * tilt.sin() + y * tilt.cos();
            let (a, b) = (xr / 62.0, yr / 47.0);
            let rho = a.hypot(b);
            let phi = b.atan2(a);
            // Filaments: a few incommensurate ripples around the shell.
            let filaments = 1.0
                + 0.3 * (7.0 * phi + 4.0 * rho).sin() * (3.0 * phi - 2.0 * rho).cos()
                + 0.15 * (13.0 * phi).sin();
            let o3 = 0.85 * (-(rho / 0.78).powi(4)).exp();
            let ha = (-((rho - 0.98) / 0.17).powi(2)).exp()
                * filaments
                * (1.0 + 0.3 * (2.0 * phi).cos())
                + 0.1 * (-((rho - 1.55) / 0.35).powi(2)).exp() * (1.0 + 0.4 * (3.0 * phi).sin());
            let n2 = 0.8
                * (-((rho - 1.08) / 0.09).powi(2)).exp()
                * (1.0 + 0.5 * (11.0 * phi + 1.0).sin().max(0.0));
            let star = 1.2 * (-(x * x + y * y) / (2.0 * 1.4 * 1.4)).exp();
            pixels[c * h + row] = [
                0.015 + 0.95 * ha + 0.75 * n2 + 0.08 * o3 + star,
                0.02 + 0.3 * ha + 0.3 * n2 + 0.7 * o3 + star,
                0.04 + 0.12 * ha + 0.08 * n2 + 0.9 * o3 + star * 1.1,
            ];
        }
    }
    // Field stars, slightly tinted, each spread over a few pixels.
    for k in 0..150 {
        let (sx, sy) = (
            uniform(201, k) * w as f64 - half_w,
            uniform(203, k) * h as f64 - half_h,
        );
        let brightness = 0.12 + 0.9 * uniform(205, k).powi(4);
        let sigma = 0.6 + 0.9 * brightness;
        let tint = noise(207, k);
        let reach = (4.0 * sigma).ceil() as i64;
        let (cx, cy) = ((sx + half_w) as i64, (sy + half_h) as i64);
        for c in (cx - reach).max(0)..=(cx + reach).min(w as i64 - 1) {
            for row in (cy - reach).max(0)..=(cy + reach).min(h as i64 - 1) {
                let dx = c as f64 + 0.5 - half_w - sx;
                let dy = row as f64 + 0.5 - half_h - sy;
                let g = brightness * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
                let p = &mut pixels[c as usize * h + row as usize];
                p[0] += g * (1.0 + 0.2 * tint);
                p[1] += g;
                p[2] += g * (1.0 - 0.2 * tint);
            }
        }
    }
    // The host's display stretch: an arcsinh curve lifts the faint halo.
    let stretch = |v: f64| ((8.0 * v).asinh() / 8f64.asinh()).clamp(0.0, 1.0);
    let pixel_x: Vec<f64> = (0..w).map(|c| c as f64 + 0.5 - half_w).collect();
    let pixel_y: Vec<f64> = (0..h).map(|row| row as f64 + 0.5 - half_h).collect();
    r.add_column("nebula-x", &pixel_x)?;
    r.add_column("nebula-y", &pixel_y)?;
    let mut channels: [Vec<String>; 3] = Default::default();
    for (k, name) in ["r", "g", "b"].into_iter().enumerate() {
        for c in 0..w {
            let id = format!("nebula-{name}-{c}");
            let values: Vec<f64> = (0..h).map(|row| stretch(pixels[c * h + row][k])).collect();
            r.add_column(id.clone(), &values)?;
            channels[k].push(id);
        }
    }
    let [red, green, blue] = channels.map(MatrixRef::columns_are_x);
    let specs = [
        SeriesConfig::rgb_image("nebula", "nebula-x", "nebula-y", red, green, blue)
            .shading(Shading::Interpolated),
    ];
    chart.set_x_range(-half_w, half_w);
    chart.set_y_range(-half_h, half_h);
    plain_tick_labels(&mut chart);
    let image = styled::export(r, &mut chart, &specs, bg)?;
    styled::save(&image, bg, &out.join("gallery-imshow.png"))
}

/// Tick labels every 40 arcsec in plain numbers; a range change resets the
/// format.
fn plain_tick_labels(chart: &mut renderer::Chart) {
    let config = chart.config_mut();
    config.bottom_x.major_spacing = 40.0;
    config.left_y.major_spacing = 40.0;
    config.bottom_x.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    config.left_y.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
}

fn main() -> Result {
    let out = output_directory("target/heatmap_contours")?;
    with_renderer(|r| render(r, &out))
}
