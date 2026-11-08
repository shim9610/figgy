//! Matrix fields: grids of registered columns drawn as cells. All inputs are
//! synthetic. `render` writes three images:
//! - `contours` writes `gallery-contours.png`: a sampled matrix with
//!   interpolated color, labeled isolines and a colorbar.
//! - `hexbin` writes `gallery-hexbin.png`: 30,000 samples binned by the host
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
use gallery::*;
use renderer::{
    Color, ColorMap, Renderer,
    config::AxisScale,
    data_config::{
        ContourConfig, ContourLabelConfig, DataRenderType, FieldFillConfig, FillMode, GridLayout,
        MatrixOrientation, MatrixRef, Shading,
    },
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
    r.add_column("field-x", &column(axis.clone()))?;
    r.add_column("field-y", &column(axis.clone()))?;
    let mut columns = Vec::new();
    for (i, x) in axis.iter().enumerate() {
        let id = format!("field-z-{i}");
        let values = axis
            .iter()
            .map(|y| {
                1.1 * (-((x + 1.0).powi(2) / 1.3 + (y - 0.7).powi(2) / 0.8)).exp()
                    + 0.85 * (-((x - 1.1).powi(2) / 0.7 + (y + 0.7).powi(2) / 1.8)).exp()
            })
            .collect();
        r.add_column(id.clone(), &column(values))?;
        columns.push(id);
    }
    let contour = ContourConfig {
        levels: (1..=10).map(|i| i as f64 * 0.1).collect(),
        line: line(Color::from_rgba(1.0, 1.0, 1.0, 0.9), 1.3),
        per_level_color: None,
        labels: Some(ContourLabelConfig {
            visible: true,
            font_size: 13.0,
            color: Color::WHITE,
            format: LabelFormat::Decimal,
            significant_digits: 2,
            spacing_px: 140.0,
            anchors: Vec::new(),
            bg_color: None,
            bg_padding_px: 3.0,
        }),
    };
    let specs = [series(
        "potential",
        "field-x",
        "field-y",
        DataRenderType::Field(renderer::data_config::FieldSeries {
            matrix: MatrixRef {
                columns,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            }),
            contour: Some(contour),
            surface: None,
        }),
    )];
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
    c.config_mut().colorbar = Some(bar);
    c.config_mut().grid.show_major_x = false;
    c.config_mut().grid.show_major_y = false;
    save(r, &c, &specs, &out.join("gallery-contours.png"))
}

/// The data areas of the two gallery-chrome scenes in pixels, measured on
/// their exports (with and without the colourbar). Each scene sizes its
/// ranges from them so a data unit spans as many pixels across as up: the
/// hexagons stay regular and the image pixels square.
const HEXBIN_AREA_PX: (f64, f64) = (911.0, 564.0);
const IMAGE_AREA_PX: (f64, f64) = (1015.0, 564.0);

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
    // radii and the row pitch √3 radii, odd columns raised half a row.
    let radius = 0.2;
    let (px, py) = (1.5 * radius, 3f64.sqrt() * radius);
    let cols = ((x_max - x_min) / px).ceil() as usize + 1;
    let rows = ((y_max - y_min) / py).ceil() as usize + 1;
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
        let c0 = ((x - x_min) / px).round() as i64;
        let r0 = ((y - y_min) / py - (c0 & 1) as f64 * 0.5).round() as i64;
        let mut best = (i64::MIN, i64::MIN, f64::INFINITY);
        for c in c0 - 1..=c0 + 1 {
            for row in r0 - 1..=r0 + 1 {
                let cx = x_min + c as f64 * px;
                let cy = y_min + (row as f64 + (c & 1) as f64 * 0.5) * py;
                let d = (x - cx).powi(2) + (y - cy).powi(2);
                if d < best.2 {
                    best = (c, row, d);
                }
            }
        }
        if (0..cols as i64).contains(&best.0) && (0..rows as i64).contains(&best.1) {
            counts[best.0 as usize * rows + best.1 as usize] += 1.0;
        }
    }
    let peak = counts.iter().copied().fold(0.0f64, f64::max);
    r.add_column(
        "hexbin-x",
        &column((0..cols).map(|c| x_min + c as f64 * px).collect()),
    )?;
    r.add_column(
        "hexbin-y",
        &column((0..rows).map(|row| y_min + row as f64 * py).collect()),
    )?;
    let mut ids = Vec::with_capacity(cols);
    for c in 0..cols {
        let id = format!("hexbin-n-{c}");
        let cells = counts[c * rows..(c + 1) * rows]
            .iter()
            .map(|n| if *n > 0.0 { *n } else { f64::NAN })
            .collect();
        r.add_column(id.clone(), &column(cells))?;
        ids.push(id);
    }
    let specs = [series(
        "pickups",
        "hexbin-x",
        "hexbin-y",
        DataRenderType::Field(renderer::data_config::FieldSeries {
            matrix: MatrixRef {
                columns: ids,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Hexagonal,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: None,
            surface: None,
        }),
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
    // Clear of the tick labels: the title's local y runs away from the strip
    // on a right-hand bar.
    bar.axis.title_option.offset_y = -24.0;
    let image = styled::export(r, &chart, &specs, bg)?;
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
    chart.config_mut().legend.bg_color = rgba(8, 10, 18, 0.7);
    chart.config_mut().legend.border_color = rgba(8, 10, 18, 0.0);
    chart.config_mut().legend.content.font_size = 19.0;
    // One image pixel per data unit, as many rows as the data area's aspect
    // asks for, so the pixels stay square.
    let w = 280usize;
    let h = (w as f64 * IMAGE_AREA_PX.1 / IMAGE_AREA_PX.0).round() as usize;
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
    r.add_column(
        "nebula-x",
        &column((0..w).map(|c| c as f64 + 0.5 - half_w).collect()),
    )?;
    r.add_column(
        "nebula-y",
        &column((0..h).map(|row| row as f64 + 0.5 - half_h).collect()),
    )?;
    let mut channels: [Vec<String>; 3] = Default::default();
    for (k, name) in ["r", "g", "b"].into_iter().enumerate() {
        for c in 0..w {
            let id = format!("nebula-{name}-{c}");
            let values = (0..h).map(|row| stretch(pixels[c * h + row][k])).collect();
            r.add_column(id.clone(), &column(values))?;
            channels[k].push(id);
        }
    }
    let lattice = |columns: &Vec<String>| MatrixRef {
        columns: columns.clone(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    };
    let specs = [series(
        "nebula",
        "nebula-x",
        "nebula-y",
        DataRenderType::RgbImage {
            red: lattice(&channels[0]),
            green: lattice(&channels[1]),
            blue: lattice(&channels[2]),
            range: [0.0, 1.0],
            shading: Shading::Interpolated,
            opacity: 1.0,
        },
    )];
    chart.set_x_range(-half_w, half_w);
    chart.set_y_range(-half_h, half_h);
    chart.config_mut().bottom_x.major_spacing = 40.0;
    chart.config_mut().left_y.major_spacing = 40.0;
    chart.config_mut().bottom_x.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    chart.config_mut().left_y.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    let image = styled::export(r, &chart, &specs, bg)?;
    styled::save(&image, bg, &out.join("gallery-imshow.png"))
}

fn main() -> Result {
    let out = output_directory("target/heatmap_contours")?;
    with_renderer(|r| render(r, &out))
}
