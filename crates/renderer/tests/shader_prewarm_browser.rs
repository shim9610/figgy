//! Real WASM initialization, pipeline reuse and labelled-contour pixel checks.
#![cfg(target_arch = "wasm32")]

use renderer::data_config::{
    ContourConfig, ContourLabelConfig, DataLineStyleConfig, GridLayout, MatrixOrientation,
    MatrixRef,
};
use renderer::{Chart, Color, DataRenderType, Renderer, SeriesConfig};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
let restore = [];
let calls = 0;
export function begin_compile_count() {
  calls = 0;
  for (const name of ['createShaderModule','createComputePipeline','createRenderPipeline','createComputePipelineAsync','createRenderPipelineAsync']) {
    const previous = GPUDevice.prototype[name];
    GPUDevice.prototype[name] = function(...args) { calls++; return previous.apply(this,args); };
    restore.push(() => GPUDevice.prototype[name] = previous);
  }
}
export function compile_count() { return calls; }
export function end_compile_count() { for(const f of restore.reverse()) f(); restore=[]; }
export function chart_canvas() {
  const canvas=document.createElement('canvas'); canvas.width=384; canvas.height=256;
  document.body.append(canvas); return canvas;
}
export function now_ms() { return performance.now(); }
"#)]
extern "C" {
    fn begin_compile_count();
    fn compile_count() -> u32;
    fn end_compile_count();
    fn chart_canvas() -> JsValue;
    fn now_ms() -> f64;
}

struct Counting;
impl Drop for Counting {
    fn drop(&mut self) {
        end_compile_count();
    }
}

fn column(data: Vec<f64>) -> renderer::data::Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    renderer::data::Column { data, min, max }
}

#[wasm_bindgen_test(async)]
async fn real_prewarm_reuses_pipelines_and_draws_contour_labels() {
    begin_compile_count();
    let _counting = Counting;
    let start = now_ms();
    let mut renderer = Renderer::for_window_async(
        wgpu::SurfaceTarget::Canvas(chart_canvas().unchecked_into()),
        (384, 256),
        64 * 1024,
    )
    .await
    .expect("actual browser renderer initialization");
    let mut stages = Vec::new();
    renderer
        .prewarm_all_observed(&mut |event| {
            if event.scope == "contour.label.async" {
                stages.push((event.stage, event.phase));
            }
        })
        .await
        .expect("complete prewarm");
    assert_eq!(
        stages,
        vec![
            ("anchor_project", renderer::InitPhase::Started),
            ("anchor_project", renderer::InitPhase::Finished),
            ("anchor_select", renderer::InitPhase::Started),
            ("anchor_select", renderer::InitPhase::Finished),
        ],
        "split compilation must retain the existing progress stages"
    );
    console_log!("SHADER_PREWARM total_ms={}", now_ms() - start);
    let calls = compile_count();
    assert!(calls > 50, "must exercise the real pipeline constructors");
    renderer.prewarm_all().await.unwrap();
    assert_eq!(
        compile_count(),
        calls,
        "cached prewarm must not re-create any shader or pipeline"
    );

    renderer::text_render::register_font_bytes(
        include_bytes!("../fonts/LiberationSans-Regular.ttf").to_vec(),
    )
    .unwrap();
    renderer
        .add_column("x", &column(vec![0., 0.5, 1.]))
        .unwrap();
    renderer
        .add_column("y", &column(vec![0., 0.5, 1.]))
        .unwrap();
    for (id, z) in [
        ("z0", vec![0., 0., 0.]),
        ("z1", vec![0., 1., 2.]),
        ("z2", vec![0., 2., 4.]),
    ] {
        renderer.add_column(id, &column(z)).unwrap();
    }
    let mut config = renderer::default::default_config();
    config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 0,
        y: 0,
        width: 384,
        height: 256,
    });
    config.legend.visible = false;
    config.colorbar = Some(renderer::default::default_colorbar_options());
    let mut chart = Chart::new(config);
    chart.set_x_range(0., 1.);
    chart.set_y_range(0., 1.);
    chart.config_mut().bottom_x.major_spacing = 0.25;
    chart.config_mut().left_y.major_spacing = 0.25;
    let series = SeriesConfig {
        series_id: "contour".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Contour {
            matrix: MatrixRef {
                columns: vec!["z0".into(), "z1".into(), "z2".into()],
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            contour: ContourConfig {
                levels: vec![0.8, 1.6, 2.4],
                line: DataLineStyleConfig {
                    line_style: renderer::line::LineStylePreset::Solid,
                    line_color: Color::new(0., 0.2, 0.8, 1.),
                    line_width: 2.,
                },
                per_level_color: None,
                labels: Some(ContourLabelConfig {
                    visible: true,
                    font_size: 18.,
                    color: Color::BLACK,
                    format: renderer::format::LabelFormat::Decimal,
                    significant_digits: 3,
                    spacing_px: 45.,
                    anchors: vec![],
                    bg_color: Some(Color::new(1., 0., 1., 1.)),
                    bg_padding_px: 2.,
                }),
            },
        },
    };
    let image = renderer
        .export_panel_rgba_async(&chart, &[series], 1.)
        .await
        .expect("pixel readback");
    assert!(
        image
            .rgba
            .chunks_exact(4)
            .any(|p| p[0] > 200 && p[1] < 100 && p[2] > 200),
        "automatic label pixels missing"
    );
    assert!(
        image
            .rgba
            .chunks_exact(4)
            .any(|p| p[0] < 80 && p[1] < 120 && p[2] > 150),
        "contour trajectory pixels missing"
    );
    let encoded = renderer::encode_png(&image).unwrap();
    let mut decoder = png::Decoder::new(std::io::Cursor::new(encoded))
        .read_info()
        .unwrap();
    let mut decoded = vec![0; decoder.output_buffer_size().unwrap()];
    let frame = decoder.next_frame(&mut decoded).unwrap();
    assert_eq!((frame.width, frame.height), (image.width, image.height));
    assert_eq!(
        &decoded[..frame.buffer_size()],
        image.rgba.as_slice(),
        "PNG pixels changed"
    );
}
