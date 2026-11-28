//! The host-facing wrappers' frame loop and device loss (#469), built from
//! the reproduction file of the bug sweep (`sweep_window.rs`).
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, ChartDrawItem, ChartId, Config, FiggyError, GpuPickError, GpuPickRequest,
    RegisteredChartDrawItem, Renderer, RendererDevice, Series,
};

fn bare(width: u32, height: u32) -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 100.0);
    chart.config().clone()
}

fn renderer_on(gpu: RendererDevice) -> Renderer {
    let mut r = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, 4 * 1024 * 1024).unwrap();
    r.enable_gpu_picking().unwrap();
    r
}

const STORED: Rect = Rect {
    x: 0,
    y: 0,
    width: 400,
    height: 300,
};

/// The request of a pick at the chart's centre, drawn as stored.
fn centre_request() -> GpuPickRequest {
    GpuPickRequest {
        canvas_position_px: [200.0, 150.0],
        display_panel_px: STORED,
        display_scale: 1.0,
        max_distance_px: 4.0,
    }
}

fn registered(r: &mut Renderer) -> ChartId {
    harness::upload(r, "x", vec![5.0]);
    harness::upload(r, "y", vec![50.0]);
    let id = r
        .register_chart(
            bare(400, 300),
            vec![harness::markers("m", "x", "y", harness::RED)],
        )
        .unwrap();
    let chart = Chart::new(bare(400, 300));
    let mut view = r.create_chart_view(&chart, STORED).unwrap();
    r.prepare_registered(&mut [RegisteredChartDrawItem {
        chart_id: id,
        view: &mut view,
    }])
    .unwrap();
    id
}

/// The reproduction of #469 item 9: after the device is lost, every pick
/// call reports `GpuPickError::DeviceLost`, typed as the renderer's other
/// calls report `FiggyError::DeviceLost`, instead of a `Gpu(String)`. The
/// typed data pick's submit, whose error is a `FiggyError`, reports
/// `FiggyError::DeviceLost` itself.
///
/// The tickets taken before the loss race the device's poll thread: a
/// readback that completed before `destroy` returns its result (the device
/// was not lost before or during that pick), and one still pending reports
/// `DeviceLost` once the loss reaches the renderer (`try_resolve` may say
/// `Pending` until then). No other error is allowed. The calls made after
/// the loss report it deterministically.
#[test]
fn a_pick_on_a_lost_device_reports_device_lost() {
    let mut r = renderer_on(harness::fresh_gpu());
    let id = registered(&mut r);
    let pending = r.pick_chart(id, centre_request()).unwrap();
    let pending_resolve = r.pick_chart(id, centre_request()).unwrap();
    let pending_data = r.pick_chart_data(id, centre_request()).unwrap();
    r.device().destroy();
    let _ = r.device().poll(wgpu::PollType::Poll);

    let lost = |error: &GpuPickError| matches!(error, GpuPickError::DeviceLost { .. });
    // Done before the loss, or lost: never another error.
    let done_or_lost = |result: &Result<(), GpuPickError>| {
        result.as_ref().is_ok() || result.as_ref().is_err_and(lost)
    };
    // The loss can reach the renderer after `destroy` returns (Metal
    // reports it asynchronously), so `try_resolve` may still say
    // `Pending`; that ticket's final answer must then be done or lost.
    let tried = match pending.try_resolve() {
        Ok(renderer::TryResolve::Ready(_)) => Ok(()),
        Ok(renderer::TryResolve::Pending(ticket)) => ticket.resolve_blocking().map(|_| ()),
        Err(error) => Err(error),
    };
    assert!(done_or_lost(&tried), "try_resolve: {tried:?}");
    let resolved = pending_resolve.resolve_blocking().map(|_| ());
    assert!(done_or_lost(&resolved), "resolve_blocking: {resolved:?}");
    let picked = r.pick_chart(id, centre_request());
    assert!(
        picked.as_ref().is_err_and(lost),
        "pick_chart: {:?}",
        picked.err()
    );
    let data = r.pick_chart_data(id, centre_request());
    assert!(
        matches!(data, Err(FiggyError::DeviceLost { .. })),
        "pick_chart_data: {:?}",
        data.err()
    );
    let data_resolved = pending_data.resolve_blocking().map(|_| ());
    assert!(
        done_or_lost(&data_resolved),
        "data pick resolve_blocking: {data_resolved:?}"
    );
    // A renderer call reports the same loss, and a pick error converted
    // into a `FiggyError` keeps it typed.
    assert!(matches!(r.wait_idle(), Err(FiggyError::DeviceLost { .. })));
    let error: FiggyError = GpuPickError::DeviceLost {
        reason: "gone".into(),
    }
    .into();
    assert!(matches!(error, FiggyError::DeviceLost { reason } if reason == "gone"));
}

/// The reproduction of #469 item 6, as the egui examples now run it: each
/// frame prepares with a moved area (a drag or a window resize), which
/// retires the previous raster, and the first prepare of the next frame
/// calls `end_gpu_frame` for the frame already submitted. Retired bytes
/// stay within a few frames' rasters, all are released once the GPU is
/// idle, and a column that fits the budget is still taken; before the fix
/// the examples never called it and the retired bytes grew by about
/// 1.5 MB a frame until the budget refused the column.
///
/// Retired bytes are released when the GPU completes the frame that used
/// them, so the loop is paced as a presenting host is: each frame's work
/// completes before the next frame starts (`wait_idle` stands in for the
/// swap chain's present). Unpaced, a CPU adapter such as WARP falls
/// further behind every frame and the level measures its lag, not the
/// release.
#[test]
fn an_egui_style_loop_with_end_gpu_frame_keeps_retired_bytes_bounded() {
    let mut r = renderer_on(harness::gpu());
    harness::upload(&mut r, "ex", vec![1.0, 2.0, 3.0]);
    harness::upload(&mut r, "ey", vec![1.0, 2.0, 3.0]);
    let series_cfg = harness::markers("m", "ex", "ey", harness::RED);
    let style = r.create_style_for_series(&series_cfg).unwrap();
    let mut chart = Chart::new(bare(400, 300));
    let mut view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let series = [Series {
        config: &series_cfg,
        style: &style,
    }];
    let live = r.gpu_memory_usage().total_bytes();
    assert_eq!(r.set_memory_budget(Some(live + 32 * 1024 * 1024)), None);
    // The peak, and the most one frame added.
    let (mut peak, mut frame_bytes, mut last) = (0, 0, 0);
    for frame in 0..200u32 {
        // The first prepare of a frame ends the previous, submitted one.
        r.end_gpu_frame();
        chart.set_area(0, 0, 400 + (frame % 7) * 10, 300);
        r.prepare(&mut [ChartDrawItem {
            view: &mut view,
            chart: &chart,
            series: &series,
        }])
        .unwrap_or_else(|e| panic!("frame {frame}: {e}"));
        // The frame is presented before the next one starts.
        r.wait_idle().unwrap();
        r.service_gpu_completions().unwrap();
        let retired = r.gpu_memory_usage().retired_bytes();
        frame_bytes = frame_bytes.max(retired.saturating_sub(last));
        last = retired;
        peak = peak.max(retired);
    }
    r.end_gpu_frame();
    r.wait_idle().unwrap();
    r.service_gpu_completions().unwrap();
    // Without end_gpu_frame every frame's raster stays retired: about
    // 200 frames' bytes (over 300 MB on lavapipe).
    assert!(
        peak <= 3 * frame_bytes,
        "retired bytes reached {peak} with end_gpu_frame once a frame \
         ({frame_bytes} at most per frame)"
    );
    assert_eq!(r.gpu_memory_usage().retired_bytes(), 0);
    let big: Vec<f64> = (0..1_000_000).map(f64::from).collect();
    r.add_column("big", &big)
        .expect("a column that fits the budget is taken");
}
