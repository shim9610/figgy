//! An axis with a range cut out (`AxisOptions::breaks`, #200).
//! - `gamma_spectrum` writes `gallery-broken-axes.png`: a gamma-ray spectrum
//!   of a Cs-137 and a Co-60 source. The energy axis cuts out the empty
//!   stretch between the 662 keV line and the 1173 / 1332 keV pair, so both
//!   ends keep a readable scale; slashes on the axis line mark the cut.
//!
//! All counts are synthetic: the host computes the peaks, the falling
//! background, the Compton edge and the counting noise.
//!
//! cargo run --locked -p figgy-renderer --example broken_axes -- target/broken-axes
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::{Result, column, line, series};
use renderer::{
    Anchor, Annotation, AnnotationKind, Color, Renderer,
    config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark},
    data_config::{DataRenderType, LineFill, XySeries},
    format::{LabelFormat, PlainNumberFormat},
    text::RichText,
};
use std::path::Path;
use styled::{cartesian, export, rgb, save};

/// The counts per 2 keV channel at `energy` (keV), before noise: three
/// Gaussian photopeaks whose width grows as the square root of the energy
/// (7 % FWHM at 662 keV), a falling background and the Compton continuum of
/// the 1332 keV line, which ends at its edge near 1118 keV.
fn spectrum(energy: f64) -> f64 {
    let peak = |at: f64, height: f64| {
        let sigma = 0.07 * 661.7 / 2.355 * (at / 661.7).sqrt();
        height * (-0.5 * ((energy - at) / sigma).powi(2)).exp()
    };
    let compton = 260. / (1. + ((energy - 1118.) / 14.).exp());
    let background = 90. + 520. * (-(energy - 500.) / 180.).exp();
    background + compton + peak(661.7, 4200.) + peak(1173.2, 1480.) + peak(1332.5, 1310.)
}

/// A deterministic value in [-0.5, 0.5) for sample `i`.
fn jitter(i: u32) -> f64 {
    ((f64::from(i) * 0.618 + 0.31).sin() * 43758.5453)
        .fract()
        .abs()
        - 0.5
}

// A gamma-ray spectrum with the energy axis broken between the two sources.
fn gamma_spectrum(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(19, 23, 37);
    let ink = rgb(192, 199, 222);
    let cyan = rgb(96, 208, 232);
    let mut chart = cartesian(
        "Gamma spectrum / Cs-137 and Co-60",
        "Energy (keV)",
        "Counts per 2 keV",
        bg,
        ink,
        rgb(40, 46, 70),
    );
    let energy: Vec<f64> = (0..=475).map(|i| 500. + 2. * f64::from(i)).collect();
    // Counting noise: about one standard deviation of a Poisson count.
    let counts: Vec<f64> = energy
        .iter()
        .zip(0u32..)
        .map(|(e, i)| {
            let mean = spectrum(*e);
            (mean + 2. * mean.sqrt() * jitter(i)).max(0.)
        })
        .collect();
    r.add_column("gamma-energy", &column(energy))?;
    r.add_column("gamma-counts", &column(counts))?;
    let mut trace = line(cyan, 2.5);
    trace.fill = Some(LineFill::ToBaseline(0.));
    trace.fill_color = Some(Color::from_rgba(cyan.r, cyan.g, cyan.b, 0.14));
    let specs = [series(
        "spectrum",
        "gamma-energy",
        "gamma-counts",
        DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(trace),
            errors: None,
            stem: None,
        }),
    )];
    chart.config_mut().legend.visible = false;
    chart.set_x_range(500., 1450.);
    chart.set_y_range(0., 6000.);
    let config = chart.config_mut();
    config.bottom_x.major_spacing = 50.;
    config.left_y.major_spacing = 1000.;
    config.left_y.out_margin = 140.;
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    }
    // The cut and its marks need the axis line they sit on.
    config.bottom_x.line_visible = true;
    config.bottom_x.line_color = Color::from_rgba(ink.r, ink.g, ink.b, 0.6);
    config.bottom_x.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(760., 1080.)],
        style: AxisBreakStyle::Continuous,
        mark: BreakMark::Slash {
            size: 14.,
            color: Some(ink),
            width: 2.,
        },
        frame_each_panel: false,
        ..AxisBreaks::default()
    });
    let label = |x: f64, y: f64, text: &str, size: f32, color: Color| {
        Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Data { x, y },
            text: RichText::plain(text, color, size, ""),
            box_style: None,
        })
    };
    config.annotations.items = vec![
        label(690., 4500., "Cs-137", 22., cyan),
        label(690., 4170., "661.7 keV", 18., ink),
        label(1232., 2750., "Co-60", 22., cyan),
        label(1140., 2120., "1173.2 keV", 18., ink),
        label(1300., 1880., "1332.5 keV", 18., ink),
    ];
    let image = export(r, &chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-broken-axes.png"))
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    gamma_spectrum(r, out)
}

fn main() -> Result {
    let out = gallery::output_directory("target/broken-axes")?;
    gallery::with_renderer(|r| render(r, &out))
}
