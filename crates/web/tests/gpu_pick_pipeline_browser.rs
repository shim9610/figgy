#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use renderer::{
    data::Column,
    data_render::{
        ColumnPool, ScatterTransform, create_instance, request_adapter_async, request_device_async,
    },
    gpu_pick::{GpuPickEngine, GpuPickQuery, GpuPickScatter, GpuPickSeriesDescriptor},
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test(async)]
async fn chrome_dawn_creates_and_executes_gpu_pick_pipeline_without_validation_error() {
    let instance = create_instance();
    let adapter = request_adapter_async(&instance)
        .await
        .expect("Chrome/Dawn must expose a WebGPU adapter");
    let (device, queue) = request_device_async(&adapter)
        .await
        .expect("Chrome/Dawn must create a WebGPU device");
    let device = Arc::new(device);
    let queue = Arc::new(queue);

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let picker = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue));
    let validation_error = device.pop_error_scope().await;

    assert!(
        validation_error.is_none(),
        "Chrome/Dawn rejected the GPU pick pipeline: {validation_error:?}"
    );
    let mut picker = picker.expect("GPU picker resource preparation failed");

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let point_count = 65_537usize;
    let mut pool =
        ColumnPool::new(&device, 2 * 1024 * 1024).expect("column pool allocation failed");
    for id in ["browser-direct-x", "browser-direct-y"] {
        pool.add_column(
            id.into(),
            &Column {
                data: vec![0.0],
                min: 0.0,
                max: 0.0,
            },
            &device,
            &queue,
        )
        .expect("browser direct-scan column upload failed");
    }
    pool.add_column(
        "browser-pick-x".into(),
        &Column {
            data: (0..point_count).map(|index| index as f32).collect(),
            min: 0.0,
            max: (point_count - 1) as f32,
        },
        &device,
        &queue,
    )
    .expect("browser pick x-column upload failed");
    pool.add_column(
        "browser-pick-y".into(),
        &Column {
            data: vec![5.0_f32; point_count],
            min: 5.0,
            max: 5.0,
        },
        &device,
        &queue,
    )
    .expect("browser pick y-column upload failed");
    picker
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("browser-direct-source".into()),
                series_id: "browser-direct-series".into(),
                x_column: "browser-direct-x".into(),
                y_column: "browser-direct-y".into(),
                scatter: Some(GpuPickScatter {
                    base_radius_px: 0.0001,
                    base_shape_id: 0,
                    style_map: None,
                }),
                line_width_px: None,
            },
        )
        .expect("browser direct-scan preparation failed");
    picker
        .add_series(
            &pool,
            GpuPickSeriesDescriptor {
                source_id: Some("browser-source".into()),
                series_id: "browser-series".into(),
                x_column: "browser-pick-x".into(),
                y_column: "browser-pick-y".into(),
                scatter: Some(GpuPickScatter {
                    // 65,537 points are projected into 100 px. Keep the
                    // visible radius below the neighbour spacing so only the
                    // exact centre point participates in the zero-distance tie.
                    base_radius_px: 0.0001,
                    base_shape_id: 0,
                    style_map: None,
                }),
                line_width_px: None,
            },
        )
        .expect("browser pick gate preparation failed");

    let picked = picker
        .pick(
            &pool,
            GpuPickQuery {
                transform: ScatterTransform {
                    data_min: [0.0, 0.0],
                    data_max: [(point_count - 1) as f32, 10.0],
                    data_min_lo: [0.0; 2],
                    data_max_lo: [0.0; 2],
                    scale_log: [0.0; 2],
                    pixel_to_ndc: [0.02; 2],
                    style_params: [[0.0; 4]; 3],
                },
                chart_rect_px: [0.0, 0.0, 100.0, 100.0],
                data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
                canvas_position_px: [50.0, 50.0],
                max_distance_px: 0.0,
            },
        )
        .expect("browser pick submission failed")
        .resolve()
        .await
        .expect("browser pick readback failed")
        .expect("known visible point was not picked");
    let execution_error = device.pop_error_scope().await;

    assert!(
        execution_error.is_none(),
        "Chrome/Dawn rejected GPU pick execution: {execution_error:?}"
    );
    assert_eq!(picked.source_id.as_deref(), Some("browser-source"));
    assert_eq!(picked.series_id, "browser-series");
    assert_eq!(picked.point_index, point_count / 2);
    assert_eq!(picked.distance_px, 0.0);
}
