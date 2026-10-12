//! A sampled matrix with interpolated color, labeled isolines and a colorbar.
//! All inputs are synthetic.
//! Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap_contours

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::{
    Color, ColorMap, Renderer,
    data_config::{
        ContourConfig, ContourLabelConfig, DataRenderType, FieldFillConfig, FillMode, GridLayout,
        MatrixOrientation, MatrixRef, Shading,
    },
    default,
    format::LabelFormat,
};
use std::path::Path;

pub fn render(r: &mut Renderer, out: &Path) -> Result {
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
        DataRenderType::HeatmapContour {
            matrix: MatrixRef {
                columns,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            },
            contour,
        },
    )];
    let mut c = chart("A smooth field, exact contour levels", "x", "y", (960, 640));
    c.set_x_range(-3.0, 3.0);
    c.set_y_range(-3.0, 3.0);
    let mut bar = default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.min = 0.0;
    bar.axis.max = 1.2;
    bar.axis.major_spacing = 0.2;
    c.config_mut().colorbar = Some(bar);
    c.config_mut().grid.show_major_x = false;
    c.config_mut().grid.show_major_y = false;
    save(r, &c, &specs, &out.join("gallery-contours.png"))
}

fn main() -> Result {
    let out = output_directory("target/heatmap_contours")?;
    with_renderer(|r| render(r, &out))
}
