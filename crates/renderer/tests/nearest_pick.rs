//! The nearest-datum pick (#381, `Renderer::pick_chart_nearest`) through
//! the public renderer API, at 1x and 2x: every pickable layer at the
//! cursor in one pick (an overlay with axes of its own included), the tie
//! rule between layers, no pick reach, a subplot under the cursor, a 3D
//! chart through its camera, a monotonic chart through its handle, and a
//! streamed source refused.
//!
//! Grid, legend, title and axis lines are off, and every mark has its own
//! pure colour, so a colour predicate attributes every pixel to one mark.

use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use harness::{BLUE, RED, markers, upload};
use test_gpu::shared_device;

use renderer::config::AxisOptions;
use renderer::layout::{ChartArea, Rect};
use renderer::monotonic::MonotonicSlices;
use renderer::renderer::MonotonicRange;
use renderer::view3d::View3d;
use renderer::{
    Chart, Config, GpuPickError, GpuPickRequest, LayerRef, NearestPick, OverlayLayer, RasterImage,
    Renderer, RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;

/// The overlay's points, in its own X units (100..200) and Y units (0..50).
const OVERLAY_POINTS: [(f64, f64); 3] = [(130.0, 10.0), (150.0, 25.0), (175.0, 35.0)];
/// The base points, in the base axes (0..10, 0..100).
const BASE_POINTS: [(f64, f64); 2] = [(2.0, 20.0), (8.0, 80.0)];

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

/// A bare chart at `(x, 0)`: x over 0..10, y over 0..100, axis lines off.
fn bare_at(x: u32) -> Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x,
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

/// [`bare_at`] 0 with one overlay scaling series `over`: its own X axis
/// over 100..200 (`link_x` off) and its own Y axis over 0..50.
fn with_overlay() -> Config {
    let mut x_axis = AxisOptions::overlay_x("own x", RED);
    x_axis.out_margin = 50.0;
    let mut chart = Chart::new(bare_at(0));
    chart.config_mut().overlays.push(OverlayLayer {
        y_axis: AxisOptions::overlay_y("own y", RED),
        link_x: false,
        x_axis,
        series: vec!["over".into()],
        ..OverlayLayer::default()
    });
    chart.set_overlay_x_range(0, 100.0, 200.0);
    chart.set_overlay_y_range(0, 0.0, 50.0);
    chart.config().clone()
}

fn fixture(r: &mut Renderer) -> [SeriesConfig; 2] {
    upload(r, "bx", BASE_POINTS.iter().map(|p| p.0).collect());
    upload(r, "by", BASE_POINTS.iter().map(|p| p.1).collect());
    upload(r, "ox", OVERLAY_POINTS.iter().map(|p| p.0).collect());
    upload(r, "oy", OVERLAY_POINTS.iter().map(|p| p.1).collect());
    [
        markers("base", "bx", "by", BLUE),
        markers("over", "ox", "oy", RED),
    ]
}

/// Where `(x, y)` lands at `scale` on the axes `axes` gives for `config`.
fn placed(
    config: &Config,
    axes: impl Fn(&Config) -> (AxisOptions, AxisOptions),
    x: f64,
    y: f64,
    scale: f32,
) -> (f32, f32) {
    let scaled = config.scaled(scale);
    let area = scaled.data_area().unwrap().0;
    let (ax, ay, aw, ah) = (
        area.x as f32,
        area.y as f32,
        area.width as f32,
        area.height as f32,
    );
    let (x_axis, y_axis) = axes(&scaled);
    let fx = renderer::layout::axis_fraction(x, &x_axis, aw);
    let fy = renderer::layout::axis_fraction(y, &y_axis, ah);
    (ax + aw * fx, ay + ah * (1.0 - fy))
}

fn base_axes(c: &Config) -> (AxisOptions, AxisOptions) {
    (c.bottom_x.clone(), c.left_y.clone())
}

fn overlay_axes(c: &Config) -> (AxisOptions, AxisOptions) {
    let (x, y) = c.scaling_axes(Some(0));
    (x.clone(), y.clone())
}

fn request(config: &Config, at: (f32, f32), scale: f32) -> GpuPickRequest {
    let document = config.document_area();
    GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: ((document.x + document.width) as f32 * scale) as u32,
            height: ((document.y + document.height) as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 0.0,
    }
}

fn pick(
    r: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    at: (f32, f32),
    scale: f32,
) -> NearestPick {
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    let picked = r
        .pick_chart_nearest(id, request(config, at, scale))
        .unwrap()
        .resolve_blocking()
        .unwrap();
    r.remove_chart(id).unwrap();
    picked
}

fn answer(picked: &NearestPick) -> Option<(String, LayerRef, usize)> {
    picked
        .nearest
        .as_ref()
        .map(|d| (d.series_id.clone(), d.layer, d.point_index))
}

/// One pick searches the base layer and its overlay, each series placed by
/// its own layer's axes, whichever layer is active: at every point of
/// either layer the nearest datum is that point, at less than a pixel, and
/// both series answer with their layer.
#[test]
fn one_pick_covers_every_layer_with_its_own_axes() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let config = with_overlay();
    for active in [LayerRef::Base, LayerRef::Overlay(0)] {
        let mut config = config.clone();
        config.set_active_layer(active);
        for scale in [1.0f32, 2.0] {
            let points = BASE_POINTS
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| {
                    (
                        "base",
                        LayerRef::Base,
                        i,
                        placed(&config, base_axes, x, y, scale),
                    )
                })
                .chain(OVERLAY_POINTS.iter().enumerate().map(|(i, &(x, y))| {
                    (
                        "over",
                        LayerRef::Overlay(0),
                        i,
                        placed(&config, overlay_axes, x, y, scale),
                    )
                }));
            for (id, layer, index, at) in points {
                let picked = pick(&mut r, &config, &series, at, scale);
                assert_eq!(
                    answer(&picked),
                    Some((id.to_string(), layer, index)),
                    "{scale}x, active {active:?}: point {id}#{index} at {at:?}"
                );
                let nearest = picked.nearest.as_ref().unwrap();
                assert!(nearest.distance_px < 0.75, "{scale}x: {nearest:?}");
                let layers: Vec<_> = picked
                    .per_series
                    .iter()
                    .map(|d| (d.series_id.as_str(), d.layer))
                    .collect();
                assert_eq!(
                    layers,
                    [("base", LayerRef::Base), ("over", LayerRef::Overlay(0))],
                    "{scale}x: every series answers with its layer"
                );
            }
        }
    }
}

/// An exact distance tie between layers goes to the active layer: an
/// overlay drawing the same point at the same place as the base answers
/// for the base while the base is active, and for itself while it is.
#[test]
fn an_exact_tie_between_layers_goes_to_the_active_layer() {
    let mut r = renderer();
    upload(&mut r, "tx", vec![5.0]);
    upload(&mut r, "ty", vec![50.0]);
    let series = [
        markers("under", "tx", "ty", BLUE),
        markers("twin", "tx", "ty", RED),
    ];
    let mut chart = Chart::new(bare_at(0));
    chart.config_mut().overlays.push(OverlayLayer {
        y_axis: AxisOptions::overlay_y("twin y", RED),
        series: vec!["twin".into()],
        ..OverlayLayer::default()
    });
    chart.set_overlay_y_range(0, 0.0, 100.0);
    let config = chart.config().clone();
    for scale in [1.0f32, 2.0] {
        // Off the point, so the tie is at a distance, not at zero.
        let (x, y) = placed(&config, base_axes, 5.0, 50.0, scale);
        let at = (x + 7.0 * scale, y - 3.0 * scale);
        for (active, winner) in [
            (LayerRef::Base, ("under", LayerRef::Base)),
            (LayerRef::Overlay(0), ("twin", LayerRef::Overlay(0))),
        ] {
            let mut config = config.clone();
            config.set_active_layer(active);
            let picked = pick(&mut r, &config, &series, at, scale);
            let distances: Vec<_> = picked.per_series.iter().map(|d| d.distance_px).collect();
            assert_eq!(distances[0], distances[1], "{scale}x: the two places tie");
            assert_eq!(
                answer(&picked),
                Some((winner.0.to_string(), winner.1, 0)),
                "{scale}x, active {active:?}"
            );
        }
    }
}

/// There is no pick reach: a cursor far from every point still gets each
/// series' nearest datum, at its true distance.
#[test]
fn there_is_no_pick_reach() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let config = bare_at(0);
    let base_only = &series[..1];
    for scale in [1.0f32, 2.0] {
        // The data area's far corner from (8, 80), nearer to (2, 20).
        let corner = placed(&config, base_axes, 0.0, 0.0, scale);
        let picked = pick(&mut r, &config, base_only, corner, scale);
        let nearest = picked.nearest.expect("an answer with no reach");
        assert_eq!(
            (nearest.series_id.as_str(), nearest.point_index),
            ("base", 0)
        );
        let point = placed(&config, base_axes, 2.0, 20.0, scale);
        let expected = (point.0 - corner.0).hypot(point.1 - corner.1);
        assert!(
            (nearest.distance_px - expected).abs() < 0.5,
            "{scale}x: {} against {expected}",
            nearest.distance_px
        );
    }
}

/// Over a subplot, the pick searches that subplot (and only it): its own
/// area and axes place its series.
#[test]
fn a_subplot_under_the_cursor_is_picked_on_its_own_axes() {
    let mut r = renderer();
    upload(&mut r, "sx", vec![3.0, 7.0]);
    upload(&mut r, "sy", vec![30.0, 70.0]);
    upload(&mut r, "px", vec![5.0]);
    upload(&mut r, "py", vec![50.0]);
    let series = [
        markers("parent", "px", "py", BLUE),
        markers("sub", "sx", "sy", RED),
    ];
    let mut config = bare_at(0);
    config.subplots.push(renderer::Subplot {
        config: bare_at(WIDTH),
        series: vec!["sub".into()],
        ..Default::default()
    });
    for scale in [1.0f32, 2.0] {
        for (index, (x, y)) in [(3.0, 30.0), (7.0, 70.0)].into_iter().enumerate() {
            let sub = config.subplots[0].config.clone();
            let at = placed(&sub, base_axes, x, y, scale);
            let picked = pick(&mut r, &config, &series, at, scale);
            assert_eq!(
                answer(&picked),
                Some(("sub".to_string(), LayerRef::Subplot(0), index)),
                "{scale}x subplot point {index} at {at:?}"
            );
            assert!(
                picked.per_series.iter().all(|d| d.series_id == "sub"),
                "{scale}x: only the subplot's series"
            );
        }
        // Over the parent, the parent's series.
        let at = placed(&config, base_axes, 5.0, 50.0, scale);
        let picked = pick(&mut r, &config, &series, at, scale);
        assert_eq!(
            answer(&picked),
            Some(("parent".to_string(), LayerRef::Base, 0))
        );
    }
}

/// A 3D chart places each datum through its camera: the pick at the
/// centre of a drawn marker answers that datum at about zero distance, at
/// 1x and 2x.
#[test]
fn a_3d_chart_is_picked_through_its_camera() {
    let mut r = renderer();
    upload(&mut r, "x3", vec![0.25, 0.7]);
    upload(&mut r, "y3", vec![0.3, 0.6]);
    upload(&mut r, "z3", vec![0.8, 0.2]);
    let mut config = bare_at(0);
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.min = 0.0;
        axis.max = 1.0;
    }
    config.set_view_3d(Some(View3d::default()));
    let mut series = markers("cloud", "x3", "y3", RED);
    series.z_column = Some("z3".into());
    let series = [series];
    for scale in [1.0f32, 2.0] {
        let with = export(&mut r, &config, &series, scale);
        let without = export(&mut r, &config, &[], scale);
        // Each marker's ink: the pixels the series changes, grouped by
        // proximity, each group's mean its centre (a 3D marker is shaded,
        // so its colour varies across it).
        let mut groups: Vec<(f32, f32, f32)> = Vec::new();
        for y in 0..with.height {
            for x in 0..with.width {
                let i = ((y * with.width + x) * 4) as usize;
                if with.rgba[i..i + 4] == without.rgba[i..i + 4] {
                    continue;
                }
                let at = (x as f32 + 0.5, y as f32 + 0.5);
                match groups
                    .iter_mut()
                    .find(|g| (g.0 / g.2 - at.0).hypot(g.1 / g.2 - at.1) < 16.0 * scale)
                {
                    Some(group) => {
                        group.0 += at.0;
                        group.1 += at.1;
                        group.2 += 1.0;
                    }
                    None => groups.push((at.0, at.1, 1.0)),
                }
            }
        }
        let marks: Vec<(f32, f32)> = groups.iter().map(|g| (g.0 / g.2, g.1 / g.2)).collect();
        assert_eq!(marks.len(), 2, "{scale}x: two markers drawn");
        for mark in marks {
            let picked = pick(&mut r, &config, &series, mark, scale);
            let nearest = picked.nearest.expect("a 3D answer");
            assert!(
                nearest.distance_px < 1.0,
                "{scale}x: marker at {mark:?} picked {nearest:?}"
            );
        }
    }
}

fn export(r: &mut Renderer, config: &Config, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(&Chart::new(config.clone()), series),
        renderer::ExportOptions::document(scale),
    )
    .block_on()
    .unwrap()
}

/// A monotonic chart is picked through its handle: the nearest kept point
/// answers its logical sample index, without the source (#271, M6).
#[test]
fn a_monotonic_chart_answers_the_logical_sample_index() {
    let mut r = renderer();
    let config = bare_at(0);
    let id = r
        .register_monotonic_chart(
            config.clone(),
            vec![SeriesConfig::monotonic_line("m", "g", 0).color(BLUE)],
        )
        .unwrap();
    // Sparse: every sample is kept.
    let values: Vec<f64> = (0..40)
        .map(|i| 50.0 + 30.0 * (i as f64 * 0.4).sin())
        .collect();
    let channels = [values.as_slice()];
    r.monotonic_chart(id)
        .unwrap()
        .set_monotonic(
            "g",
            &MonotonicSlices::new(0.0, 0.25, &channels),
            MonotonicRange::X {
                min: 0.0,
                max: 10.0,
            },
        )
        .unwrap();
    for index in [3usize, 17, 31] {
        let at = placed(&config, base_axes, index as f64 * 0.25, values[index], 1.0);
        let picked = r
            .monotonic_chart(id)
            .unwrap()
            .pick_nearest(request(&config, at, 1.0))
            .unwrap()
            .resolve_blocking()
            .unwrap();
        let nearest = picked.nearest.expect("a monotonic answer");
        assert_eq!(
            (nearest.series_id.as_str(), nearest.point_index),
            ("m", index)
        );
        assert!(nearest.distance_px < 0.75, "{nearest:?}");
    }
}

/// A streamed source is refused, as every registered-chart pick refuses
/// one (rule 8).
#[test]
fn a_streamed_source_is_refused() {
    let mut r = renderer();
    r.register_streamed_columns(vec![
        stream_column("stream-x", 4),
        stream_column("stream-y", 4),
    ])
    .unwrap();
    let config = bare_at(0);
    let id = r
        .register_chart(
            config.clone(),
            vec![markers("streamed", "stream-x", "stream-y", BLUE)],
        )
        .unwrap();
    let at = placed(&config, base_axes, 5.0, 50.0, 1.0);
    assert!(matches!(
        r.pick_chart_nearest(id, request(&config, at, 1.0)),
        Err(GpuPickError::ColumnNotResident(_))
    ));
}

fn stream_column(id: &str, len: u64) -> renderer::StreamColumn {
    renderer::StreamColumn {
        id: id.into(),
        len,
        encoding: renderer::StreamEncoding::ScalarF32,
        replay: renderer::StreamReplay::RandomAccess,
        revision: 1,
        statistics: renderer::StreamStatistics::Unknown,
    }
}
