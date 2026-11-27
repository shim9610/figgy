#![cfg(target_arch = "wasm32")]

//! A probability axis (#209) through the WASM wrapper, at the JS boundary:
//! every scale is written and read back as JSON (`{"Probability": ...}`),
//! and one draw answers `pick_data` and `pointer_to_anchor` as JSON and
//! exports. The placement, picks, steps, exports and the other interactions
//! of every scale are the renderer's tests (`probability_interactions.rs`,
//! `pick_draw_agreement.rs`).

use figgy::FiggyChart;
use renderer::config::{AxisScale, Config, ProbabilityScale};
use renderer::data_config::{DataScatterStyleConfig, ScatterShape, XySeries};
use renderer::format::LabelFormat;
use renderer::{Color, DataRenderType, SeriesConfig};
use wasm_bindgen_test::*;

mod support;
use support::canvas_480x360 as canvas;

wasm_bindgen_test_configure!(run_in_browser);

const POINTS: [(f64, f64); 3] = [(2.0, 0.01), (5.0, 0.5), (8.0, 0.98)];

fn markers() -> SeriesConfig {
    SeriesConfig {
        series_id: "plot".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "p".into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: Some(DataScatterStyleConfig {
                point_color: Color::from_rgb8(200, 40, 40),
                point_shape: ScatterShape::CircleFilled,
                point_size: 8.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: None,
            errors: None,
            stem: None,
        }),
    }
}

/// Where the chrome of `config` places `(x, p)`, in canvas pixels.
fn at(config: &Config, x: f64, p: f64) -> (f32, f32) {
    let area = config.data_area().unwrap().0;
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x, area.width as f32);
    let fy = renderer::layout::axis_fraction(p, &config.left_y, area.height as f32);
    (
        area.x as f32 + area.width as f32 * fx,
        area.y as f32 + area.height as f32 * (1.0 - fy),
    )
}

#[wasm_bindgen_test(async)]
async fn a_probability_axis_round_trips_draws_picks_and_exports() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .register_column_f64("x", &POINTS.map(|p| p.0))
        .unwrap();
    chart
        .register_column_f64("p", &POINTS.map(|p| p.1))
        .unwrap();
    for scale in [
        ProbabilityScale::Normal,
        ProbabilityScale::Weibull,
        ProbabilityScale::Logistic,
    ] {
        let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
        config.bottom_x.scale = AxisScale::Linear;
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        for axis in [&mut config.left_y, &mut config.right_y] {
            axis.scale = AxisScale::Probability(scale);
            axis.min = 0.001;
            axis.max = 0.999;
            axis.minor_count = 1;
            axis.label_style.format = LabelFormat::Percent;
        }
        config.picked_data = None;
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains(&format!("{{\"Probability\":\"{scale:?}\"}}")));
        chart.set_config(&json).unwrap();
        chart
            .set_series(&serde_json::to_string(&vec![markers()]).unwrap())
            .unwrap();

        let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
        assert_eq!(config.left_y.scale, AxisScale::Probability(scale));
    }

    // One draw, on the last scale: a pick and an anchor as JSON, and an
    // export.
    chart.frame().unwrap();
    // A pick answers the ink of the last drawn frame, and a probability axis
    // draws its series once their pipelines have compiled.
    chart.series_ready().await.unwrap();
    let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let (index, (x, p)) = (1, POINTS[1]);
    let (px, py) = at(&config, x, p);
    let picked = chart.pick_data(px, py, 4.0).await.unwrap();
    let picked: serde_json::Value =
        serde_json::from_str(&picked.as_string().expect("a point is picked")).unwrap();
    assert_eq!(picked["kind"], "point", "{picked}");
    assert_eq!(picked["series_id"], "plot");
    assert_eq!(picked["point_index"], index, "{picked}");

    let anchor: serde_json::Value =
        serde_json::from_str(&chart.pointer_to_anchor(px, py, "Data").unwrap()).unwrap();
    let y = anchor["Data"]["y"].as_f64().expect("a data anchor");
    assert!(
        (y - p).abs() <= 1e-4 * p.min(1.0 - p),
        "anchor {y} for p = {p}"
    );

    let png = chart.export_png(2.0).await.unwrap();
    assert!(png.length() > 0);
}
