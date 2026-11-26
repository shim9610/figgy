//! The window wrapper's picks and its surface-acquire errors (#469).
//!
//! No adapter in CI presents a surface, so a `WindowedRenderer` cannot be
//! built here. The picks are tested through the placement they all build
//! their request from (`Renderer::window_pick_placement`), and the acquire
//! errors through the classification `draw_prepared` returns them by.
use super::super::*;
use super::surface_status;

/// A 400x300 chart over x 0..10, y 0..100, with markers at x 2 and 8.
fn two_marker_chart(r: &mut Renderer) -> ChartId {
    let x = crate::Column {
        data: vec![2.0f64, 8.0],
    };
    let y = crate::Column {
        data: vec![50.0f64, 50.0],
    };
    r.add_column("x", &x).unwrap();
    r.add_column("y", &y).unwrap();
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 400,
        height: 300,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 100.0);
    let series = SeriesConfig::scatter("m", "x", "y");
    r.register_chart(chart.config().clone(), vec![series])
        .unwrap()
}

/// Draw chart `id` as the window draws it: `prepare_registered` through a
/// view at its display placement.
fn draw(r: &mut Renderer, id: ChartId) {
    let (config, panel, scale) = r.chart_display_config(id).unwrap();
    let chart = Chart::new(config.into_owned());
    let mut view = r.create_chart_view_at(&chart, panel, scale).unwrap();
    r.prepare_registered(&mut [RegisteredChartDrawItem {
        chart_id: id,
        view: &mut view,
    }])
    .unwrap();
}

/// Where the datum `(x, y)` is drawn on the surface, from the placed config.
fn drawn_at(r: &Renderer, id: ChartId, x: f64, y: f64) -> [f32; 2] {
    let (config, _, _) = r.chart_display_config(id).unwrap();
    let area = config.data_area().unwrap().0;
    let (ax, ay, aw, ah) = (
        area.x as f32,
        area.y as f32,
        area.width as f32,
        area.height as f32,
    );
    let fx = crate::layout::axis_fraction(x, &config.bottom_x, aw);
    let fy = crate::layout::axis_fraction(y, &config.left_y, ah);
    [ax + aw * fx, ay + ah * (1.0 - fy)]
}

/// The reproduction of #469 item 5: a window pick on the ink drawn under
/// the cursor answers that datum, whatever the chart's display (drawn as
/// stored, fitted to a larger or a smaller surface, or held), because the
/// request goes through the placement the draw used. Before the fix the
/// request letterboxed the document to the window's surface, so on an
/// 800x600 window a chart drawn as stored picked point 0 at 131 px.
#[test]
fn a_window_pick_uses_the_placement_the_draw_used() {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        1 << 20,
    )
    .unwrap();
    r.enable_gpu_picking().unwrap();
    let id = two_marker_chart(&mut r);
    for (display, hold) in [
        (None, false),
        (Some((800, 600)), false),
        (Some((200, 150)), false),
        (Some((800, 600)), true),
    ] {
        r.set_chart_display(id, display).unwrap();
        r.hold_chart_display(id, hold).unwrap();
        draw(&mut r, id);
        let (display_scale, display_panel_px) = r.window_pick_placement(id).unwrap();
        let (placed, _, scale) = r.chart_display_config(id).unwrap();
        assert_eq!(
            (display_scale, display_panel_px),
            (scale, placed.document_area()),
            "{display:?} held {hold}"
        );
        for (index, x) in [(0, 2.0), (1, 8.0)] {
            let at = drawn_at(&r, id, x, 50.0);
            let hit = r
                .pick_chart(
                    id,
                    GpuPickRequest {
                        canvas_position_px: at,
                        display_panel_px,
                        display_scale,
                        max_distance_px: 4.0,
                    },
                )
                .unwrap()
                .resolve_blocking()
                .unwrap()
                .unwrap_or_else(|| panic!("{display:?} held {hold}: no hit at {at:?}"));
            assert_eq!(hit.point_index, index, "{display:?} held {hold}");
            assert!(
                hit.distance_px <= 4.0,
                "{display:?} held {hold}: {} px",
                hit.distance_px
            );
        }
        r.hold_chart_display(id, false).unwrap();
    }
}

/// The reproduction of #469 item 7 (traced): every acquire status that
/// carries no texture is a typed kind a host branches on, and the error
/// names it. Outdated is reconfigured once inside `draw_prepared` before it
/// is returned as `Reconfigure`.
#[test]
fn a_surface_acquire_failure_is_classed_by_what_the_host_does() {
    use wgpu::CurrentSurfaceTexture as Status;
    for (status, kind, name) in [
        (Status::Timeout, SurfaceAcquireKind::Skip, "timeout"),
        (Status::Occluded, SurfaceAcquireKind::Skip, "occluded"),
        (Status::Validation, SurfaceAcquireKind::Skip, "validation"),
        (
            Status::Outdated,
            SurfaceAcquireKind::Reconfigure,
            "outdated",
        ),
        (Status::Lost, SurfaceAcquireKind::Lost, "lost"),
    ] {
        assert_eq!(surface_status(&status), (kind, name));
        let error = FiggyError::SurfaceAcquireFailed {
            kind,
            reason: name.into(),
        };
        assert_eq!(
            error.to_string(),
            format!("wgpu surface acquire failed ({kind:?}): {name}")
        );
    }
}
