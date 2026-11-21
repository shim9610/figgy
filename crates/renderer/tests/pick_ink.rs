//! The hit rule of a registered chart's pick (#381, `Renderer::pick_chart`):
//! the cursor hits when the drawn data layer has ink within
//! `max_distance_px` of it, read from the panel's ink mask, and the hit is
//! then the nearest datum. `resolve_full` answers every series' nearest
//! datum whether or not the cursor hits. At 1x and 2x, for a 1 px line, a
//! styled panel, an overlay layer, a mask that follows a data edit, and the
//! mask's lifetime (none before the first on-screen prepare, none after its
//! chart is removed or the unused memory is released).
//!
//! Grid, legend, title and axis lines are off, so the only ink is the data.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::{BLUE, RED, markers, upload};
use test_gpu::shared_device;

use renderer::config::{AxisOptions, DrawStyle};
use renderer::data_config::{DataLineStyleConfig, LineInterpolation, XySeries};
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::{
    Chart, ChartId, Config, DataRenderType, GpuPickRequest, LayerRef, OverlayLayer, PickResult,
    RegisteredChartDrawItem, Renderer, RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.enable_gpu_picking().unwrap();
    r
}

/// A bare chart: x over 0..10, y over 0..100, nothing drawn but data.
fn bare() -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.chart_title.visible = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.line_visible = false;
    }
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 100.0);
    chart.config().clone()
}

/// A 1 px line through `x` and `y`.
fn thin_line(id: &str, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: None,
            line: Some(DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: BLUE,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: LineInterpolation::Linear,
            }),
            errors: None,
            stem: None,
        }),
    }
}

/// Register `config` with `series`, shown at `scale` (letterboxed onto a
/// surface `scale` times its document), and return its id and the config
/// it is drawn with.
fn register(r: &mut Renderer, config: &Config, series: &[SeriesConfig], scale: f32) -> ChartId {
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    if scale != 1.0 {
        let document = r.chart_config(id).unwrap().document_area();
        r.set_chart_display(
            id,
            Some((
                ((document.x + document.width) as f32 * scale) as u32,
                ((document.y + document.height) as f32 * scale) as u32,
            )),
        )
        .unwrap();
    }
    id
}

/// One on-screen prepare of chart `id`, as a host's frame runs it.
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

/// Where `(x, y)` of the axes `axes` gives lands on chart `id` as drawn.
fn placed(
    r: &Renderer,
    id: ChartId,
    axes: impl Fn(&Config) -> (AxisOptions, AxisOptions),
    x: f64,
    y: f64,
) -> (f32, f32) {
    let (config, _, _) = r.chart_display_config(id).unwrap();
    let area = config.data_area().unwrap().0;
    let (ax, ay, aw, ah) = (
        area.x as f32,
        area.y as f32,
        area.width as f32,
        area.height as f32,
    );
    let (x_axis, y_axis) = axes(&config);
    let fx = renderer::layout::axis_fraction(x, &x_axis, aw);
    let fy = renderer::layout::axis_fraction(y, &y_axis, ah);
    (ax + aw * fx, ay + ah * (1.0 - fy))
}

fn base_axes(c: &Config) -> (AxisOptions, AxisOptions) {
    (c.bottom_x.clone(), c.left_y.clone())
}

fn pick(r: &mut Renderer, id: ChartId, at: (f32, f32), reach: f32) -> PickResult {
    let (_, panel, scale) = r.chart_display_config(id).unwrap();
    let document = r.chart_config(id).unwrap().document_area();
    let display = Rect {
        x: panel.x,
        y: panel.y,
        width: ((document.x + document.width) as f32 * scale) as u32,
        height: ((document.y + document.height) as f32 * scale) as u32,
    };
    let full = r
        .pick_chart(
            id,
            GpuPickRequest {
                canvas_position_px: [at.0, at.1],
                display_panel_px: display,
                display_scale: scale,
                max_distance_px: reach,
            },
        )
        .unwrap()
        .resolve_full_blocking()
        .unwrap();
    // `resolve` is the hit of the same pick.
    let hit = r
        .pick_chart(
            id,
            GpuPickRequest {
                canvas_position_px: [at.0, at.1],
                display_panel_px: display,
                display_scale: scale,
                max_distance_px: reach,
            },
        )
        .unwrap()
        .resolve_blocking()
        .unwrap();
    assert_eq!(hit, full.hit, "resolve and resolve_full agree on the hit");
    full
}

fn hit_series(result: &PickResult) -> Option<(&str, usize)> {
    result
        .hit
        .as_ref()
        .map(|hit| (hit.series_id.as_str(), hit.point_index))
}

/// A 1 px horizontal line is hit from a few pixels away when the reach
/// covers the gap, and not when it does not; every series' nearest datum
/// is answered either way.
#[test]
fn a_thin_line_hits_within_the_reach() {
    for scale in [1.0f32, 2.0] {
        let mut r = renderer();
        upload(&mut r, "lx", vec![0.0, 10.0]);
        upload(&mut r, "ly", vec![50.0, 50.0]);
        let id = register(&mut r, &bare(), &[thin_line("line", "lx", "ly")], scale);
        draw(&mut r, id);
        let (x, y) = placed(&r, id, base_axes, 2.0, 50.0);
        let gap = 4.0 * scale;
        let near = pick(&mut r, id, (x, y - gap), 6.0 * scale);
        assert_eq!(hit_series(&near), Some(("line", 0)), "at {scale}x");
        let short = pick(&mut r, id, (x, y - gap), 1.0);
        assert_eq!(short.hit, None, "out of reach at {scale}x");
        for result in [&near, &short] {
            assert_eq!(result.nearest.len(), 1, "at {scale}x");
            assert_eq!(result.nearest[0].series_id, "line");
            assert_eq!(result.nearest[0].layer, LayerRef::Base);
        }
        // On the line itself, a reach of 0 hits.
        assert_eq!(
            hit_series(&pick(&mut r, id, (x, y), 0.0)),
            Some(("line", 0)),
            "on the line at {scale}x"
        );
    }
}

/// Before the chart's first on-screen prepare there is no ink to read: the
/// pick hits nothing and still answers each series' nearest datum. The
/// mask goes with its chart and with a release of unused memory, and the
/// next on-screen prepare draws it again.
#[test]
fn the_mask_lives_from_the_first_prepare_to_its_chart_or_a_release() {
    let mut r = renderer();
    upload(&mut r, "bx", vec![2.0, 8.0]);
    upload(&mut r, "by", vec![20.0, 80.0]);
    let series = [markers("dots", "bx", "by", BLUE)];
    let id = register(&mut r, &bare(), &series, 1.0);
    let at = placed(&r, id, base_axes, 2.0, 20.0);
    let before = pick(&mut r, id, at, 2.0);
    assert_eq!(before.hit, None, "no ink before the first prepare");
    assert_eq!(before.nearest.len(), 1);
    assert_eq!(before.nearest[0].point_index, 0);

    let empty = r.gpu_memory_usage().live_bytes();
    draw(&mut r, id);
    assert_eq!(hit_series(&pick(&mut r, id, at, 2.0)), Some(("dots", 0)));
    // The mask is one byte per panel pixel (`R8Unorm`).
    let (_, panel, _) = r.chart_display_config(id).unwrap();
    let mask = u64::from(panel.width) * u64::from(panel.height);
    let drawn = r.gpu_memory_usage().live_bytes();
    assert!(
        drawn >= empty + mask,
        "the mask is charged: {empty} -> {drawn}, mask {mask}"
    );

    r.release_unused_pool_memory_blocking(0).unwrap();
    assert_eq!(pick(&mut r, id, at, 2.0).hit, None, "released");
    draw(&mut r, id);
    assert_eq!(hit_series(&pick(&mut r, id, at, 2.0)), Some(("dots", 0)));

    // A new chart in the removed one's place has no ink until it is drawn.
    r.remove_chart(id).unwrap();
    let id = register(&mut r, &bare(), &series, 1.0);
    assert_eq!(
        pick(&mut r, id, at, 2.0).hit,
        None,
        "removed with its chart"
    );
}

/// The mask follows an edit of the data on the next on-screen prepare: the
/// ink moves with the point, and the old place is empty.
#[test]
fn the_mask_follows_a_data_edit() {
    let mut r = renderer();
    upload(&mut r, "bx", vec![2.0]);
    upload(&mut r, "by", vec![20.0]);
    let id = register(&mut r, &bare(), &[markers("dot", "bx", "by", BLUE)], 1.0);
    draw(&mut r, id);
    let old = placed(&r, id, base_axes, 2.0, 20.0);
    let new = placed(&r, id, base_axes, 7.0, 70.0);
    assert!(pick(&mut r, id, old, 1.0).hit.is_some());
    r.upsert_column("bx", &harness::col(vec![7.0])).unwrap();
    r.upsert_column("by", &harness::col(vec![70.0])).unwrap();
    draw(&mut r, id);
    assert_eq!(
        pick(&mut r, id, old, 1.0).hit,
        None,
        "the old place is empty"
    );
    assert_eq!(hit_series(&pick(&mut r, id, new, 1.0)), Some(("dot", 0)));
}

/// Every draw style keeps a mask: a styled panel hits where its ink is.
#[test]
fn a_styled_panel_hits_where_it_draws() {
    for style in [
        DrawStyle::Sketch(Default::default()),
        DrawStyle::Milkyway(Default::default()),
        DrawStyle::Constellation(Default::default()),
    ] {
        for scale in [1.0f32, 2.0] {
            let mut r = renderer();
            upload(&mut r, "bx", vec![2.0, 8.0]);
            upload(&mut r, "by", vec![20.0, 80.0]);
            let mut config = bare();
            config.draw_style = style;
            // Constellation draws a chain: markers joined by a line.
            let mut dots = markers("dots", "bx", "by", BLUE);
            if let (DataRenderType::Xy(xy), DataRenderType::Xy(line)) = (
                &mut dots.render_type,
                thin_line("dots", "bx", "by").render_type,
            ) {
                xy.line = line.line;
            }
            let id = register(&mut r, &config, &[dots], scale);
            draw(&mut r, id);
            let at = placed(&r, id, base_axes, 8.0, 80.0);
            assert_eq!(
                hit_series(&pick(&mut r, id, at, 2.0 * scale)),
                Some(("dots", 1)),
                "{style:?} at {scale}x"
            );
            let empty = placed(&r, id, base_axes, 8.0, 20.0);
            assert_eq!(
                pick(&mut r, id, empty, 2.0 * scale).hit,
                None,
                "{style:?} at {scale}x, away from the data"
            );
        }
    }
}

/// An overlay layer's ink hits, and the hit is its datum, placed by its
/// own axes, whichever layer is active.
#[test]
fn an_overlay_layer_hits_with_its_own_datum() {
    for active in [LayerRef::Base, LayerRef::Overlay(0)] {
        for scale in [1.0f32, 2.0] {
            let mut r = renderer();
            upload(&mut r, "bx", vec![2.0]);
            upload(&mut r, "by", vec![20.0]);
            upload(&mut r, "ox", vec![150.0]);
            upload(&mut r, "oy", vec![25.0]);
            let mut x_axis = AxisOptions::overlay_x("own x", RED);
            x_axis.out_margin = 50.0;
            let mut chart = Chart::new(bare());
            chart.config_mut().overlays.push(OverlayLayer {
                y_axis: AxisOptions::overlay_y("own y", RED),
                link_x: false,
                x_axis,
                series: vec!["over".into()],
                ..OverlayLayer::default()
            });
            chart.set_overlay_x_range(0, 100.0, 200.0);
            chart.set_overlay_y_range(0, 0.0, 50.0);
            let mut config = chart.config().clone();
            config.set_active_layer(active);
            let series = [
                markers("base", "bx", "by", BLUE),
                markers("over", "ox", "oy", RED),
            ];
            let id = register(&mut r, &config, &series, scale);
            draw(&mut r, id);
            let overlay_axes = |c: &Config| {
                let (x, y) = c.scaling_axes(Some(0));
                (x.clone(), y.clone())
            };
            let at = placed(&r, id, overlay_axes, 150.0, 25.0);
            let result = pick(&mut r, id, at, 2.0 * scale);
            assert_eq!(
                hit_series(&result),
                Some(("over", 0)),
                "{active:?} {scale}x"
            );
            assert_eq!(result.nearest.len(), 2, "both layers answer");
            let at = placed(&r, id, base_axes, 2.0, 20.0);
            assert_eq!(
                hit_series(&pick(&mut r, id, at, 2.0 * scale)),
                Some(("base", 0)),
                "{active:?} {scale}x"
            );
        }
    }
}
