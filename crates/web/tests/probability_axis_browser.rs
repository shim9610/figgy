#![cfg(target_arch = "wasm32")]

//! A probability axis (#209) through the WASM wrapper: the scale set as
//! JSON (`{"Probability": ...}`) draws, `pick_data` names the point under
//! the cursor where the axis chrome places it, `pointer_to_anchor` answers
//! a probability, `step_selection` moves along the series, the chart
//! exports, and the configuration round-trips with the scale unchanged.

use figgy::FiggyChart;
use renderer::config::{AxisScale, Config, DataSelectionsConfig, PickedDataRef, ProbabilityScale};
use renderer::data_config::{DataScatterStyleConfig, ScatterShape, XySeries};
use renderer::format::LabelFormat;
use renderer::{Color, DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const POINTS: [(f64, f64); 3] = [(2.0, 0.01), (5.0, 0.5), (8.0, 0.98)];

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
    canvas.set_height(360);
    canvas
}

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
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x);
    let fy = renderer::layout::axis_fraction(p, &config.left_y);
    (
        area.x as f32 + area.width as f32 * fx,
        area.y as f32 + area.height as f32 * (1.0 - fy),
    )
}

#[wasm_bindgen_test(async)]
async fn a_probability_axis_draws_picks_anchors_steps_and_exports() {
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
        chart.frame().unwrap();

        let config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
        assert_eq!(config.left_y.scale, AxisScale::Probability(scale));
        for (index, &(x, p)) in POINTS.iter().enumerate() {
            let (px, py) = at(&config, x, p);
            let picked = chart.pick_data(px, py, 4.0).await.unwrap();
            let picked: serde_json::Value =
                serde_json::from_str(&picked.as_string().expect("a point is picked")).unwrap();
            assert_eq!(picked["kind"], "point", "{scale:?} {picked}");
            assert_eq!(picked["series_id"], "plot");
            assert_eq!(picked["point_index"], index, "{scale:?} {picked}");

            let anchor: serde_json::Value =
                serde_json::from_str(&chart.pointer_to_anchor(px, py, "Data").unwrap()).unwrap();
            let y = anchor["Data"]["y"].as_f64().expect("a data anchor");
            assert!(
                (y - p).abs() <= 1e-4 * p.min(1.0 - p),
                "{scale:?}: anchor {y} for p = {p}"
            );
        }

        let mut selected = config.clone();
        selected.picked_data = Some(DataSelectionsConfig {
            refs: vec![PickedDataRef::Point {
                source_id: None,
                series_id: "plot".into(),
                point_index: 0,
            }],
            ..Default::default()
        });
        chart
            .set_config(&serde_json::to_string(&selected).unwrap())
            .unwrap();
        assert!(chart.step_selection(1, 0).unwrap());
        let stepped: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
        assert_eq!(
            stepped.picked_data.unwrap().refs[0],
            PickedDataRef::Point {
                source_id: None,
                series_id: "plot".into(),
                point_index: 1,
            }
        );

        let png = chart.export_png(2.0).await.unwrap();
        assert!(png.length() > 0);
        // Text is written as glyph outlines (`<path id="g…">`), not as
        // characters, and the data as an embedded image.
        let svg = chart.export_svg(1.0).await.unwrap();
        assert!(
            svg.contains("<path id=\"g"),
            "{scale:?}: the tick label glyphs"
        );
        assert!(svg.contains("<image"), "{scale:?}: the data image");
    }
}
