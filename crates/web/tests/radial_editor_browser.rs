#![cfg(target_arch = "wasm32")]

//! The radial chart's editor calls in a real browser (#313): a radial chart
//! on a `FiggyChart`'s device (one device, one ledger, the parent's and the
//! child's disposal and a device loss), the export background override at
//! DPR 1 and 2 and two export scales, the generated slice labels after a
//! value or format edit with a save and reopen, the decoration position
//! reset, and the read-only annotation preview at DPR 1 and 2 (#334).

use figgy::{FiggyChart, FiggyRadialChart};
use renderer::Color;
use renderer::radial::{RadialChart, RadialLabelFormat, RadialLabels, RadialSlice};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

/// The document's logical size.
const SIZE: (u32, u32) = (480, 360);

fn canvas(dpr: u32) -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(SIZE.0 * dpr);
    canvas.set_height(SIZE.1 * dpr);
    canvas
}

/// A titled two-slice pie with outside `NamePercent` labels and no drawn
/// text: opening it writes the generated texts.
fn chart() -> RadialChart {
    let mut chart = RadialChart {
        slices: vec![
            RadialSlice::new("A", 25.0, Color::new(1.0, 0.0, 0.0, 1.0)),
            RadialSlice::new("B", 75.0, Color::new(0.0, 0.0, 1.0, 1.0)),
        ],
        labels: RadialLabels::Outside,
        label_format: RadialLabelFormat::NamePercent,
        label_decimals: 0,
        background: Color::new(0.4, 0.6, 0.8, 1.0),
        ..Default::default()
    };
    chart.set_title_text("Budget");
    chart
}

fn document() -> String {
    serde_json::to_string(&chart()).unwrap()
}

async fn radial(dpr: u32) -> FiggyRadialChart {
    FiggyRadialChart::create(canvas(dpr), "radial", &document(), SIZE.0, SIZE.1)
        .await
        .unwrap()
}

fn read(chart: &FiggyRadialChart) -> RadialChart {
    serde_json::from_str(&chart.get_document().unwrap()).unwrap()
}

/// Count every `GPUAdapter.requestDevice` from now on and keep the last
/// device it returned, so a test can lose it.
fn count_devices() {
    js_sys::eval(
        r#"(() => {
  const P = GPUAdapter.prototype;
  if (!P.__figgyRequestDevice) P.__figgyRequestDevice = P.requestDevice;
  const S = globalThis.__devices = { count: 0, last: null };
  P.requestDevice = async function (d) {
    const device = await P.__figgyRequestDevice.call(this, d);
    S.count += 1;
    S.last = device;
    return device;
  };
})()"#,
    )
    .unwrap();
}

fn devices_requested() -> u32 {
    js_sys::eval("globalThis.__devices.count")
        .unwrap()
        .as_f64()
        .unwrap() as u32
}

fn total_bytes(parent: &FiggyChart) -> u64 {
    let status: serde_json::Value = serde_json::from_str(&parent.gpu_memory_status()).unwrap();
    status["total_bytes"].as_u64().unwrap()
}

/// One timer turn of 20 ms.
async fn turn() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

/// Poll `future` once, so the work it starts is submitted before the test
/// goes on, and hand it back to be awaited (or dropped) later.
fn started<F: std::future::Future>(future: F) -> std::pin::Pin<Box<F>> {
    let mut future = Box::pin(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(
        future.as_mut().poll(&mut context).is_pending(),
        "the export waits for the GPU"
    );
    future
}

/// The PNG's size and its straight-alpha RGBA at physical pixels `points`,
/// decoded by the browser.
async fn pixels(png: &js_sys::Uint8Array, points: &[(f64, f64)]) -> serde_json::Value {
    let decode: js_sys::Function = js_sys::eval(
        r#"(async (bytes, points) => {
  const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }),
    { premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const g = canvas.getContext('2d');
  g.drawImage(bitmap, 0, 0);
  const at = ([x, y]) => Array.from(g.getImageData(Math.floor(x * bitmap.width),
    Math.floor(y * bitmap.height), 1, 1).data);
  return JSON.stringify({ width: bitmap.width, height: bitmap.height, at: points.map(at) });
})"#,
    )
    .unwrap()
    .dyn_into()
    .unwrap();
    let points = serde_json::to_string(points).unwrap();
    let points = js_sys::JSON::parse(&points).unwrap();
    let promise: js_sys::Promise = decode
        .call2(&JsValue::NULL, png, &points)
        .unwrap()
        .dyn_into()
        .unwrap();
    let text = JsFuture::from(promise).await.unwrap().as_string().unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The JS error message of a rejected call.
fn message(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &"message".into())
        .ok()
        .and_then(|m| m.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_default()
}

// --- 1. One device with the Cartesian chart ---

/// Switching between a Cartesian chart and radial charts on its device
/// requests one device, and each radial chart charges the parent's ledger;
/// disposing a radial chart, from Rust or from JS (`free()`), leaves the
/// parent drawing and exporting.
#[wasm_bindgen_test(async)]
async fn radial_charts_on_a_cartesian_device_share_one_device_and_ledger() {
    count_devices();
    let mut parent = FiggyChart::create(canvas(1)).await.unwrap();
    assert_eq!(devices_requested(), 1);
    let alone = total_bytes(&parent);
    for round in 0..3 {
        let mut child = FiggyRadialChart::create_on_device_of(
            &parent,
            canvas(1),
            "radial".into(),
            document(),
            SIZE.0,
            SIZE.1,
        )
        .await
        .unwrap();
        child.first_frame_ready().await.unwrap();
        assert_eq!(devices_requested(), 1, "round {round}: no second device");
        assert!(
            total_bytes(&parent) > alone,
            "round {round}: the radial frame is on the parent's ledger"
        );
        let png = child.export_png(1.0).await.unwrap();
        assert!(png.length() > 0);
        drop(child);
        parent.frame().unwrap();
        parent.first_frame_ready().await.unwrap();
        assert!(parent.export_png(1.0).await.unwrap().length() > 0);
    }
    // The JS form: a promise of the chart, disposed with `free()`.
    let promise = FiggyRadialChart::create_on(
        &parent,
        canvas(2),
        "radial".into(),
        document(),
        SIZE.0,
        SIZE.1,
    );
    let child = JsFuture::from(promise).await.unwrap();
    assert!(child.is_object());
    assert_eq!(devices_requested(), 1);
    let free: js_sys::Function = js_sys::Reflect::get(&child, &"free".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    free.call0(&child).unwrap();
    parent.frame().unwrap();
    parent.first_frame_ready().await.unwrap();
    // A family that is not radial, and an invalid document, reject.
    let promise = FiggyRadialChart::create_on(
        &parent,
        canvas(1),
        "radar".into(),
        document(),
        SIZE.0,
        SIZE.1,
    );
    let error = JsFuture::from(promise).await.unwrap_err();
    assert!(message(&error).contains("set_radar_document"), "{error:?}");
    let error = FiggyRadialChart::create_on_device_of(
        &parent,
        canvas(1),
        "radial".into(),
        document(),
        100,
        100,
    )
    .await
    .err()
    .unwrap();
    assert!(message(&error).contains("160x160"), "{error:?}");
    assert_eq!(devices_requested(), 1);
}

/// Disposing the parent while a child's export is pending does not cancel
/// it; dropping a child's pending export and the child leaves the parent
/// working.
#[wasm_bindgen_test(async)]
async fn disposing_either_chart_during_a_pending_export() {
    let parent = FiggyChart::create(canvas(1)).await.unwrap();
    let mut child = FiggyRadialChart::create_on_device_of(
        &parent,
        canvas(1),
        "radial".into(),
        document(),
        SIZE.0,
        SIZE.1,
    )
    .await
    .unwrap();
    let expected = child.export_png(1.0).await.unwrap().to_vec();
    let pending = started(child.export_png(1.0));
    drop(parent);
    let bytes = pending.await.unwrap().to_vec();
    assert_eq!(
        bytes, expected,
        "the parent's disposal left the export whole"
    );
    child.frame().unwrap();
    child.first_frame_ready().await.unwrap();

    let mut parent = FiggyChart::create(canvas(1)).await.unwrap();
    let mut child = FiggyRadialChart::create_on_device_of(
        &parent,
        canvas(1),
        "radial".into(),
        document(),
        SIZE.0,
        SIZE.1,
    )
    .await
    .unwrap();
    let pending = started(child.export_png(2.0));
    drop(pending);
    drop(child);
    parent.frame().unwrap();
    parent.first_frame_ready().await.unwrap();
    assert!(parent.export_png(1.0).await.unwrap().length() > 0);
}

/// A lost device fails the radial child and the Cartesian parent alike.
#[wasm_bindgen_test(async)]
async fn a_device_loss_reaches_the_radial_child_and_the_parent() {
    count_devices();
    let mut parent = FiggyChart::create(canvas(1)).await.unwrap();
    let mut child = FiggyRadialChart::create_on_device_of(
        &parent,
        canvas(1),
        "radial".into(),
        document(),
        SIZE.0,
        SIZE.1,
    )
    .await
    .unwrap();
    child.first_frame_ready().await.unwrap();
    js_sys::eval("globalThis.__devices.last.destroy()").unwrap();
    // The browser reports the loss asynchronously.
    let start = js_sys::Date::now();
    let error = loop {
        match child.export_png(1.0).await {
            Err(error) => break error,
            Ok(_) => {
                assert!(
                    js_sys::Date::now() - start < 10_000.0,
                    "the radial chart never saw the loss"
                );
                turn().await;
            }
        }
    };
    assert!(!message(&error).is_empty(), "{error:?}");
    assert!(
        parent.export_png(1.0).await.is_err(),
        "the parent's export fails on the lost device too"
    );
}

// --- 2. Export background override ---

/// Transparent, colour and document backgrounds at DPR 1 and 2 and at two
/// export scales: only the exported background changes; the document, its
/// selection and a later plain export stay as they were.
#[wasm_bindgen_test(async)]
async fn an_export_background_override_changes_only_the_export() {
    for dpr in [1, 2] {
        let mut chart = radial(dpr).await;
        chart.step_selection(false);
        chart.frame().unwrap();
        let before = chart.get_document().unwrap();
        assert!(read(&chart).interaction.selected.is_some());
        let corner = (0.005, 0.005);
        let slice = (0.5, 0.62);
        for scale in [1.0, 2.0] {
            let plain = chart.export_png(scale).await.unwrap();
            let document = chart
                .export_png_with_background(scale, "document")
                .await
                .unwrap();
            assert_eq!(plain.to_vec(), document.to_vec(), "{dpr} {scale}");
            let transparent = chart
                .export_png_with_background(scale, "transparent")
                .await
                .unwrap();
            let green = chart
                .export_png_with_background(scale, r#"{"r":0,"g":1,"b":0,"a":1}"#)
                .await
                .unwrap();
            let [plain, transparent, green] = [
                pixels(&plain, &[corner, slice]).await,
                pixels(&transparent, &[corner, slice]).await,
                pixels(&green, &[corner, slice]).await,
            ];
            let expected = (SIZE.0 as f32 * scale) as u64;
            for image in [&plain, &transparent, &green] {
                assert_eq!(image["width"], expected, "{dpr} {scale}");
            }
            assert_eq!(plain["at"][0], serde_json::json!([102, 153, 204, 255]));
            assert_eq!(transparent["at"][0][3], 0, "{dpr} {scale}");
            assert_eq!(green["at"][0], serde_json::json!([0, 255, 0, 255]));
            assert_eq!(transparent["at"][1], plain["at"][1], "the slices stay");
            assert_eq!(green["at"][1], plain["at"][1], "the slices stay");
            // The document, its selection and the plain export afterwards.
            assert_eq!(chart.get_document().unwrap(), before);
            let after = chart.export_png(scale).await.unwrap();
            assert_eq!(
                pixels(&after, &[corner, slice]).await,
                plain,
                "{dpr} {scale}"
            );
            chart.frame().unwrap();
        }
        for bad in ["", "white", r#"{"r":2,"g":0,"b":0,"a":1}"#] {
            let error = chart
                .export_png_with_background(1.0, bad)
                .await
                .err()
                .unwrap();
            assert!(!message(&error).is_empty(), "{bad:?}");
        }
        let code = js_sys::Reflect::get(
            &chart
                .export_png_with_background(1.0, "white")
                .await
                .err()
                .unwrap(),
            &"code".into(),
        )
        .unwrap();
        assert_eq!(code.as_string().as_deref(), Some("export_background"));
        assert_eq!(chart.get_document().unwrap(), before);
    }
}

// --- 3. Generated slice labels ---

fn texts(chart: &RadialChart) -> Vec<Option<String>> {
    chart.slices.iter().map(|s| s.text.clone()).collect()
}

/// Opening a document marks its written labels generated; a value, total,
/// format, precision or suffix edit sent back through `set_document`
/// regenerates them, a host's own text stays, and both survive a save and
/// reopen. `regenerate_labels(true)` rewrites the host's text too.
#[wasm_bindgen_test(async)]
async fn generated_labels_follow_an_edit_and_user_text_stays() {
    let mut chart = radial(1).await;
    let opened = read(&chart);
    assert_eq!(
        texts(&opened),
        [Some("A\n25%".into()), Some("B\n75%".into())]
    );
    assert!(opened.slices.iter().all(|s| s.text_generated));

    // A value edit changes the total: both percentages are rewritten.
    let mut edited = opened.clone();
    edited.slices[1].value = 25.0;
    let size = SIZE;
    chart
        .set_document(&serde_json::to_string(&edited).unwrap(), size.0, size.1)
        .unwrap();
    assert_eq!(
        texts(&read(&chart)),
        [Some("A\n50%".into()), Some("B\n50%".into())]
    );

    // A host's own text on A, then a format, precision and suffix edit.
    let mut edited = read(&chart);
    edited.slices[0].set_text(Some("Mine".into()));
    edited.label_format = RadialLabelFormat::ValuePercent;
    edited.label_decimals = 1;
    edited.value_suffix = " kg".into();
    chart
        .set_document(&serde_json::to_string(&edited).unwrap(), size.0, size.1)
        .unwrap();
    let read_back = read(&chart);
    assert_eq!(
        texts(&read_back),
        [Some("Mine".into()), Some("25 kg (50%)".into())]
    );
    assert!(!read_back.slices[0].text_generated);
    assert!(read_back.slices[1].text_generated);

    // Save and reopen: the marker survives, so a later edit still
    // regenerates B and keeps A.
    let saved = chart.get_document().unwrap();
    let mut reopened = FiggyRadialChart::create(canvas(1), "radial", &saved, size.0, size.1)
        .await
        .unwrap();
    assert_eq!(reopened.get_document().unwrap(), saved);
    let mut edited = read(&reopened);
    edited.slices[0].value = 75.0;
    reopened
        .set_document(&serde_json::to_string(&edited).unwrap(), size.0, size.1)
        .unwrap();
    assert_eq!(
        texts(&read(&reopened)),
        [Some("Mine".into()), Some("25 kg (25%)".into())]
    );

    // The explicit rewrite replaces the host's text.
    reopened.regenerate_labels(false).unwrap();
    assert_eq!(texts(&read(&reopened))[0].as_deref(), Some("Mine"));
    reopened.regenerate_labels(true).unwrap();
    let rewritten = read(&reopened);
    assert_eq!(
        texts(&rewritten),
        [Some("75 kg (75%)".into()), Some("25 kg (25%)".into())]
    );
    assert!(rewritten.slices.iter().all(|s| s.text_generated));
    reopened.frame().unwrap();
    assert!(reopened.export_png(1.0).await.unwrap().length() > 0);
}

// --- 4. Decoration position reset ---

/// `reset_decoration_position` removes a dragged decoration's stored
/// offset (not a drag back), reports whether anything changed, and rejects
/// an unknown id and an annotation.
#[wasm_bindgen_test(async)]
async fn a_decoration_position_resets_to_the_layout() {
    let mut chart = radial(1).await;
    let initial = chart.get_document().unwrap();
    assert!(chart.drag("chart_title", 12.0, 8.0, None).unwrap());
    assert!(chart.drag("radial_label_0_1", -6.0, 4.0, None).unwrap());
    assert_eq!(read(&chart).decorations.offsets.len(), 2);
    assert!(chart.reset_decoration_position("chart_title").unwrap());
    let offsets = read(&chart).decorations.offsets;
    assert_eq!(
        offsets.len(),
        1,
        "the title's offset is removed, not zeroed"
    );
    assert!(!chart.reset_decoration_position("chart_title").unwrap());
    assert!(chart.reset_decoration_position("radial_label_0_1").unwrap());
    assert_eq!(chart.get_document().unwrap(), initial);
    assert!(chart.reset_decoration_position("no_such_thing").is_err());
    let item = renderer::Annotation::new(renderer::AnnotationKind::Text {
        anchor: renderer::Anchor::Ratio { x: 0.2, y: 0.8 },
        text: renderer::text::RichText::plain("note", Color::BLACK, 14.0, ""),
        box_style: None,
        align: Default::default(),
    });
    let index = chart
        .add_annotation(&serde_json::to_string(&item).unwrap())
        .unwrap();
    chart.frame().unwrap();
    let id = format!("annotation_{index}");
    assert!(chart.annotations().unwrap().contains(&id), "drawn");
    let error = chart.reset_decoration_position(&id).err().unwrap();
    assert!(message(&error).contains("drag_annotation"), "{error:?}");
    chart.frame().unwrap();
}

// --- 5. Annotation preview (#334) ---

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

fn rect(value: &serde_json::Value) -> [f64; 4] {
    let n = |i: usize| value[i].as_f64().unwrap();
    [n(0), n(1), n(2), n(3)]
}

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1e-3)
}

fn code(error: &JsValue) -> Option<String> {
    js_sys::Reflect::get(error, &JsValue::from_str("code"))
        .ok()
        .and_then(|code| code.as_string())
}

fn note(anchor: renderer::Anchor, text: &str) -> renderer::Annotation {
    renderer::Annotation::new(renderer::AnnotationKind::Text {
        anchor,
        text: renderer::text::RichText::plain(text, Color::BLACK, 14.0, ""),
        box_style: Some(Default::default()),
        align: Default::default(),
    })
}

/// The preview of a text box and of a shape is where the layer draws the
/// item once it is added, in document and canvas pixels at DPR 1 and 2;
/// previewing changes neither the document nor the selection, and an item
/// the radial raster skips reports the reason its warning gives.
#[wasm_bindgen_test(async)]
async fn an_annotation_preview_is_where_the_layer_draws_the_item() {
    use renderer::{Anchor, AnnotationKind};
    for dpr in [1u32, 2] {
        let mut chart = radial(dpr).await;
        // A selected annotation and a picked slice, which a preview keeps.
        let selected = chart
            .add_annotation(
                &serde_json::to_string(&note(Anchor::Ratio { x: 0.1, y: 0.9 }, "kept")).unwrap(),
            )
            .unwrap();
        chart.select_annotation(Some(selected)).unwrap();
        chart.frame().unwrap();
        let shape = renderer::Annotation::new(AnnotationKind::Rect {
            from: Anchor::Ratio { x: 0.55, y: 0.6 },
            to: Anchor::Ratio { x: 0.8, y: 0.3 },
            style: Default::default(),
        });
        let text = note(Anchor::Ratio { x: 0.3, y: 0.25 }, "peak");
        for (item, kind, shape_name) in [(&text, "Text", "text"), (&shape, "Rect", "polygon")] {
            let item = serde_json::to_string(item).unwrap();
            let document = chart.get_document().unwrap();
            let listed = chart.annotations().unwrap();
            let preview = json(&chart.annotation_preview(&item).unwrap());
            assert_eq!(chart.get_document().unwrap(), document, "{dpr}");
            assert_eq!(chart.annotations().unwrap(), listed, "{dpr}");
            assert_eq!(chart.selected_annotation(), Some(selected), "{dpr}");
            assert_eq!(preview["kind"], kind);
            assert_eq!(preview["drawn"], true);
            assert_eq!(preview["visible"], true);
            assert_eq!(preview["canvas"]["shape"], shape_name);
            assert_eq!(preview["document"]["shape"], shape_name);
            let bounds = rect(&preview["document"]["bounds"]);
            assert!(
                close(
                    rect(&preview["canvas"]["bounds"]),
                    bounds.map(|v| v * f64::from(dpr))
                ),
                "{dpr}: {preview}"
            );

            // Added, the drawn item's selection box (document pixels) is
            // the preview's bounds padded by the selection padding, and the
            // item answers a hit inside them.
            let index = chart.add_annotation(&item).unwrap();
            chart.frame().unwrap();
            let drawn = json(&chart.annotations().unwrap());
            let entry = drawn
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["index"] == index)
                .unwrap_or_else(|| panic!("{dpr}: {drawn}"));
            let pad = renderer::select::SELECTION_PADDING as f64;
            let [x, y, w, h] = bounds;
            assert!(
                close(
                    rect(&entry["rect"]),
                    [x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad]
                ),
                "{dpr}: {entry} vs {bounds:?}"
            );
            let center = ((x + w / 2.0) as f32, (y + h / 2.0) as f32);
            assert_eq!(
                chart.hit_annotation(center.0, center.1),
                Some(index),
                "{dpr}"
            );
            // The preview of an item already in the document is unchanged
            // by adding it.
            assert_eq!(
                json(&chart.annotation_preview(&item).unwrap()),
                preview,
                "{dpr}"
            );
            chart.remove_annotation(index).unwrap();
            chart.frame().unwrap();
        }
        assert_eq!(chart.selected_annotation(), Some(selected));

        // The radial chart has no value axes: a reference line is skipped
        // with the reason its warning gives once added.
        let line =
            serde_json::to_string(&renderer::Annotation::new(AnnotationKind::ReferenceLine {
                axis: renderer::AnnotationAxis::X,
                value: 1.0,
                style: Default::default(),
                label: None,
            }))
            .unwrap();
        let skipped = json(&chart.annotation_preview(&line).unwrap());
        assert_eq!(skipped["kind"], "ReferenceLine");
        assert_eq!(skipped["drawn"], false);
        assert_eq!(skipped["reason"], "NoValueAxis");
        assert!(skipped.get("canvas").is_none(), "{skipped}");
        let index = chart.add_annotation(&line).unwrap();
        chart.frame().unwrap();
        let warnings = json(&chart.annotation_warnings().unwrap());
        let warning = warnings
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["index"] == index)
            .unwrap_or_else(|| panic!("{warnings}"));
        assert_eq!(warning["reason"], skipped["reason"], "{warnings}");
        chart.remove_annotation(index).unwrap();

        // A hidden item is placed as it is drawn once shown.
        let mut hidden = text.clone();
        hidden.visible = false;
        let hidden = json(
            &chart
                .annotation_preview(&serde_json::to_string(&hidden).unwrap())
                .unwrap(),
        );
        assert_eq!(hidden["visible"], false);
        assert_eq!(
            hidden["canvas"],
            json(
                &chart
                    .annotation_preview(&serde_json::to_string(&text).unwrap())
                    .unwrap()
            )["canvas"]
        );

        // Malformed input is refused with a code.
        let error = chart.annotation_preview("{").unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("annotation_json"));
        let mut bad = text.clone();
        bad.id = "a\u{7}b".into();
        let error = chart
            .annotation_preview(&serde_json::to_string(&bad).unwrap())
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("annotation_invalid"));
        chart.frame().unwrap();
    }
}
