//! Interactions on probability axes (#209, CLAUDE.md rule 6), through the
//! public renderer API and the headless `export_rgba`: the picked highlight
//! at a tail point, `step_selection`, the annotation layer and its pointer
//! round trip, the same placement at 1x and 2x, a moved and resized plot
//! area, an overlay layer on a logistic axis, linked Weibull subplots, and
//! the documented f32 error of a narrow window.
//!
//! The draw and pick agreement of each drawn kind is in
//! `pick_draw_agreement.rs`; decoration drag and the plot-area drag of the
//! `ProbabilityPaper` scene are in `decoration_oracle.rs`; the edits of every
//! probability scale are in the SSOT invalidation matrix.
//!
//! Grid, legend, title and axis lines are off, and every mark has its own
//! pure colour, so a colour predicate attributes every pixel to one mark.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::{AxisScale, ProbabilityScale};
use renderer::data::Column;
use renderer::data_config::{DataScatterStyleConfig, ScatterShape, XySeries};
use renderer::layout::{ChartArea, Rect};
use renderer::{Chart, Color, DataRenderType, RasterImage, Renderer, RendererDevice, SeriesConfig};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const GREEN: Color = Color::from_rgb8(0, 255, 0);
/// The default picked-highlight colour.
const GOLD: Color = Color::from_rgb8(255, 215, 0);
const SCALES: [ProbabilityScale; 3] = [
    ProbabilityScale::Normal,
    ProbabilityScale::Weibull,
    ProbabilityScale::Logistic,
];

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init")
}

fn upload(r: &mut Renderer, id: &str, data: Vec<f64>) {
    r.add_column(id, &Column { data }).unwrap();
}

/// A bare chart: x over 0..10, y a probability axis of `kind` over
/// `y_min..y_max`.
fn config(kind: ProbabilityScale, y_min: f64, y_max: f64) -> renderer::Config {
    chart_with_y(AxisScale::Probability(kind), y_min, y_max)
}

/// A bare chart: x over 0..10, y on `scale` over `y_min..y_max`, set through
/// the range helpers as a host does.
fn chart_with_y(scale: AxisScale, y_min: f64, y_max: f64) -> renderer::Config {
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
    chart.with_axis_range_change(|c| {
        for axis in [&mut c.left_y, &mut c.right_y] {
            axis.scale = scale.clone();
        }
    });
    chart.set_y_range(y_min, y_max);
    chart.config().clone()
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

fn export(
    r: &mut Renderer,
    config: &renderer::Config,
    series: &[SeriesConfig],
    scale: f32,
) -> RasterImage {
    let chart = Chart::new(config.clone());
    r.export_rgba(
        renderer::ExportSource::chart(&chart, series),
        // The whole document: overlay layers and subplots draw too.
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

/// The data area of `config` at `scale`, in image pixels.
fn data_area(config: &renderer::Config, scale: f32) -> (f32, f32, f32, f32) {
    let da = config.scaled(scale).data_area().unwrap().0;
    (da.x as f32, da.y as f32, da.width as f32, da.height as f32)
}

/// Where the chrome of `config` places `(x, y)` at `scale`, against its own
/// `left_y` (or `y_axis` when given).
fn chrome_at(
    config: &renderer::Config,
    y_axis: Option<&renderer::config::AxisOptions>,
    x: f64,
    y: f64,
    scale: f32,
) -> (f32, f32) {
    let (ax, ay, aw, ah) = data_area(config, scale);
    let fx = renderer::layout::axis_fraction(x, &config.bottom_x);
    let fy = renderer::layout::axis_fraction(y, y_axis.unwrap_or(&config.left_y));
    (ax + aw * fx, ay + ah * (1.0 - fy))
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

/// Points in both tails and at the median, inside the [`LO`]..[`HI`] axis
/// far enough from its ends that no mark or highlight ring is cut by the
/// data area's edge on any of the three papers.
const TAIL_POINTS: [(f64, f64); 3] = [(2.0, 0.002), (5.0, 0.5), (8.0, 0.98)];
const LO: f64 = 1e-5;
const HI: f64 = 1.0 - 1e-5;

fn tail_fixture(r: &mut Renderer) -> [SeriesConfig; 1] {
    upload(r, "tx", TAIL_POINTS.iter().map(|p| p.0).collect());
    upload(r, "ty", TAIL_POINTS.iter().map(|p| p.1).collect());
    [markers("m", "tx", "ty", RED)]
}

/// The picked highlight rings the mark where the probability axis puts it,
/// at both tails, for every scale, at 1x and 2x.
#[test]
fn the_picked_highlight_rings_a_tail_point() {
    let mut r = renderer();
    let series = tail_fixture(&mut r);
    for kind in SCALES {
        let plain = config(kind, LO, HI);
        for index in [0, 2] {
            let (x, y) = TAIL_POINTS[index];
            let config = picked(plain.clone(), "m", index);
            for scale in [1.0f32, 2.0] {
                let img = export(&mut r, &config, &series, scale);
                let at = chrome_at(&config, None, x, y, scale);
                let ring = centroid(&img, at, 14.0 * scale, is(GOLD)).unwrap_or_else(|| {
                    panic!("{kind:?} {scale}x point {index}: no highlight at {at:?}")
                });
                assert!(
                    (ring.0 - at.0).abs() < 1.5 && (ring.1 - at.1).abs() < 1.5,
                    "{kind:?} {scale}x point {index}: ring at {ring:?}, mark at {at:?}"
                );
            }
        }
    }
}

/// `step_selection` walks the indices of a series on a probability axis,
/// and the highlight follows to the next point.
#[test]
fn a_step_moves_the_selection_to_the_next_point() {
    let mut r = renderer();
    let series = tail_fixture(&mut r);
    let mut config = picked(config(ProbabilityScale::Normal, LO, HI), "m", 0);
    let stepped = config
        .step_selection(&series, 1, 0, |_| Some(TAIL_POINTS.len()))
        .expect("a point selection steps");
    assert!(stepped);
    assert_eq!(
        config.picked_data.as_ref().unwrap().refs[0],
        renderer::PickedDataRef::Point {
            source_id: None,
            series_id: "m".into(),
            point_index: 1,
        }
    );
    let img = export(&mut r, &config, &series, 1.0);
    let (x, y) = TAIL_POINTS[1];
    let at = chrome_at(&config, None, x, y, 1.0);
    let ring = centroid(&img, at, 14.0, is(GOLD)).expect("the highlight moved to point 1");
    assert!((ring.0 - at.0).abs() < 1.5 && (ring.1 - at.1).abs() < 1.5);
}

/// Every point sits at the same place at 1x and 2x: the 2x centroid is the
/// 1x centroid doubled, within the pixel snapping of each.
#[test]
fn one_x_and_two_x_place_every_point_alike() {
    let mut r = renderer();
    let series = tail_fixture(&mut r);
    for kind in SCALES {
        let config = config(kind, LO, HI);
        let one = export(&mut r, &config, &series, 1.0);
        let two = export(&mut r, &config, &series, 2.0);
        for &(x, y) in &TAIL_POINTS {
            let at1 = chrome_at(&config, None, x, y, 1.0);
            let at2 = chrome_at(&config, None, x, y, 2.0);
            let c1 = centroid(&one, at1, 8.0, is(RED)).expect("1x mark");
            let c2 = centroid(&two, at2, 16.0, is(RED)).expect("2x mark");
            assert!(
                (c2.0 - 2.0 * c1.0).abs() <= 1.5 && (c2.1 - 2.0 * c1.1).abs() <= 1.5,
                "{kind:?} p = {y}: 1x {c1:?}, 2x {c2:?}"
            );
        }
    }
}

/// The annotation layer reads the probability axis: a span between two
/// probabilities and a reference line at the median are drawn where the
/// chrome puts them, a data anchor at `p = 0.999` round trips through the
/// pointer conversion, and the conversion agrees with the chrome.
#[test]
fn annotations_place_and_round_trip_on_a_probability_axis() {
    use renderer::AnnotationSpace;
    use renderer::annotation::pixel_space_for_config;
    use renderer::{Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationKind};
    let mut r = renderer();
    let series = tail_fixture(&mut r);
    for kind in SCALES {
        let mut config = config(kind, LO, HI);
        config.annotations.items = vec![
            Annotation::new(AnnotationKind::Span {
                axis: AnnotationAxis::Y,
                from: 0.01,
                to: 0.1,
                fill: GREEN,
            }),
            Annotation::new(AnnotationKind::ReferenceLine {
                axis: AnnotationAxis::Y,
                value: 0.9,
                style: renderer::AnnotationLineStyle {
                    color: BLUE,
                    width: 3.0,
                    ..Default::default()
                },
                label: None,
            }),
        ];
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &config, &series, scale);
            let column = data_area(&config, scale).0 + 10.0 * scale;
            let line_row = chrome_at(&config, None, 0.0, 0.9, scale).1;
            let line = centroid(&img, (column, line_row), 6.0 * scale, is(BLUE))
                .unwrap_or_else(|| panic!("{kind:?} {scale}x: no reference line"));
            assert!(
                (line.1 - line_row).abs() < 1.0,
                "{kind:?} {scale}x: the line at row {}, the chrome at {line_row}",
                line.1
            );
            let top = chrome_at(&config, None, 0.0, 0.1, scale).1;
            let bottom = chrome_at(&config, None, 0.0, 0.01, scale).1;
            for (row, inside) in [
                (top + 2.0, true),
                (bottom - 2.0, true),
                (top - 3.0, false),
                (bottom + 3.0, false),
            ] {
                let p = pixel(&img, column as u32, row as u32);
                assert_eq!(
                    is(GREEN)(p),
                    inside,
                    "{kind:?} {scale}x: span at row {row} (top {top}, bottom {bottom})"
                );
            }
        }
        let space = pixel_space_for_config(&config);
        for p in [0.0001, 0.002, 0.5, 0.9, 0.98, 0.9999] {
            let px = space
                .value_to_px(AnnotationAxis::Y, p)
                .expect("inside the axis");
            let row = chrome_at(&config, None, 0.0, p, 1.0).1;
            assert!(
                (px - row).abs() < 0.01,
                "{kind:?} p = {p}: the space at {px}, the chrome at {row}"
            );
            // The pixel is an f32: the round trip holds to its resolution,
            // which near a tail is a small part of the tail itself.
            let back = space.px_to_value(AnnotationAxis::Y, px).unwrap();
            assert!(
                (back - p).abs() <= 1e-5 * p.min(1.0 - p),
                "{kind:?} p = {p}: back {back}"
            );
        }
        let anchor = Anchor::Data { x: 5.0, y: 0.999 };
        let px = space.anchor_to_px(&anchor).expect("a data anchor places");
        match space.px_to_anchor(px, AnchorKind::Data) {
            Some(Anchor::Data { x, y }) => {
                assert!((x - 5.0).abs() < 1e-4, "{kind:?}: x {x}");
                assert!((y - 0.999).abs() < 1e-7, "{kind:?}: y {y}");
            }
            other => panic!("{kind:?}: {other:?}"),
        }
        assert_eq!(space.px_to_value(AnnotationAxis::Y, px[1] + 1e6), None);
    }
}

/// A plot area moved and resized through the drag API is drawn and picked
/// where its rebuilt transform puts each point.
#[test]
fn a_moved_and_resized_plot_area_picks_where_it_draws() {
    use renderer::layout::{Element, NudgeResult, Side};
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    let series = tail_fixture(&mut r);
    let measure =
        renderer::CpuTextMeasure::for_style(&renderer::default::default_config().draw_style);
    for kind in SCALES {
        let mut config = config(kind, LO, HI);
        assert_eq!(
            config.nudge(Element::DataArea, &measure, 12.0, -8.0),
            NudgeResult::Moved
        );
        assert_eq!(
            config.nudge(Element::DataAreaEdge(Side::Right), &measure, -40.0, 0.0),
            NudgeResult::Moved
        );
        assert_eq!(
            config.nudge(Element::DataAreaEdge(Side::Top), &measure, 0.0, 30.0),
            NudgeResult::Moved
        );
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &config, &series, scale);
            for (index, &(x, y)) in TAIL_POINTS.iter().enumerate() {
                let at = chrome_at(&config, None, x, y, scale);
                let mark = centroid(&img, at, 8.0 * scale, is(RED))
                    .unwrap_or_else(|| panic!("{kind:?} {scale}x: no mark {index}"));
                assert!(
                    (mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0,
                    "{kind:?} {scale}x point {index}: drawn at {mark:?}, chrome {at:?}"
                );
                assert_eq!(
                    pick_at(&mut r, &config, &series, at, scale),
                    Some(("m".into(), index)),
                    "{kind:?} {scale}x point {index}"
                );
            }
        }
    }
}

/// An overlay layer whose axis is logistic places its series by that axis,
/// over a linear base axis, and picks it there.
#[test]
fn an_overlay_on_a_logistic_axis_draws_and_picks_by_its_axis() {
    let mut r = renderer();
    r.enable_gpu_picking().unwrap();
    upload(&mut r, "bx", vec![1.0, 9.0]);
    upload(&mut r, "by", vec![10.0, 90.0]);
    let points = [(3.0, 0.02), (6.0, 0.5), (8.5, 0.97)];
    upload(&mut r, "ox", points.iter().map(|p| p.0).collect());
    upload(&mut r, "oy", points.iter().map(|p| p.1).collect());
    let series = [
        markers("base", "bx", "by", BLUE),
        markers("over", "ox", "oy", RED),
    ];
    let mut config = chart_with_y(AxisScale::Linear, 0.0, 100.0);
    let mut overlay = renderer::OverlayLayer {
        series: vec!["over".into()],
        ..Default::default()
    };
    overlay.y_axis.scale = AxisScale::Probability(ProbabilityScale::Logistic);
    overlay.y_axis.min = 0.01;
    overlay.y_axis.max = 0.99;
    overlay.y_axis.out_margin = 60.0;
    config.overlays.push(overlay);
    config
        .validate_chart()
        .expect("a logistic overlay axis validates");
    let overlay_axis = config.overlays[0].y_axis.clone();
    // A pick answers for the active layer's series, as a host's pointer
    // does once the user has made the overlay active.
    let mut active = config.clone();
    active.set_active_layer(renderer::LayerRef::Overlay(0));
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        for (index, &(x, y)) in points.iter().enumerate() {
            let at = chrome_at(&config, Some(&overlay_axis), x, y, scale);
            let mark = centroid(&img, at, 8.0 * scale, is(RED))
                .unwrap_or_else(|| panic!("{scale}x: no overlay mark {index} at {at:?}"));
            assert!(
                (mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0,
                "{scale}x overlay point {index}: drawn at {mark:?}, chrome {at:?}"
            );
            assert_eq!(
                pick_at(&mut r, &active, &series, at, scale),
                Some(("over".into(), index)),
                "{scale}x overlay point {index}"
            );
        }
    }
}

/// A subplot linked on y to a Weibull parent takes the parent's scale and
/// range, and its series lands where its own chrome puts it.
#[test]
fn a_subplot_linked_to_a_weibull_axis_takes_its_scale() {
    let mut r = renderer();
    upload(&mut r, "sx", vec![2.0, 7.0]);
    upload(&mut r, "sy", vec![0.01, 0.95]);
    let series = [markers("sub", "sx", "sy", RED)];
    let mut config = config(ProbabilityScale::Weibull, LO, HI);
    config.chart_area.0.width = WIDTH / 2;
    let mut sub_config = config.clone();
    sub_config.chart_area.0.x = WIDTH / 2;
    sub_config.left_y.scale = AxisScale::Linear;
    sub_config.left_y.min = 0.0;
    sub_config.left_y.max = 100.0;
    config.subplots = vec![renderer::Subplot {
        config: sub_config,
        series: vec!["sub".into()],
        link_x: None,
        link_y: Some(renderer::CellRef::Parent),
        marker: None,
    }];
    let linked = config.subplot_config(0).expect("the subplot resolves");
    assert_eq!(
        linked.left_y.scale,
        AxisScale::Probability(ProbabilityScale::Weibull)
    );
    assert_eq!((linked.left_y.min, linked.left_y.max), (LO, HI));
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        for (x, y) in [(2.0, 0.01), (7.0, 0.95)] {
            let at = chrome_at(&linked, None, x, y, scale);
            let mark = centroid(&img, at, 8.0 * scale, is(RED))
                .unwrap_or_else(|| panic!("{scale}x: no subplot mark at {at:?}"));
            assert!(
                (mark.0 - at.0).abs() < 1.0 && (mark.1 - at.1).abs() < 1.0,
                "{scale}x subplot p = {y}: drawn at {mark:?}, chrome {at:?}"
            );
        }
    }
}

/// The documented f32 limit (CONFIG.md, SHADER_COMMON.md §5.24): on a window
/// as narrow as `0.4999..0.5001` the GPU places a value at `Φ⁻¹(p)` in f32
/// before the range is subtracted, so a mark may sit off the f64 chrome by
/// up to about 2e-4 of the axis (1.6 px at 8192 px), plus the marker's own
/// pixel snapping. Pinned here so a regression past it fails.
#[test]
fn a_narrow_window_stays_within_the_documented_f32_error() {
    let mut r = renderer();
    let points = [(2.0, 0.49992), (5.0, 0.5), (8.0, 0.50008)];
    upload(&mut r, "nx", points.iter().map(|p| p.0).collect());
    upload(&mut r, "ny", points.iter().map(|p| p.1).collect());
    let series = [markers("n", "nx", "ny", RED)];
    let config = config(ProbabilityScale::Normal, 0.4999, 0.5001);
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &config, &series, scale);
        let height = data_area(&config, scale).3;
        let bound = 0.5 * scale + 2e-4 * height;
        for &(x, y) in &points {
            let at = chrome_at(&config, None, x, y, scale);
            let mark = centroid(&img, at, 8.0 * scale, is(RED))
                .unwrap_or_else(|| panic!("{scale}x: no mark at p = {y}"));
            assert!(
                (mark.1 - at.1).abs() <= bound,
                "{scale}x p = {y}: drawn at row {}, chrome {} (bound {bound})",
                mark.1,
                at.1
            );
        }
    }
}
