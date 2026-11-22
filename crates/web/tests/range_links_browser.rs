//! Range links (#357) through the public WASM API: `fit_ranges` fits a chart
//! with links as its bars alone (#405), and a cold frame whose bars still
//! compile while the links are ready does not fail; the links appear once
//! the bars do (#406).
#![cfg(target_arch = "wasm32")]

use figgy::FiggyChart;
use renderer::data_config::{BarGroups, CategoryBarDirection, CategoryBarMode};
use renderer::{Color, DataRenderType, SeriesConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

mod support;
use support::{canvas_320x240 as canvas, turn};

wasm_bindgen_test_configure!(run_in_browser);

const RANGE_BAR_PIPELINE: &str = "figgy range bar columnar pipeline";

/// Hold every async compile of `label` until `globalThis.__releaseHeld()`
/// is called; every other compile runs as usual.
fn hold(label: &str) {
    js_sys::eval(&format!(
        r#"(() => {{
  const P = GPUDevice.prototype;
  if (!P.__figgyOriginalAsync) P.__figgyOriginalAsync = P.createRenderPipelineAsync;
  const original = P.__figgyOriginalAsync;
  let release;
  const gate = new Promise((resolve) => {{ release = resolve; }});
  globalThis.__held = 0;
  globalThis.__releaseHeld = () => release();
  P.createRenderPipelineAsync = function (d) {{
    if (d.label === {label:?}) {{
      globalThis.__held += 1;
      return gate.then(() => original.call(this, d));
    }}
    return original.call(this, d);
  }};
}})()"#
    ))
    .unwrap();
}

/// Put the original `createRenderPipelineAsync` back.
fn unhook() {
    js_sys::eval(
        "(() => { const P = GPUDevice.prototype; \
         if (P.__figgyOriginalAsync) P.createRenderPipelineAsync = P.__figgyOriginalAsync; })()",
    )
    .unwrap();
}

/// Horizontal bars over a numeric position axis.
fn config(chart: &FiggyChart) -> serde_json::Value {
    let mut config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    config["bar_groups"] = serde_json::to_value(BarGroups {
        mode: CategoryBarMode::Grouped,
        direction: CategoryBarDirection::Horizontal,
        group_width: 0.8,
        bar_gap: 4.0,
    })
    .unwrap();
    config["legend"]["visible"] = false.into();
    config["bottom_x"]["min"] = (-1.0).into();
    config["bottom_x"]["max"] = 9.0.into();
    config["left_y"]["min"] = (-0.5).into();
    config["left_y"]["max"] = 1.5.into();
    config
}

/// The two range bars of the issues (positions 0 and 1, from 0 to 2 and
/// from 5 to 8) and a link from the first to the second.
fn show(chart: &mut FiggyChart, with_links: bool) {
    for (id, data) in [
        ("pos", [0.0, 1.0]),
        ("start", [0.0, 5.0]),
        ("end", [2.0, 8.0]),
    ] {
        chart.register_column_f64(id, &data).unwrap();
    }
    chart.set_config(&config(chart).to_string()).unwrap();
    let bars = SeriesConfig::range_bars("bars", "pos", "start", "end")
        .color(Color::from_rgb8(0, 255, 0))
        .style_range_bars(|bars| {
            bars.style.material = renderer::Material::Flat;
            bars.style.outline = false;
            bars.style.corner_radius = 0.0;
            bars.slot_width = Some(1.0);
        });
    let mut links = SeriesConfig::range_links("links", "bars", [(0, 1)]);
    if let DataRenderType::RangeLinks(style) = &mut links.render_type {
        style.color = Color::from_rgb8(255, 128, 0);
        style.width = 3.0;
    }
    assert!(links.x_column.is_empty() && links.y_column.is_empty());
    let series = if with_links {
        vec![bars, links]
    } else {
        vec![bars]
    };
    chart
        .set_series(&serde_json::to_string(&series).unwrap())
        .unwrap();
}

fn ranges(chart: &FiggyChart) -> [f64; 4] {
    let config: serde_json::Value = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    [
        config["bottom_x"]["min"].as_f64().unwrap(),
        config["bottom_x"]["max"].as_f64().unwrap(),
        config["left_y"]["min"].as_f64().unwrap(),
        config["left_y"]["max"].as_f64().unwrap(),
    ]
}

/// The RGBA pixels of a PNG, decoded by the browser.
async fn rgba(source: &wasm_bindgen::JsValue) -> Vec<u8> {
    let read: js_sys::Function = js_sys::eval(
        r#"(async (source) => {
  const bitmap = await createImageBitmap(new Blob([source], { type: 'image/png' }),
    { premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const g = canvas.getContext('2d');
  g.drawImage(bitmap, 0, 0);
  return new Uint8Array(g.getImageData(0, 0, bitmap.width, bitmap.height).data.buffer);
})"#,
    )
    .unwrap()
    .dyn_into()
    .unwrap();
    let promise: js_sys::Promise = read
        .call1(&wasm_bindgen::JsValue::NULL, source)
        .unwrap()
        .dyn_into()
        .unwrap();
    let bytes = wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    js_sys::Uint8Array::new(&bytes).to_vec()
}

fn count(pixels: &[u8], inked: fn(&[u8]) -> bool) -> usize {
    pixels.chunks_exact(4).filter(|p| inked(p)).count()
}

fn is_green(p: &[u8]) -> bool {
    p[1] > 200 && p[0] < 60 && p[2] < 60
}

fn is_orange(p: &[u8]) -> bool {
    p[0] > 200 && (90..170).contains(&p[1]) && p[2] < 60
}

/// #405: `fit_ranges` on a chart with links succeeds and fits the axes as
/// the bars alone do.
#[wasm_bindgen_test(async)]
async fn fit_ranges_with_range_links_is_the_fit_of_their_bars() {
    let mut expected = FiggyChart::create(canvas()).await.unwrap();
    show(&mut expected, false);
    expected.fit_ranges().unwrap();

    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show(&mut chart, true);
    chart.fit_ranges().expect("fit_ranges with range links");
    assert_eq!(ranges(&chart), ranges(&expected));
    assert_ne!(ranges(&chart), [-1.0, 9.0, -0.5, 1.5], "the fit moved");
    chart.first_frame_ready().await.unwrap();
}

/// #406: bars and links installed in one `set_series`, with the bars'
/// pipeline still compiling: frames leave the bars out without failing,
/// and once the compile resolves the first-frame wait settles with both.
#[wasm_bindgen_test(async)]
async fn a_cold_frame_with_links_ready_before_their_bars_does_not_fail() {
    hold(RANGE_BAR_PIPELINE);
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    show(&mut chart, true);
    // Draw until the held compile is asked for: every frame meanwhile
    // succeeds and leaves the bars out.
    let start = js_sys::Date::now();
    loop {
        chart.frame().expect("a frame without the links' bars");
        let held = js_sys::eval("globalThis.__held").unwrap().as_f64().unwrap();
        if held > 0.0 {
            break;
        }
        assert!(
            js_sys::Date::now() - start < 120_000.0,
            "the range bar compile was never issued"
        );
        turn().await;
    }
    chart.frame().expect("a frame while the bars compile");
    assert!(!chart.frame_complete(), "the bars are left out");

    js_sys::eval("globalThis.__releaseHeld()").unwrap();
    chart
        .first_frame_ready()
        .await
        .expect("the first frame settles with the bars and the links");
    assert!(chart.frame_complete());
    unhook();

    let png = chart.export_png(1.0).await.unwrap();
    let pixels = rgba(&png.into()).await;
    assert!(count(&pixels, is_green) > 0, "the bars draw");
    assert!(count(&pixels, is_orange) > 0, "the links draw");
}
