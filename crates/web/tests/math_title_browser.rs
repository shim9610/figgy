#![cfg(target_arch = "wasm32")]

//! A math title written with `rich_text_from_tex` (#45) is set through
//! `set_config`, laid out from the bundled math face (no font is
//! registered), drawn, hit and exported; a malformed tree is refused before
//! the chart changes.

use figgy::{FiggyChart, rich_text_from_tex};
use renderer::Config;
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
    canvas.set_height(360);
    canvas
}

#[wasm_bindgen_test(async)]
async fn a_math_title_draws_hits_and_exports_with_the_bundled_font() {
    let mut chart = FiggyChart::create(canvas()).await.unwrap();
    let mut config: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    let title = rich_text_from_tex(
        r"$\sum_{i=1}^{n} \frac{x_i^2}{\sigma}$",
        18.0,
        "",
        0.0,
        0.0,
        0.0,
        1.0,
    )
    .unwrap();
    config.chart_title.text = serde_json::from_str(&title).unwrap();
    config.chart_title.top_margin = 64.0;
    chart
        .set_config(&serde_json::to_string(&config).unwrap())
        .unwrap();
    chart.frame().unwrap();

    let bounds: serde_json::Value =
        serde_json::from_str(&chart.decoration_bounds("chart_title").unwrap()).unwrap();
    let rect = |i: usize| bounds["rect"][i].as_f64().unwrap();
    let (x, y, w, h) = (rect(0), rect(1), rect(2), rect(3));
    assert!(w > 20.0 && h > 18.0, "{bounds}");
    let hit = chart.hit_test((x + w / 2.0) as f32, (y + h / 2.0) as f32);
    assert_eq!(hit.as_deref(), Some("chart_title"));

    let png = chart.export_png(2.0).await.unwrap();
    assert!(png.length() > 0);

    // A malformed tree fails at parse: the chart keeps its title.
    let json = serde_json::to_string(&config).unwrap();
    let broken = json.replacen(
        r#""math_parent":{"id":0,"slot":0}"#,
        r#""math_parent":{"id":9,"slot":0}"#,
        1,
    );
    assert!(chart.set_config(&broken).is_err());
    let kept: Config = serde_json::from_str(&chart.get_config().unwrap()).unwrap();
    assert_eq!(kept.chart_title.text, config.chart_title.text);
    assert!(rich_text_from_tex(r"$\frac{a}$", 12.0, "", 0.0, 0.0, 0.0, 1.0).is_err());
}
