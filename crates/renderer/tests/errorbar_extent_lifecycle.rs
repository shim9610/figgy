use std::sync::{Arc, OnceLock};

use renderer::data_render::{create_instance, request_adapter, request_device};
use renderer::{
    Column, GpuErrorbarError, GpuSeriesExtentColumnIds, GpuSeriesExtentMode, Renderer,
    RendererDevice,
};

fn col_f64(data: Vec<f64>) -> Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

fn shared_device() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    static DEVICE: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let instance = create_instance();
            let adapter = request_adapter(&instance).ok()?;
            let (device, queue) = request_device(&adapter).ok()?;
            Some((Arc::new(device), Arc::new(queue)))
        })
        .as_ref()
        .map(|(device, queue)| (Arc::clone(device), Arc::clone(queue)))
}

fn try_renderer() -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Bgra8Unorm,
        1024 * 1024,
    )
    .ok()
}

fn point_columns() -> GpuSeriesExtentColumnIds<'static> {
    GpuSeriesExtentColumnIds {
        x: "x",
        y: "y",
        x_lower: None,
        x_upper: None,
        y_lower: None,
        y_upper: None,
    }
}

#[test]
fn explicit_preparation_publishes_engine_for_shared_submission() {
    let Some(mut renderer) = try_renderer() else {
        eprintln!("no GPU adapter; skipping extent lifecycle assertions");
        return;
    };
    renderer
        .add_hilo_column("x", &col_f64(vec![1.0, 2.0]))
        .unwrap();
    renderer
        .add_hilo_column("y", &col_f64(vec![10.0, 20.0]))
        .unwrap();

    assert!(!renderer.errorbar_extent_engine_ready());
    renderer.prepare(&[]).unwrap();
    assert!(!renderer.errorbar_extent_engine_ready());
    assert!(matches!(
        renderer.begin_errorbar_extent("x", "x", "x"),
        Err(GpuErrorbarError::EngineNotReady)
    ));
    assert!(matches!(
        renderer.begin_series_extent(GpuSeriesExtentMode::Points, point_columns()),
        Err(GpuErrorbarError::EngineNotReady)
    ));

    {
        let provisional = renderer
            .begin_upsert_hilo_column("x", &col_f64(vec![100.0, 200.0]))
            .unwrap();
        assert!(matches!(
            provisional.begin_series_extent(GpuSeriesExtentMode::Points, point_columns()),
            Err(GpuErrorbarError::EngineNotReady)
        ));
    }
    assert!(!renderer.errorbar_extent_engine_ready());

    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
    assert!(renderer.errorbar_extent_engine_ready());
    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

    let shared_renderer = &renderer;
    let extent = pollster::block_on(
        shared_renderer
            .begin_series_extent(GpuSeriesExtentMode::Points, point_columns())
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((extent.x.min, extent.x.max), (1.0, 2.0));
    assert_eq!((extent.y.min, extent.y.max), (10.0, 20.0));
}

#[test]
fn prepared_engine_reads_provisional_upsert_without_publishing_rollback() {
    let Some(mut renderer) = try_renderer() else {
        eprintln!("no GPU adapter; skipping provisional extent assertions");
        return;
    };
    renderer
        .add_hilo_column("x", &col_f64(vec![1.0, 2.0]))
        .unwrap();
    renderer
        .add_hilo_column("y", &col_f64(vec![10.0, 20.0]))
        .unwrap();
    pollster::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

    let provisional_ticket = {
        let provisional = renderer
            .begin_upsert_hilo_column("x", &col_f64(vec![100.0, 200.0]))
            .unwrap();
        provisional
            .begin_series_extent(GpuSeriesExtentMode::Points, point_columns())
            .unwrap()
    };
    let provisional = pollster::block_on(provisional_ticket.resolve())
        .unwrap()
        .unwrap();
    assert_eq!((provisional.x.min, provisional.x.max), (100.0, 200.0));

    let rolled_back = pollster::block_on(
        renderer
            .begin_series_extent(GpuSeriesExtentMode::Points, point_columns())
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((rolled_back.x.min, rolled_back.x.max), (1.0, 2.0));
}
