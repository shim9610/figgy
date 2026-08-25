#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use renderer::data::Column;
use renderer::data_config::{
    ContourConfig, DataLineStyleConfig, FieldFillConfig, FillMode, GridLayout, MatrixOrientation,
    MatrixRef, Shading,
};
use renderer::data_render::{create_instance, request_adapter_async, request_device_async};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, Color, ColorMap, DataRenderType, RasterImage, Renderer, RendererDevice, SeriesConfig,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SIZE: u32 = 400;

fn column(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

async fn renderer() -> Renderer {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("Chrome/Dawn must expose a WebGPU adapter");
    let (device, queue) = request_device_async(&adapter)
        .await
        .expect("Chrome/Dawn must create a WebGPU device");
    Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Bgra8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer initialization failed")
}

fn colorbar() -> renderer::ColorBarOptions {
    let mut bar = renderer::default::default_colorbar_options();
    bar.colormap = ColorMap::Custom {
        stops: vec![
            Color::new(1.0, 0.0, 0.0, 1.0),
            Color::new(0.0, 1.0, 0.0, 1.0),
        ],
    };
    bar.axis.min = 0.0;
    bar.axis.max = 4.0;
    bar.axis.major_spacing = 1.0;
    bar
}

fn chart(x_max: f64, y_max: f64) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: SIZE,
        height: SIZE,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.colorbar = Some(colorbar());
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, x_max);
    chart.set_y_range(0.0, y_max);
    chart
}

fn matrix(columns: &[&str], grid_layout: GridLayout) -> MatrixRef {
    MatrixRef {
        columns: columns.iter().map(|id| (*id).to_owned()).collect(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout,
    }
}

fn contour_series(levels: Vec<f64>, colors: Vec<Color>) -> SeriesConfig {
    SeriesConfig {
        series_id: "browser-contour".into(),
        source_id: None,
        label: None,
        x_column: "gx".into(),
        y_column: "gy".into(),
        render_type: DataRenderType::Contour {
            matrix: matrix(&["p0", "p1", "p2"], GridLayout::Centers),
            contour: ContourConfig {
                levels,
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(0.0, 0.0, 1.0, 1.0),
                    line_width: 3.0,
                },
                per_level_color: Some(colors),
                labels: None,
            },
        },
    }
}

fn band_series(levels: Vec<f64>) -> SeriesConfig {
    SeriesConfig {
        series_id: "browser-bands".into(),
        source_id: None,
        label: None,
        x_column: "bx".into(),
        y_column: "by".into(),
        render_type: DataRenderType::HeatmapContour {
            matrix: matrix(&["bz"], GridLayout::Edges),
            fill: FieldFillConfig {
                mode: FillMode::Bands,
                shading: Shading::Flat,
                opacity: 1.0,
            },
            contour: ContourConfig {
                levels,
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::new(0.0, 0.0, 0.0, 0.0),
                    line_width: 0.0,
                },
                per_level_color: None,
                labels: None,
            },
        },
    }
}

fn pixel(image: &RasterImage, x: u32, y: u32) -> &[u8] {
    let offset = ((y * image.width + x) * 4) as usize;
    &image.rgba[offset..offset + 4]
}

fn data_area_center(chart: &Chart) -> (u32, u32) {
    let area = chart.config().data_area().expect("data area");
    (area.x + area.width / 2, area.y + area.height / 2)
}

fn equivalent_layer(layer: Color, count: usize) -> Color {
    let source = [
        layer.r * layer.a,
        layer.g * layer.a,
        layer.b * layer.a,
        layer.a,
    ];
    let mut premul = [0.0f32; 4];
    for _ in 0..count {
        for channel in 0..3 {
            premul[channel] = source[channel] + premul[channel] * (1.0 - source[3]);
        }
        premul[3] = source[3] + premul[3] * (1.0 - source[3]);
    }
    Color::new(
        premul[0] / premul[3],
        premul[1] / premul[3],
        premul[2] / premul[3],
        premul[3],
    )
}

#[wasm_bindgen_test(async)]
async fn chrome_webgpu_contour_lookup_is_portable_and_complete() {
    let mut renderer = renderer().await;
    renderer
        .add_columns(&[
            (
                "gx",
                &column(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "gy",
                &column(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p0",
                &column(vec![0.0, 1.0, 2.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p1",
                &column(vec![1.0, 2.0, 3.0]) as &dyn renderer::ColumnSource,
            ),
            (
                "p2",
                &column(vec![2.0, 3.0, 4.0]) as &dyn renderer::ColumnSource,
            ),
            ("bx", &column(vec![0.0, 1.0]) as &dyn renderer::ColumnSource),
            ("by", &column(vec![0.0, 1.0]) as &dyn renderer::ColumnSource),
            ("bz", &column(vec![1.0]) as &dyn renderer::ColumnSource),
        ])
        .expect("browser contour fixtures must upload");

    let contour_chart = chart(2.0, 2.0);
    let marker = Color::new(0.0, 1.0, 1.0, 1.0);
    let mut levels = vec![99.0; renderer::MAX_CONTOUR_LEVELS];
    levels[1023] = 2.0;
    let mut colors = vec![Color::new(0.0, 0.0, 0.0, 1.0); renderer::MAX_CONTOUR_LEVELS];
    colors[1023] = marker;
    let indexed = renderer
        .export_panel_rgba_async(&contour_chart, &[contour_series(levels, colors)], 1.0)
        .await
        .expect("index-1023 contour export");
    let indexed_reference = renderer
        .export_panel_rgba_async(
            &contour_chart,
            &[contour_series(vec![2.0], vec![marker])],
            1.0,
        )
        .await
        .expect("index-1023 reference export");
    assert_eq!(indexed.rgba, indexed_reference.rgba);

    let finite = Color::new(0.0, 0.0, 1.0, 0.75);
    let non_finite = renderer
        .export_panel_rgba_async(
            &contour_chart,
            &[contour_series(
                vec![f64::NAN, f64::NEG_INFINITY, f64::INFINITY, 2.0],
                vec![Color::WHITE, Color::WHITE, Color::WHITE, finite],
            )],
            1.0,
        )
        .await
        .expect("non-finite contour export");
    let finite_reference = renderer
        .export_panel_rgba_async(
            &contour_chart,
            &[contour_series(vec![2.0], vec![finite])],
            1.0,
        )
        .await
        .expect("finite contour reference export");
    assert_eq!(non_finite.rgba, finite_reference.rgba);

    let bands_chart = chart(1.0, 1.0);
    let mixed_bands = renderer
        .export_panel_rgba_async(
            &bands_chart,
            &[band_series(vec![
                f64::NAN,
                f64::NEG_INFINITY,
                f64::INFINITY,
                0.5,
            ])],
            1.0,
        )
        .await
        .expect("mixed non-finite bands export");
    let finite_bands = renderer
        .export_panel_rgba_async(&bands_chart, &[band_series(vec![0.0, 0.5, 2.0, 3.0])], 1.0)
        .await
        .expect("finite bands reference export");
    assert_eq!(mixed_bands.rgba, finite_bands.rgba);

    let layer = Color::new(0.8, 0.2, 0.1, 1.0 / 2048.0);
    let expanded = renderer
        .export_panel_rgba_async(
            &contour_chart,
            &[contour_series(
                vec![2.0; renderer::MAX_CONTOUR_LEVELS],
                vec![layer; renderer::MAX_CONTOUR_LEVELS],
            )],
            1.0,
        )
        .await
        .expect("1024-layer contour export");
    let complete = renderer
        .export_panel_rgba_async(
            &contour_chart,
            &[contour_series(
                vec![2.0],
                vec![equivalent_layer(layer, renderer::MAX_CONTOUR_LEVELS)],
            )],
            1.0,
        )
        .await
        .expect("1024-layer contour reference export");
    let truncated = renderer
        .export_panel_rgba_async(
            &contour_chart,
            &[contour_series(
                vec![2.0],
                vec![equivalent_layer(layer, 621)],
            )],
            1.0,
        )
        .await
        .expect("621-layer contour reference export");
    let center = data_area_center(&contour_chart);
    assert_eq!(
        pixel(&expanded, center.0, center.1),
        pixel(&complete, center.0, center.1)
    );
    assert_ne!(
        pixel(&expanded, center.0, center.1),
        pixel(&truncated, center.0, center.1),
        "alpha 1/2048 must distinguish 1024 composites from 621"
    );
}
