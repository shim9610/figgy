#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::{
    Color, DataRenderType, SeriesConfig,
    config::{DataSelectionsConfig, PickedDataRef},
    data_config::{
        BarOrientation, DataBarStyleConfig, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    },
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
                half: None,
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

#[wasm_bindgen_test(async)]
async fn point_vector_selection_steps_along_its_anchors() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    // The u column is one value shorter, so the series draws three arrows.
    chart
        .register_column_f32("vx", &[0.0, 1.0, 2.0, 3.0])
        .unwrap();
    chart
        .register_column_f32("vy", &[0.0, 1.0, 2.0, 3.0])
        .unwrap();
    chart.register_column_f32("vu", &[1.0, 1.0, 1.0]).unwrap();
    chart
        .register_column_f32("vv", &[0.0, 0.5, 1.0, 1.5])
        .unwrap();
    let series = vec![SeriesConfig {
        series_id: "arrows".into(),
        source_id: Some("source-arrows".into()),
        label: None,
        x_column: "vx".into(),
        y_column: "vy".into(),
        render_type: DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "vu".into(),
                v: "vv".into(),
            },
            style: VectorStyle {
                color: Color::BLACK,
                width_px: 2.0,
                scale: VectorScale::Data { factor: 1.0 },
                pivot: VectorPivot::Tail,
                head_length_px: 8.0,
                head_width_px: 8.0,
                min_length_px: 0.0,
                color_map: None,
            },
        },
    }];
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
    select(
        &mut chart,
        PickedDataRef::Point {
            source_id: Some("source-arrows".into()),
            series_id: "arrows".into(),
            point_index: 0,
        },
    );
    let point_index = |chart: &FiggyChart| -> serde_json::Value {
        let config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
        config["picked_data"]["refs"][0]["point_index"].clone()
    };
    assert!(chart.step_selection(1, 0).unwrap());
    assert_eq!(point_index(&chart), 1);
    assert!(chart.step_selection(1, 0).unwrap());
    assert_eq!(point_index(&chart), 2);
    // The last arrow is the last index every column has.
    assert!(!chart.step_selection(1, 0).unwrap());
    assert_eq!(point_index(&chart), 2);
    assert!(chart.step_selection(-1, 0).unwrap());
    assert_eq!(point_index(&chart), 1);
}

fn trace(id: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: Some(format!("source-{id}")),
        label: None,
        x_column: "ox".into(),
        y_column: "oy".into(),
        render_type: DataRenderType::Line {
            line: renderer::DataLineStyleConfig {
                line_width: 2.0,
                ..Default::default()
            },
        },
    }
}

/// Offset spectra (#50) through the wrapper: `line_offsets` round-trips
/// through `set_config` / `get_config`, the hit map registers only the base
/// layer's elements (a member panel has none), a pick on a member's shifted
/// trace names the member, and its selection details report the column's
/// own, unshifted values.
#[wasm_bindgen_test(async)]
async fn offset_lines_round_trip_hit_test_and_report_unshifted_values() {
    use renderer::data_config::LineOffsets;
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    chart
        .register_column_f64("ox", &[1.0, 3.0, 5.0, 7.0, 9.0])
        .unwrap();
    chart
        .register_column_f64("oy", &[2.0, 3.5, 2.5, 4.0, 3.0])
        .unwrap();
    chart
        .set_series(&serde_json::to_string(&vec![trace("low"), trace("high")]).unwrap())
        .unwrap();
    let mut config: renderer::Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    for (axis, max) in [(&mut config.bottom_x, 10.0), (&mut config.left_y, 12.0)] {
        axis.min = 0.0;
        axis.max = max;
    }
    config.line_offsets = Some(LineOffsets {
        series: vec!["low".into(), "high".into()],
        step_y: 5.0,
        step_x: 0.5,
    });
    let json = serde_json::to_string(&config).unwrap();
    chart.set_config(&json).unwrap();
    let back: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(
        back["line_offsets"],
        serde_json::json!({ "series": ["low", "high"], "step_y": 5.0, "step_x": 0.5 })
    );
    assert_eq!(
        serde_json::from_value::<renderer::Config>(back).unwrap(),
        config
    );
    chart.frame().unwrap();

    // Data (x, y) to canvas pixels through the displayed base panel.
    let (display, _, _) = renderer::display_config_for_surface(&config, (480, 320));
    let area = display.data_area().unwrap().0;
    let px = |x: f64, y: f64| {
        (
            area.x as f32 + (x / 10.0) as f32 * area.width as f32,
            (area.y + area.height) as f32 - (y / 12.0) as f32 * area.height as f32,
        )
    };
    // The data area answers for every panel; no member element exists.
    let (cx, cy) = px(5.0, 6.0);
    assert_eq!(chart.hit_test(cx, cy).as_deref(), Some("data_area"));

    // Point 3 of `high` is drawn half a unit right and five up.
    let (x, y) = px(7.5, 9.0);
    let picked = chart.pick_point(x, y, 3.0).await.unwrap();
    let picked: serde_json::Value = serde_json::from_str(&picked.as_string().unwrap()).unwrap();
    assert_eq!(picked["series_id"], "high");
    assert_eq!(picked["point_index"], 3);
    select(
        &mut chart,
        PickedDataRef::Point {
            source_id: Some("source-high".into()),
            series_id: "high".into(),
            point_index: 3,
        },
    );
    let selected = details(&chart).await;
    assert_eq!(selected["x"], 7.0);
    assert_eq!(selected["y"], 4.0);
    // Stepping walks the source indices, unshifted.
    assert!(chart.step_selection(-1, 0).unwrap());
    let selected = details(&chart).await;
    assert_eq!(
        (selected["index"].clone(), selected["y"].clone()),
        (2.into(), 2.5.into())
    );
}
