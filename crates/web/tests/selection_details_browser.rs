#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::{
    Color, DataRenderType, SeriesConfig,
    config::{DataSelectionsConfig, PickedDataRef},
    data_config::{BarOrientation, DataBarStyleConfig},
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

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
    canvas.set_height(320);
    canvas
}

fn histogram(id: &str, edges: &str, values: &str, orientation: BarOrientation) -> SeriesConfig {
    let (x, y) = if orientation == BarOrientation::Horizontal {
        (values, edges)
    } else {
        (edges, values)
    };
    SeriesConfig {
        series_id: id.into(),
        source_id: Some(format!("source-{id}")),
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::BLACK,
                border_color: Color::BLACK,
                border_width: 0.0,
                baseline: 0.0,
                gap_px: 0.0,
                width_ratio: 1.0,
                orientation,
                bar_style_overrides: None,
            },
        },
    }
}

fn select(chart: &mut FiggyChart, reference: PickedDataRef) {
    let mut config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![reference],
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
async fn histogram_selection_reads_live_gpu_bins_and_steps_in_both_orientations() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    assert!(details(&chart).await.is_null());
    // Nonuniform edges and fractional heights distinguish bin values from
    // source sample counts. f64 edges exercise both halves of GPU hi/lo pairs.
    let origin = 1_000_000_000.125;
    chart
        .register_column_f64(
            "edges",
            &[origin, origin + 0.25, origin + 1.5, origin + 5.0],
        )
        .unwrap();
    chart
        .register_column_f64("heights", &[0.125, 1.25, 2.75])
        .unwrap();
    chart
        .register_column_f32("short-edges", &[0.0, 4.0])
        .unwrap();
    chart.register_column_f32("short-heights", &[3.5]).unwrap();
    let series = vec![
        histogram("vertical", "edges", "heights", BarOrientation::Vertical),
        histogram("horizontal", "edges", "heights", BarOrientation::Horizontal),
        histogram(
            "short-vertical",
            "short-edges",
            "short-heights",
            BarOrientation::Vertical,
        ),
        histogram(
            "short-horizontal",
            "short-edges",
            "short-heights",
            BarOrientation::Horizontal,
        ),
    ];
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
    for (id, orientation, bin_move, series_move) in [
        ("vertical", "Vertical", (1, 0), (0, 1)),
        ("horizontal", "Horizontal", (0, 1), (1, 0)),
    ] {
        select(
            &mut chart,
            PickedDataRef::HistogramBin {
                source_id: Some(format!("source-{id}")),
                series_id: id.into(),
                bin_index: 1,
            },
        );
        let selected = details(&chart).await;
        assert_eq!(selected["bin_start"], origin + 0.25);
        assert_eq!(selected["bin_end"], origin + 1.5);
        assert_eq!(selected["value"], 1.25);
        assert_eq!(selected["orientation"], orientation);
        assert_eq!(selected["identity"]["kind"], "histogram_bin");
        assert_eq!(selected["identity"]["bin_index"], 1);
        assert!(chart.step_selection(bin_move.0, bin_move.1).unwrap());
        assert_eq!(details(&chart).await["index"], 2);
        assert!(!chart.step_selection(bin_move.0, bin_move.1).unwrap());
        assert!(chart.step_selection(series_move.0, series_move.1).unwrap());
        let selected = details(&chart).await;
        assert_eq!(selected["series_id"], format!("short-{id}"));
        assert_eq!(selected["index"], 0);
        assert_eq!(selected["bin_start"], 0.0);
        assert_eq!(selected["bin_end"], 4.0);
        assert_eq!(selected["value"], 3.5);
        assert_eq!(
            selected["identity"]["source_id"],
            format!("source-short-{id}")
        );
    }
    // A GPU column replacement must be reflected, not a stale host-side copy.
    chart
        .update_register_column_f64("short-heights", &[6.25])
        .unwrap();
    assert_eq!(details(&chart).await["value"], 6.25);
}
