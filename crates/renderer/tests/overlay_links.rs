//! Overlay layers with linked or unlinked axes (#283, CLAUDE.md rule 6),
//! through the public renderer API and the headless `export_rgba`: an
//! overlay with an X axis of its own draws, picks, highlights and steps by
//! it at 1x and 2x and draws that axis on the top side; an overlay that
//! follows the parent's Y axis draws and picks by it; the view state, the
//! layer fit, vector export, a JSON round trip and an overlay of a subplot
//! follow the links.
//!
//! The model side (expansion, bands, hit map and drag, fit roles,
//! validation, serde) is in the model's unit tests; render warnings and the
//! SSOT matrix are in the renderer's unit tests; monotonic overlays are in
//! `monotonic_render.rs`.
//!
//! Grid, legend, title and axis lines are off, and every mark has its own
//! pure colour, so a colour predicate attributes every pixel to one mark.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::AxisOptions;
use renderer::data::Column;
use renderer::data_config::{DataScatterStyleConfig, ScatterShape, XySeries};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, Color, DataRenderType, LayerRef, OverlayLayer, RasterImage, Renderer, RendererDevice,
    SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const GOLD: Color = Color::from_rgb8(255, 215, 0);

/// The overlay's points, in its own X units (100..200) and Y units (0..50).
const OVERLAY_POINTS: [(f64, f64); 3] = [(130.0, 10.0), (150.0, 25.0), (175.0, 35.0)];

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

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

fn upload(r: &mut Renderer, id: &str, data: Vec<f64>) {
    r.add_column(id, &Column { data }).unwrap();
}

/// A bare chart: x over 0..10, y over 0..100, axis lines off.
fn bare() -> renderer::Config {
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

/// [`bare`] with one overlay scaling series `over`: its own X axis over
/// 100..200 on the top side (`link_x` off) and its own Y axis over 0..50.
fn with_own_x() -> renderer::Config {
    let mut config = bare();
    let mut x_axis = AxisOptions::overlay_x("own x", RED);
    x_axis.out_margin = 50.0;
    let y_axis = AxisOptions::overlay_y("own y", RED);
    let mut chart = Chart::new(config.clone());
    chart.config_mut().overlays.push(OverlayLayer {
        y_axis,
        link_x: false,
        x_axis,
        series: vec!["over".into()],
        ..OverlayLayer::default()
    });
    chart.set_overlay_x_range(0, 100.0, 200.0);
    chart.set_overlay_y_range(0, 0.0, 50.0);
    config = chart.config().clone();
    config
        .validate_chart()
        .expect("an own overlay X axis validates");
    config
}

fn markers(id: &str, x: &str, y: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter: Some(DataScatterStyleConfig {
                point_color: color,
                point_shape: ScatterShape::SquareFilled,
                point_size: 8.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            }),
            line: None,
            errors: None,
            stem: None,
        }),
    }
}

/// The base series `base` at (2, 20) and (8, 80), and the overlay series
/// `over` at [`OVERLAY_POINTS`].
fn fixture(r: &mut Renderer) -> [SeriesConfig; 2] {
    upload(r, "bx", vec![2.0, 8.0]);
    upload(r, "by", vec![20.0, 80.0]);
    upload(r, "ox", OVERLAY_POINTS.iter().map(|p| p.0).collect());
    upload(r, "oy", OVERLAY_POINTS.iter().map(|p| p.1).collect());
    [
        markers("base", "bx", "by", BLUE),
        markers("over", "ox", "oy", RED),
    ]
}

fn export(
    r: &mut Renderer,
    config: &renderer::Config,
    series: &[SeriesConfig],
    scale: f32,
) -> RasterImage {
    let chart = Chart::new(config.clone());
    r.export_rgba(
        renderer::ExportSource::chart(&chart, series),
        renderer::ExportOptions::document(scale),
    )
    .block_on()
    .unwrap()
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

fn is(color: Color) -> impl Fn([u8; 4]) -> bool {
    let [r, g, b] = [color.r, color.g, color.b].map(|c| (c * 255.0).round() as i32);
    move |p| {
        (p[0] as i32 - r).abs() < 40 && (p[1] as i32 - g).abs() < 40 && (p[2] as i32 - b).abs() < 40
    }
}

/// Where `(x, y)` lands at `scale` against `x_axis` and `y_axis` (axes of
/// `config` scaled with it).
fn placed(
    config: &renderer::Config,
    axes: impl Fn(&renderer::Config) -> (AxisOptions, AxisOptions),
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

/// The axes overlay 0 of a config scales its series by.
fn overlay_axes(config: &renderer::Config) -> (AxisOptions, AxisOptions) {
    let (x, y) = config.scaling_axes(Some(0));
    (x.clone(), y.clone())
}

/// The centroid of the pixels matching `pred` within `radius` of `(cx, cy)`.
fn centroid(
    img: &RasterImage,
    (cx, cy): (f32, f32),
    radius: f32,
    pred: impl Fn([u8; 4]) -> bool,
) -> Option<(f32, f32)> {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    let lo = |c: f32| (c - radius).max(0.0) as u32;
    for y in lo(cy)..((cy + radius) as u32).min(img.height) {
        for x in lo(cx)..((cx + radius) as u32).min(img.width) {
            if pred(pixel(img, x, y)) {
                sx += x as f32 + 0.5;
                sy += y as f32 + 0.5;
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| (sx / n, sy / n))
}

/// What a data pick at `at` (image pixels) answers, as `(series_id, point
/// index)`.
fn pick_at(
    r: &mut Renderer,
    config: &renderer::Config,
    series: &[SeriesConfig],
    at: (f32, f32),
    scale: f32,
) -> Option<(String, usize)> {
    let panel = config.chart_area.0;
    let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: ((panel.x + panel.width) as f32 * scale) as u32,
            height: ((panel.y + panel.height) as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 6.0 * scale,
    };
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    match hit {
        Some(renderer::PickedData::Point {
            series_id,
            point_index,
            ..
        }) => Some((series_id, point_index)),
        Some(other) => panic!("unexpected pick {other:?}"),
        None => None,
    }
}

fn picked(mut config: renderer::Config, series_id: &str, index: usize) -> renderer::Config {
    config.picked_data = Some(renderer::DataSelectionsConfig {
        refs: vec![renderer::PickedDataRef::Point {
            source_id: None,
            series_id: series_id.into(),
            point_index: index,
        }],
        ..Default::default()
    });
    config
}

/// The number of pixels above the data area, inside the chart, that match
/// `pred`: the top band.
fn in_top_band(
    img: &RasterImage,
    config: &renderer::Config,
    scale: f32,
    pred: impl Fn([u8; 4]) -> bool,
) -> usize {
    let area = config.scaled(scale).data_area().unwrap().0;
    (0..area.y)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| pred(pixel(img, x, y)))
        .count()
}

/// An overlay with an X axis of its own places its marks by that axis and
/// its own Y axis, and the base marks keep the parent's, at 1x and 2x; a
/// pick on the active overlay answers its point there; the overlay's X axis
/// is drawn on the top side in its colour; and linking X again places the
/// overlay's marks by the parent's X.
#[test]
fn an_overlay_with_its_own_x_draws_and_picks_by_it() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let config = with_own_x();
    let mut active = config.clone();
    active.set_active_layer(LayerRef::Overlay(0));
    let parent_axes = |c: &renderer::Config| (c.bottom_x.clone(), c.left_y.clone());
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        for (index, &(x, y)) in OVERLAY_POINTS.iter().enumerate() {
            let at = placed(&config, overlay_axes, x, y, scale);
            let mark = centroid(&img, at, 8.0 * scale, is(RED))
                .unwrap_or_else(|| panic!("{scale}x: no overlay mark {index} at {at:?}"));
            assert!(
                (mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0,
                "{scale}x overlay point {index}: drawn at {mark:?}, axis {at:?}"
            );
            assert_eq!(
                pick_at(&mut r, &active, &series, at, scale),
                Some(("over".into(), index)),
                "{scale}x overlay point {index}"
            );
        }
        for (x, y) in [(2.0, 20.0), (8.0, 80.0)] {
            let at = placed(&config, parent_axes, x, y, scale);
            let mark = centroid(&img, at, 8.0 * scale, is(BLUE)).expect("base mark");
            assert!((mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0);
        }
        // The overlay's own X axis: its red labels and title above the plot.
        assert!(
            in_top_band(&img, &config, scale, is(RED)) > 20,
            "{scale}x: no own X axis on the top side"
        );
    }
    // Linked again: the parent's X places the overlay's marks (130 is far
    // past the parent's 0..10, so every point leaves the plot), and the top
    // band draws nothing red.
    let mut linked = config.clone();
    linked.overlays[0].link_x = true;
    let img = export(&mut r, &linked, &series, 1.0);
    assert_eq!(in_top_band(&img, &linked, 1.0, is(RED)), 0);
    let own = placed(&config, overlay_axes, 150.0, 25.0, 1.0);
    assert!(centroid(&img, own, 6.0, is(RED)).is_none());
}

/// The picked highlight rings an overlay point where its own X axis puts it,
/// and `step_selection` walks the overlay series' indices with the ring
/// following, at 1x and 2x.
#[test]
fn the_highlight_and_step_follow_an_own_x_axis() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let mut config = picked(with_own_x(), "over", 0);
    assert!(
        config
            .step_selection(&series, 1, 0, |_| Some(OVERLAY_POINTS.len()))
            .expect("a point selection steps")
    );
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        let (x, y) = OVERLAY_POINTS[1];
        let at = placed(&config, overlay_axes, x, y, scale);
        let ring = centroid(&img, at, 14.0 * scale, is(GOLD))
            .unwrap_or_else(|| panic!("{scale}x: no highlight at {at:?}"));
        assert!(
            (ring.0 - at.0).abs() < 1.5 && (ring.1 - at.1).abs() < 1.5,
            "{scale}x: ring at {ring:?}, mark at {at:?}"
        );
    }
}

/// An overlay that follows the parent's Y axis draws and picks its marks
/// by the parent's `left_y`, whatever its own `y_axis` holds, and draws no
/// Y axis of its own.
#[test]
fn an_overlay_that_follows_the_parent_y_draws_and_picks_by_it() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let mut config = bare();
    let mut y_axis = AxisOptions::overlay_y("unused", RED);
    y_axis.min = -1000.0;
    y_axis.max = 1000.0;
    config.overlays.push(OverlayLayer {
        y_axis,
        link_y: true,
        series: vec!["over".into()],
        ..OverlayLayer::default()
    });
    // The overlay's points fall inside the parent's 0..10 x 0..100.
    let points = [(3.0, 30.0), (6.0, 60.0)];
    upload(&mut r, "lx", points.iter().map(|p| p.0).collect());
    upload(&mut r, "ly", points.iter().map(|p| p.1).collect());
    let series = [series[0].clone(), markers("over", "lx", "ly", RED)];
    let mut active = config.clone();
    active.set_active_layer(LayerRef::Overlay(0));
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        for (index, &(x, y)) in points.iter().enumerate() {
            let at = placed(&config, overlay_axes, x, y, scale);
            let at_parent = placed(
                &config,
                |c| (c.bottom_x.clone(), c.left_y.clone()),
                x,
                y,
                scale,
            );
            assert_eq!(at, at_parent);
            let mark = centroid(&img, at, 8.0 * scale, is(RED)).expect("overlay mark");
            assert!((mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0);
            assert_eq!(
                pick_at(&mut r, &active, &series, at, scale),
                Some(("over".into(), index))
            );
        }
        // No red axis title or labels right of the plot.
        let area = config.scaled(scale).data_area().unwrap().0;
        let right = (area.x + area.width + 2..img.width)
            .flat_map(|x| (0..img.height).map(move |y| (x, y)))
            .filter(|&(x, y)| is(RED)(pixel(&img, x, y)))
            .count();
        assert_eq!(right, 0, "{scale}x: a followed Y axis draws nothing");
    }
}

/// The view state carries each overlay's own X range: reading it gives the
/// overlay's range, writing a new one moves only that axis, and a view
/// state with a missing entry is refused.
#[test]
fn the_view_state_moves_an_overlay_x_axis() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let id = r.register_chart(with_own_x(), series.to_vec()).unwrap();
    let mut view = r.chart_view_state(id).unwrap();
    assert_eq!(view.overlay_x.len(), 1);
    assert_eq!(
        (view.overlay_x[0].min, view.overlay_x[0].max),
        (100.0, 200.0)
    );
    let bottom = (view.bottom_x.min, view.bottom_x.max);
    view.overlay_x[0].min = 140.0;
    view.overlay_x[0].max = 160.0;
    r.set_chart_view_state(id, view.clone()).unwrap();
    let config = r.chart_config(id).unwrap();
    assert_eq!(
        (config.overlays[0].x_axis.min, config.overlays[0].x_axis.max),
        (140.0, 160.0)
    );
    assert_eq!((config.bottom_x.min, config.bottom_x.max), bottom);
    view.overlay_x.clear();
    assert!(r.set_chart_view_state(id, view).is_err());
}

/// A layer fit of an overlay with its own X axis fits that axis to its own
/// series, not the parent's; the parent's fit leaves the overlay's series
/// out of its X axis.
#[test]
fn a_layer_fit_follows_the_overlay_links() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let mut chart = Chart::new(with_own_x());
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Overlay(0), &series, 0.0)
        .unwrap();
    let overlay = &chart.config().overlays[0].x_axis;
    assert!(overlay.min <= 130.0 && overlay.max >= 175.0 && overlay.max < 300.0);
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    let bottom = &chart.config().bottom_x;
    assert!(bottom.min <= 2.0 && bottom.max >= 8.0 && bottom.max < 100.0);
    // Linked, the overlay's series join the parent's X fit again.
    let mut linked = with_own_x();
    linked.overlays[0].link_x = true;
    let mut chart = Chart::new(linked);
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    assert!(chart.config().bottom_x.max >= 175.0);
}

/// A vector export draws the overlay's own X axis (its labels' colour is in
/// the SVG, and only while it is unlinked), and a document that went through
/// JSON draws the same image.
#[test]
fn vector_export_and_a_json_round_trip_keep_the_own_x_axis() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let mut config = with_own_x();
    let teal = Color::from_rgb8(0x00, 0x99, 0x88);
    config.overlays[0].x_axis.label_style.color = teal;
    config.overlays[0].x_axis.title_option.text.color = teal;
    let svg = |r: &mut Renderer, config: &renderer::Config| {
        let chart = Chart::new(config.clone());
        r.export_svg_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap()
    };
    assert!(
        svg(&mut r, &config).contains("#009988"),
        "the own X axis is drawn"
    );
    let mut linked = config.clone();
    linked.overlays[0].link_x = true;
    assert!(!svg(&mut r, &linked).contains("#009988"));
    // The PDF draws the same axis: the own X axis adds its content.
    let pdf = |r: &mut Renderer, config: &renderer::Config| {
        let chart = Chart::new(config.clone());
        r.export_pdf_blocking(
            renderer::ExportSource::chart(&chart, &series),
            renderer::ExportOptions::document(1.0),
        )
        .unwrap()
    };
    let (own, without) = (pdf(&mut r, &config), pdf(&mut r, &linked));
    assert!(own.starts_with(b"%PDF") && without.starts_with(b"%PDF"));
    assert!(
        own.len() > without.len(),
        "the own X axis adds content to the PDF"
    );
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"link_x\":false"));
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        let a = export(&mut r, &config, &series, 1.0);
        let b = export(&mut r, &back, &series, 1.0);
        assert_eq!(a.rgba, b.rgba);
    }
}

/// An overlay of a subplot with its own X axis places its marks by that
/// axis inside the subplot.
#[test]
fn a_subplot_overlay_draws_by_its_own_x() {
    let mut r = renderer();
    let series = fixture(&mut r);
    // The subplot is a whole chart of its own below the parent.
    let mut config = bare();
    let mut sub = with_own_x();
    sub.chart_area.0.y = HEIGHT;
    config.subplots = vec![renderer::Subplot {
        config: sub,
        series: vec!["over".into()],
        ..renderer::Subplot::default()
    }];
    let resolved = config.subplot_config(0).unwrap();
    let img = export(&mut r, &config, &series, 1.0);
    for &(x, y) in &OVERLAY_POINTS {
        let at = placed(&resolved, overlay_axes, x, y, 1.0);
        let mark = centroid(&img, at, 8.0, is(RED)).expect("subplot overlay mark");
        assert!((mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0);
    }
}

/// Autoscale with Y linked: the overlay's series join the parent's Y fit,
/// a fit of the overlay layer fits the parent's Y axes it follows, and the
/// overlay's own stored `y_axis` is left as it is.
#[test]
fn autoscale_with_a_linked_y_fits_the_parent_y_axis() {
    let mut r = renderer();
    let series = fixture(&mut r);
    upload(&mut r, "hy", vec![150.0, 300.0, 250.0]);
    let series = [series[0].clone(), markers("over", "ox", "hy", RED)];
    let mut config = bare();
    config.overlays.push(OverlayLayer {
        link_y: true,
        series: vec!["over".into()],
        ..OverlayLayer::default()
    });
    let stored = config.overlays[0].y_axis.clone();
    for layer in [LayerRef::Base, LayerRef::Overlay(0)] {
        let mut chart = Chart::new(config.clone());
        chart
            .auto_fit_layer_union(r.pool(), layer, &series, 0.0)
            .unwrap();
        let y = &chart.config().left_y;
        assert!(y.min <= 20.0 && y.max >= 300.0, "{layer:?}: {y:?}");
        assert_eq!(chart.config().overlays[0].y_axis, stored, "{layer:?}");
    }
    // Unlinked, the parent's Y fit leaves the overlay's series out.
    config.overlays[0].link_y = false;
    let mut chart = Chart::new(config);
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    assert!(chart.config().left_y.max < 150.0);
}

/// The GPU auto-fit of the active layer (`begin_auto_fit_all`, the WASM
/// `auto_fit_all`) follows the links: with the overlay with its own X axis
/// active it fits that axis to the overlay's series and leaves the parent's;
/// with the base active it fits the parent's X to the base series only.
#[test]
fn the_gpu_auto_fit_follows_the_overlay_links() {
    let mut r = renderer();
    let series = fixture(&mut r);
    r.ensure_errorbar_extent_engine_blocking().unwrap();
    let id = r.register_chart(with_own_x(), series.to_vec()).unwrap();
    let fit = |r: &mut Renderer| {
        let ticket = r.begin_auto_fit_all(id, 0.0).unwrap();
        let resolved = ticket.resolve_blocking().unwrap();
        r.commit_auto_fit_all(resolved).unwrap();
        r.chart_config(id).unwrap().clone()
    };
    r.set_active_layer(id, LayerRef::Overlay(0)).unwrap();
    let before = r.chart_config(id).unwrap().bottom_x.clone();
    let config = fit(&mut r);
    let own = &config.overlays[0].x_axis;
    assert!(own.min <= 130.0 && own.min > 100.0, "{own:?}");
    assert!(own.max >= 175.0 && own.max < 200.0, "{own:?}");
    assert_eq!(
        (config.bottom_x.min, config.bottom_x.max),
        (before.min, before.max)
    );
    r.set_active_layer(id, LayerRef::Base).unwrap();
    let config = fit(&mut r);
    let x = &config.bottom_x;
    assert!(x.min <= 2.0 && x.max >= 8.0 && x.max < 20.0, "{x:?}");
}

/// An overlay of a subplot fits its own X axis from its own series, and its
/// followed Y axis is the subplot's.
#[test]
fn a_subplot_overlay_fit_follows_its_links() {
    let mut r = renderer();
    let series = fixture(&mut r);
    let mut config = bare();
    let mut sub = with_own_x();
    sub.chart_area.0.y = HEIGHT;
    sub.overlays[0].link_y = true;
    config.subplots = vec![renderer::Subplot {
        config: sub,
        series: vec!["over".into()],
        ..renderer::Subplot::default()
    }];
    let mut chart = Chart::new(config);
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::SubplotOverlay(0, 0), &series, 0.0)
        .unwrap();
    let sub = &chart.config().subplots[0].config;
    let own = &sub.overlays[0].x_axis;
    assert!(
        own.min <= 130.0 && own.max >= 175.0 && own.max < 200.0,
        "{own:?}"
    );
    let y = &sub.left_y;
    assert!(y.min <= 10.0 && y.max >= 35.0 && y.max < 60.0, "{y:?}");
}
