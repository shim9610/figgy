//! Tick-guard substitutions reach the host on the `ChartView` its dirty-driven
//! raster refresh rebuilt.

#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::Arc;

use renderer::default::default_config;
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, ChartView, RenderWarning, Renderer, RendererDevice, SpacingRejection, WarningAxis,
};

const PANEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 320,
    height: 240,
};

/// One host frame: refresh the raster only when the dirty flag says it
/// changed. Returns whether it refreshed.
fn host_frame(renderer: &mut Renderer, view: &mut ChartView, chart: &mut Chart) -> bool {
    let raster_dirty = chart.consume_raster_dirty();
    if raster_dirty {
        renderer.refresh_axis(view, chart, PANEL).unwrap();
    }
    raster_dirty
}

#[test]
fn a_refreshed_view_reports_the_substituted_spacing_until_it_is_fixed() {
    let (device, queue) = test_gpu::device();
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        4096,
    )
    .expect("renderer creation");

    let mut config = default_config();
    config.chart_area = ChartArea(PANEL);
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 10.0;
    config.bottom_x.major_spacing = 2.0;
    let mut chart = Chart::new(config);
    let mut view = renderer.create_chart_view(&chart, PANEL).unwrap();
    assert!(view.render_warnings().is_empty());
    assert!(host_frame(&mut renderer, &mut view, &mut chart));

    // A spacing that would draw ten billion majors: the raster walks the
    // automatic spacing for 0..10 instead, and the refreshed view says so.
    chart.config_mut().bottom_x.major_spacing = 1e-9;
    assert!(host_frame(&mut renderer, &mut view, &mut chart));
    let replaced = [RenderWarning::MajorSpacingReplaced {
        axis: WarningAxis::BottomX,
        requested: 1e-9,
        used: 2.0,
        reason: SpacingRejection::TooManyTicks { limit: 1000 },
    }];
    assert_eq!(view.render_warnings(), replaced);
    assert!(
        view.render_warnings()[0]
            .to_string()
            .contains("bottom_x.major_spacing")
    );

    // Clean frames do not refresh; the view keeps describing its raster.
    assert!(!host_frame(&mut renderer, &mut view, &mut chart));
    assert_eq!(view.render_warnings(), replaced);

    // A view created from the same chart reports the same.
    let created = renderer.create_chart_view(&chart, PANEL).unwrap();
    assert_eq!(created.render_warnings(), replaced);

    // Fixing the config and refreshing clears the warning.
    chart.config_mut().bottom_x.major_spacing = 2.5;
    assert!(host_frame(&mut renderer, &mut view, &mut chart));
    assert!(
        view.render_warnings().is_empty(),
        "{:?}",
        view.render_warnings()
    );
}
