//! Parallel coordinates: one polyline per row across axes with their own
//! ranges, every row coloured by a value column through the colourbar.
//! - `render` writes `gallery-parallel.png`: 120 runs of a hyperparameter
//!   sweep across six axes (learning rate and batch size on logarithmic
//!   axes, and dropout among the linear ones), each run coloured by its validation accuracy. The
//!   host orders the rows by accuracy so the best runs draw on top.
//!
//! cargo run --locked -p figgy-renderer --example parallel -- target/parallel
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/styled.rs"]
mod styled;
use gallery::Result;
use renderer::{
    BarAlign, ColorMap, Renderer, SeriesConfig,
    config::{AxisScale, Category, TickVisibility},
    layout::Side,
    parallel::{ParallelAxis, ParallelOptions},
    text::RichText,
};
use std::path::Path;
use styled::{cartesian, colorbar, export, rgb, save};

/// A deterministic value in `0..1` for row `i` and stream `seed`.
fn unit(seed: u64, i: u64) -> f64 {
    let mut x = (seed << 32 ^ i).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

pub fn render(r: &mut Renderer, out: &Path) -> Result {
    let bg = rgb(15, 18, 30);
    let ink = rgb(186, 194, 216);
    let mut chart = cartesian(
        "Hyperparameter sweep / 120 training runs",
        "",
        "",
        bg,
        ink,
        bg,
    );

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
    let specs = [
        SeriesConfig::parallel("runs", ids.map(|id| format!("sweep-{id}")))
            .color(ink)
            .width(1.2)
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
    config.bottom_x.label_style.font_size = 20.;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    // The parallel axes draw in the left axis' style.
    config.left_y.min = 0.;
    config.left_y.max = 1.;
    config.left_y.title_option.visible = false;
    config.left_y.line_visible = true;
    config.left_y.line_color = rgb(96, 104, 130);
    config.left_y.tick = TickVisibility::Outside;
    config.left_y.label_style.font_size = 16.;
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
    colorbar(
        config,
        ColorMap::Custom {
            stops: vec![
                rgb(38, 42, 72),
                rgb(62, 52, 104),
                rgb(120, 54, 146),
                rgb(214, 70, 120),
                rgb(250, 136, 84),
                rgb(255, 220, 120),
            ],
        },
        50.,
        95.,
        10.,
        ink,
    );
    let bar = config.colorbar.as_mut().ok_or("no colourbar")?;
    bar.axis.label_style.significant_digits = 1;
    // A horizontal bar over the last axes leaves the plot the full width.
    bar.side = Side::Top;
    bar.align = BarAlign::End;
    bar.length_frac = 0.3;
    bar.thickness_px = 12.;
    bar.gap_px = 16.;
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text = RichText::plain("Validation accuracy (%)", ink, 19., "");

    let image = export(r, &mut chart, &specs, bg)?;
    save(&image, bg, &out.join("gallery-parallel.png"))
}

fn main() -> Result {
    let out = gallery::output_directory("target/parallel")?;
    gallery::with_renderer(|r| render(r, &out))
}
