#![cfg(target_arch = "wasm32")]

//! The record and ragged registration round trip of the Cartesian wrapper:
//! `register_record` takes an object of typed arrays keyed by box plot field
//! name, `register_ragged` a values and an offsets array; both replace on a
//! second call, reject what the pool rejects, and leave the column registry
//! alone.

use figgy::FiggyChart;
use js_sys::{Float32Array, Float64Array, Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn canvas() -> web_sys::HtmlCanvasElement {
    let canvas = web_sys::window()
        .expect("window")
        .document()
        .expect("document")
        .create_element("canvas")
        .expect("canvas element")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("HtmlCanvasElement");
    canvas.set_width(480);
    canvas.set_height(320);
    canvas
}

fn fields(rows: &[(&str, &[f64])]) -> JsValue {
    let object = Object::new();
    for (name, values) in rows {
        Reflect::set(
            &object,
            &JsValue::from_str(name),
            &Float64Array::from(*values),
        )
        .unwrap();
    }
    object.into()
}

fn message(error: JsValue) -> String {
    Reflect::get(&error, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_default()
}

#[wasm_bindgen_test(async)]
async fn records_and_ragged_columns_register_replace_and_reject() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let summary: [(&str, &[f64]); 5] = [
        ("median", &[2.0, 3.0]),
        ("q1", &[1.0, 2.0]),
        ("q3", &[3.0, 4.0]),
        ("whisker_low", &[0.0, 1.0]),
        ("whisker_high", &[4.0, 5.0]),
    ];
    chart.register_record("box", fields(&summary)).unwrap();

    // Replacement with a new length and an optional Float32Array field.
    let longer: [(&str, &[f64]); 5] = [
        ("median", &[2.0, 3.0, 4.0]),
        ("q1", &[1.0, 2.0, 3.0]),
        ("q3", &[3.0, 4.0, 5.0]),
        ("whisker_low", &[0.0, 1.0, 2.0]),
        ("whisker_high", &[4.0, 5.0, 6.0]),
    ];
    let object = fields(&longer);
    Reflect::set(
        &object,
        &JsValue::from_str("mean"),
        &Float32Array::from(&[2.5f32, f32::NAN, 4.5][..]),
    )
    .unwrap();
    chart.register_record("box", object).unwrap();

    let unknown = fields(&summary);
    Reflect::set(
        &unknown,
        &JsValue::from_str("mode"),
        &Float64Array::from(&[1.0, 2.0][..]),
    )
    .unwrap();
    let error = message(chart.register_record("box", unknown).unwrap_err());
    assert!(error.contains("'mode' is not a field"), "{error}");
    let error = message(
        chart
            .register_record("box", fields(&summary[..4]))
            .unwrap_err(),
    );
    assert!(error.contains("whisker_high"), "{error}");
    let error = message(
        chart
            .register_record("box", JsValue::from_f64(1.0))
            .unwrap_err(),
    );
    assert!(error.contains("object of typed arrays"), "{error}");

    chart
        .register_ragged("outliers", &[9.0, 10.0, -1.0], &[0.0, 2.0, 2.0, 3.0])
        .unwrap();
    chart
        .register_ragged("outliers", &[9.0], &[0.0, 0.0, 1.0])
        .unwrap();
    let error = message(
        chart
            .register_ragged("outliers", &[9.0, 10.0], &[0.0, 2.0, 1.0, 2.0])
            .unwrap_err(),
    );
    assert!(error.contains("non-decreasing"), "{error}");

    // One id space: a column id is not a record id, nor the other way round.
    chart.register_column_f64("x", &[1.0, 2.0]).unwrap();
    assert!(chart.register_record("x", fields(&summary)).is_err());
    assert!(chart.register_ragged("x", &[1.0], &[0.0, 1.0]).is_err());
    assert!(chart.register_ragged("box", &[1.0], &[0.0, 1.0]).is_err());
    assert!(chart.register_column_f64("box", &[1.0]).is_err());
    assert!(
        !chart.remove_column("box").unwrap(),
        "a record is not a column"
    );

    // The chart still draws and exports with the records resident.
    chart.export_png(1.0).await.unwrap();
}
