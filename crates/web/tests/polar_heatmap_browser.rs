#![cfg(target_arch = "wasm32")]

//! A heatmap on a polar chart (#208) through the WASM wrapper: the series
//! JSON draws, `pick_data` names the annular cell under the cursor as a
//! `matrix_cell`, the selection reports the cell's angle and radius centres
//! and value read back from the GPU columns, `step_selection` moves along
//! the angle and the radius, and the chart exports.

use figgy::FiggyChart;
use renderer::config::{DataSelectionsConfig, PickedDataRef};
use renderer::data_config::{
    FieldFillConfig, FieldSeries, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
};
use renderer::polar::PolarOptions;
use renderer::{Config, DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const ANGLES: usize = 6;
const RINGS: usize = 3;

fn canvas() -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(480);
    canvas.set_height(480);
    canvas
}

fn field() -> SeriesConfig {
    SeriesConfig {
        series_id: "radar".into(),
        source_id: None,
        label: None,
        x_column: "az".into(),
        y_column: "range".into(),
        z_column: None,
        render_type: DataRenderType::Field(FieldSeries {
            matrix: MatrixRef {
                columns: (0..ANGLES).map(|i| format!("z{i}")).collect(),
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Edges,
            },
            fill: Some(FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            }),
            contour: None,
            surface: None,
        }),
    }
}

fn z(i: usize, j: usize) -> f64 {
    (i * RINGS + j) as f64 + 0.25
}

fn select(chart: &mut FiggyChart, x_index: usize, y_index: usize) {
    let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::MatrixCell {
            source_id: None,
            series_id: "radar".into(),
            x_index,
            y_index,
        }],
        ..Default::default()
    });
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
}

async fn details(chart: &FiggyChart) -> serde_json::Value {
    serde_json::from_str(&chart.selection_details().await.unwrap()).unwrap()
}

#[wasm_bindgen_test(async)]
async fn a_polar_heatmap_picks_selects_steps_and_exports() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let az: Vec<f64> = (0..=ANGLES).map(|i| i as f64 * 60.0).collect();
    let range: Vec<f64> = (0..=RINGS).map(|j| j as f64 * 10.0).collect();
    chart.register_column_f64("az", &az).unwrap();
    chart.register_column_f64("range", &range).unwrap();
    for i in 0..ANGLES {
        let values: Vec<f64> = (0..RINGS).map(|j| z(i, j)).collect();
        chart
            .register_column_f64(&format!("z{i}"), &values)
            .unwrap();
    }
    let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.set_polar(Some(PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    }));
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 360.0;
    config.left_y.min = 0.0;
    config.left_y.max = 30.0;
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = (ANGLES * RINGS) as f64;
    bar.axis.major_spacing = 3.0;
    config.colorbar = Some(bar);
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    chart
        .set_series(&serde_json::to_string(&vec![field()]).unwrap())
        .unwrap();
    chart.frame().unwrap();

    // The cell under the cursor, in (angle, radius) indices.
    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let geometry = config.polar_geometry().unwrap();
    let map = config.polar_angle_map().unwrap();
    let (cx, cy) = (f64::from(geometry.center[0]), f64::from(geometry.center[1]));
    let inner = f64::from(geometry.inner_radius);
    let outer = f64::from(geometry.radius);
    let at = |angle: f64, radius: f64| {
        let a = map.screen_angle(angle);
        let r = inner + (outer - inner) * radius / 30.0;
        ((cx + a.cos() * r) as f32, (cy - a.sin() * r) as f32)
    };
    for (i, j) in [(0usize, 0usize), (2, 1), (5, 2)] {
        let (x, y) = at(i as f64 * 60.0 + 30.0, j as f64 * 10.0 + 5.0);
        let picked = chart.pick_data(x, y, 0.0).await.unwrap();
        let picked: serde_json::Value =
            serde_json::from_str(&picked.as_string().expect("a cell is picked")).unwrap();
        assert_eq!(picked["kind"], "matrix_cell", "{picked}");
        assert_eq!(picked["series_id"], "radar");
        assert_eq!(picked["x_index"], i);
        assert_eq!(picked["y_index"], j);
    }
    // Past the outer ring nothing is picked.
    let (x, y) = at(30.0, 34.0);
    assert!(chart.pick_data(x, y, 0.0).await.unwrap().is_undefined());

    // The selection reads the cell's centre and value from the GPU columns.
    select(&mut chart, 2, 1);
    let selected = details(&chart).await;
    assert_eq!(selected["x"], 150.0);
    assert_eq!(selected["y"], 15.0);
    assert_eq!(selected["value"], z(2, 1));
    // Along the angle, then the radius; the angle clamps at the last cell.
    assert!(chart.step_selection(1, 0).unwrap());
    assert_eq!(details(&chart).await["x"], 210.0);
    assert!(chart.step_selection(0, 1).unwrap());
    let selected = details(&chart).await;
    assert_eq!(selected["y"], 25.0);
    assert_eq!(selected["value"], z(3, 2));
    assert!(chart.step_selection(10, 0).unwrap());
    assert_eq!(details(&chart).await["x_index"], ANGLES - 1);

    let png = chart.export_png(2.0).await.unwrap();
    assert!(png.length() > 0);

    // The series JSON round-trips unchanged.
    let series: Vec<SeriesConfig> = serde_json::from_str(&chart.get_series().unwrap()).unwrap();
    assert_eq!(series, vec![field()]);
}
