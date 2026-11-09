#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::{
    Column, GpuExtentError, GpuResourceKind, GpuSeriesExtentColumnIds, GpuSeriesExtentMode,
    Renderer, RendererDevice,
};

fn col_f64(data: Vec<f64>) -> Column<f64> {
    Column { data }
}

fn shared_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    static DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let (device, queue) = test_gpu::device();
            Some((device, queue))
        })
        .as_ref()
        .map(|(device, queue)| (device.clone(), queue.clone()))
}

fn try_renderer() -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Some(
        Renderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        )
        .expect("required GPU test: renderer initialization failed"),
    )
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
    renderer.prepare(&mut []).unwrap();
    assert!(!renderer.errorbar_extent_engine_ready());
    assert!(matches!(
        renderer.begin_errorbar_extent("x", "x", "x"),
        Err(GpuExtentError::EngineNotReady)
    ));
    assert!(matches!(
        renderer.begin_series_extent(GpuSeriesExtentMode::Points, point_columns()),
        Err(GpuExtentError::EngineNotReady)
    ));

    {
        let provisional = renderer
            .begin_upsert_hilo_column("x", &col_f64(vec![100.0, 200.0]))
            .unwrap();
        assert!(matches!(
            provisional.begin_series_extent(GpuSeriesExtentMode::Points, point_columns()),
            Err(GpuExtentError::EngineNotReady)
        ));
    }
    assert!(!renderer.errorbar_extent_engine_ready());

    test_gpu::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();
    assert!(renderer.errorbar_extent_engine_ready());
    test_gpu::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

    let shared_renderer = &renderer;
    // A live ticket owns reduction scratch and a MAP_READ buffer; both are
    // charged for exactly as long as the ticket holds them.
    let idle = renderer.gpu_memory_usage();
    assert_eq!(idle.live_bytes_of(GpuResourceKind::ErrorbarScratch), 0);
    let ticket = shared_renderer
        .begin_series_extent(GpuSeriesExtentMode::Points, point_columns())
        .unwrap();
    let pending = renderer.gpu_memory_usage();
    assert!(
        pending.live_bytes_of(GpuResourceKind::ErrorbarScratch) > 0
            && pending.live_bytes_of(GpuResourceKind::Readback) > 0,
        "extent scratch and readback must be charged while a ticket is live\n{}",
        pending.report()
    );

    let extent = test_gpu::block_on(ticket.resolve()).unwrap().unwrap();
    assert_eq!((extent.x.min, extent.x.max), (1.0, 2.0));
    assert_eq!((extent.y.min, extent.y.max), (10.0, 20.0));

    renderer.end_gpu_frame();
    let settled = renderer.gpu_memory_usage();
    assert_eq!(
        settled.live_bytes_of(GpuResourceKind::ErrorbarScratch),
        0,
        "resolving the ticket releases its scratch\n{}",
        settled.report()
    );
    assert_eq!(settled.live_bytes_of(GpuResourceKind::Readback), 0);
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
    test_gpu::block_on(renderer.ensure_errorbar_extent_engine()).unwrap();

    let provisional_ticket = {
        let provisional = renderer
            .begin_upsert_hilo_column("x", &col_f64(vec![100.0, 200.0]))
            .unwrap();
        provisional
            .begin_series_extent(GpuSeriesExtentMode::Points, point_columns())
            .unwrap()
    };
    let provisional = test_gpu::block_on(provisional_ticket.resolve())
        .unwrap()
        .unwrap();
    assert_eq!((provisional.x.min, provisional.x.max), (100.0, 200.0));

    let rolled_back = test_gpu::block_on(
        renderer
            .begin_series_extent(GpuSeriesExtentMode::Points, point_columns())
            .unwrap()
            .resolve(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((rolled_back.x.min, rolled_back.x.max), (1.0, 2.0));
}
