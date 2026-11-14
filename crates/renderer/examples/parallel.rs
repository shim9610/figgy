//! Parallel coordinates: one polyline per row across axes with their own
//! ranges, every row coloured by a value column through the colourbar.
//! - `render` writes `gallery-parallel.png`: 120 runs of a hyperparameter
//!   sweep across six axes (learning rate and batch size on logarithmic
//!   axes, and dropout among the linear ones), each run coloured by its validation accuracy. The
//!   host orders the rows by accuracy so the best runs draw on top. The
//!   look is the parallel theme `midnight`: the rows, the axes, the ramp
//!   and the slim colourbar.
//!
//! cargo run --locked -p figgy-renderer --example parallel -- target/parallel
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::theme::ThemeFamily;
use renderer::{
    BarAlign, Renderer, SeriesConfig,
    config::{AxisScale, Category},
    layout::Side,
    parallel::{ParallelAxis, ParallelOptions},
    theme::ChartKind,
};
use std::path::Path;
use styled::{background, export, save, scene_chart, themed};

/// A deterministic value in `0..1` for row `i` and stream `seed`.
fn unit(seed: u64, i: u64) -> f64 {
    let mut x = (seed << 32 ^ i).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let theme = ThemeFamily::Midnight.theme(ChartKind::Parallel);
    let bg = background(&theme);
    let mut chart = scene_chart("Hyperparameter sweep / 120 training runs", "", "");

    // The sweep: each run draws its settings, and its accuracy is a smooth
    // response surface around a good region plus a small run-to-run spread.
    let n = 120u64;
    let mut runs: Vec<[f64; 6]> = (0..n)
        .map(|i| {
            let log_lr = -4. + 3. * unit(1, i);
            let batch = 2f64.powi(4 + (6. * unit(2, i)).floor() as i32);
            let dropout = 0.5 * unit(3, i);
            let layers = (2. + 11. * unit(4, i)).floor();
            let epochs = (10. + 91. * unit(5, i)).floor();
            let accuracy = 94.
                - 12. * (log_lr + 2.6).powi(2)
                - 2.2 * (batch.log2() - 6.).powi(2)
                - 120. * (dropout - 0.2).powi(2)
                - 0.35 * (layers - 8.).powi(2)
                - 0.006 * (epochs - 70.).powi(2)
                + 2. * (unit(6, i) - 0.5);
            [
                10f64.powf(log_lr),
                batch,
                dropout,
                layers,
                epochs,
                accuracy.max(50.),
            ]
        })
        .collect();
    runs.sort_by(|a, b| a[5].total_cmp(&b[5]));

    let ids = ["lr", "batch", "dropout", "layers", "epochs", "accuracy"];
    for (k, id) in ids.iter().enumerate() {
        let values: Vec<f64> = runs.iter().map(|run| run[k]).collect();
        r.add_column(format!("sweep-{id}"), &values)?;
    }
    let mut specs = [
        SeriesConfig::parallel("runs", ids.map(|id| format!("sweep-{id}")))
            .color_by("sweep-accuracy"),
    ];

    let config = chart.config_mut();
    config.bottom_x.categories = Some(
        [
            "Learning rate",
            "Batch size",
            "Dropout",
            "Layers",
            "Epochs",
            "Accuracy (%)",
        ]
        .iter()
        .zip(ids)
        .map(|(label, id)| Category::new(id, *label))
        .collect(),
    );
    // The parallel axes draw in the left axis' style.
    config.left_y.min = 0.;
    config.left_y.max = 1.;
    config.left_y.title_option.visible = false;
    // One significant digit: 0.1, 2, 50 and 0.001, without padding zeros.
    config.left_y.label_style.significant_digits = 1;
    // A little air between the axes' ends and their names.
    config.bottom_x.label_style.label_offset_y = 10.;
    config.legend.visible = false;
    let log = |min: f64, max: f64| ParallelAxis {
        scale: AxisScale::Logarithmic,
        ..ParallelAxis::new(min, max)
    };
    config.set_parallel(Some(ParallelOptions {
        axes: vec![
            log(1e-4, 1e-1),
            log(10., 1000.),
            ParallelAxis::new(0., 0.5),
            ParallelAxis::new(2., 12.),
            ParallelAxis::new(10., 100.),
            ParallelAxis::new(50., 95.),
        ],
    }));
    // The accuracy scale; the theme draws the bar and its ramp.
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 50.;
    bar.axis.max = 95.;
    bar.axis.major_spacing = 10.;
    bar.axis.label_style.significant_digits = 1;
    // A horizontal bar over the last axes leaves the plot the full width.
    bar.side = Side::Top;
    bar.align = BarAlign::End;
    config.colorbar = Some(bar);
    let mut chart = chart.with_colorbar_title("Validation accuracy (%)");
    themed(&mut chart, &theme, &mut specs)?;

    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-parallel.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/parallel")?;
    gallery::with_renderer(|r| render(r, &out))
}
