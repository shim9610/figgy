//! End-to-end verification of the shape annotations (`AnnotationKind::Rect`,
//! `Ellipse` and `Polygon`, #138) through the public renderer API: the
//! fills at 1x and 2x, the plot clip, the underlay against the data, the
//! hit test inside a concave shape, whole-shape drags, box-handle resizes
//! and vertex drags through the chart view's annotation map, the vector
//! export's closed paths, and the polar chart's data-anchor rule.
//!
//! Every shape is filled with an opaque primary colour and drawn without a
//! border, so a colour predicate attributes a pixel to one shape; the
//! expected positions come from the chart's own data area and ratio
//! anchors.

#[path = "support/svg_parts.rs"]
mod svg_parts;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::data::Column;
use renderer::layout::{ChartArea, Rect, RectF};
use renderer::resize::ResizeHandle;
use renderer::{
    Anchor, Annotation, AnnotationKind, AnnotationLineStyle, AnnotationSkip, Chart, Color,
    DataLineStyleConfig, DataRenderType, DrawnPart, LayerRef, RasterImage, RenderWarning, Renderer,
    RendererDevice, SeriesConfig, ShapeStyle, layout::NudgeResult,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;
const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const GREEN: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    static DEVICE: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    });
    (Arc::clone(device), Arc::clone(queue))
}

/// A renderer holding one horizontal line's columns, `y = 5` over `x` 0..10.
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column(
        "x",
        &Column {
            data: vec![0.0, 10.0],
            min: 0.0,
            max: 10.0,
        },
    )
    .unwrap();
    r.add_column(
        "y",
        &Column {
            data: vec![5.0, 5.0],
            min: 5.0,
            max: 5.0,
        },
    )
    .unwrap();
    r
}

/// A black line across the plot at `y = 5`, four pixels wide.
fn line() -> SeriesConfig {
    SeriesConfig {
        series_id: "line".into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_color: Color::BLACK,
                line_width: 4.0,
                ..Default::default()
            },
        },
    }
}

/// Grid, legend and title off, linear axes 0..10 both ways; the annotation
/// layer active with `items`.
fn chart(items: Vec<Annotation>) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 10.0;
    config.left_y.min = 0.0;
    config.left_y.max = 10.0;
    config.annotations.items = items;
    config.set_active_layer(LayerRef::Annotations);
    Chart::new(config)
}

fn filled(color: Color) -> ShapeStyle {
    ShapeStyle {
        fill: Some(color),
        border: AnnotationLineStyle {
            width: 0.0,
            ..Default::default()
        },
        below: false,
    }
}

fn rect(from: (f32, f32), to: (f32, f32), style: ShapeStyle) -> Annotation {
    Annotation::new(AnnotationKind::Rect {
        from: Anchor::Ratio {
            x: from.0,
            y: from.1,
        },
        to: Anchor::Ratio { x: to.0, y: to.1 },
        style,
    })
}

fn ellipse(from: (f32, f32), to: (f32, f32), style: ShapeStyle) -> Annotation {
    Annotation::new(AnnotationKind::Ellipse {
        from: Anchor::Ratio {
            x: from.0,
            y: from.1,
        },
        to: Anchor::Ratio { x: to.0, y: to.1 },
        style,
    })
}

/// A concave L: the top arm 0.1..0.5 × 0.5..0.6, the left arm 0.1..0.2 ×
/// 0.5..0.9, the notch 0.2..0.5 × 0.6..0.9 empty.
fn l_shape(style: ShapeStyle) -> Annotation {
    let v = |x: f32, y: f32| Anchor::Ratio { x, y };
    Annotation::new(AnnotationKind::Polygon {
        vertices: vec![
            v(0.1, 0.5),
            v(0.5, 0.5),
            v(0.5, 0.6),
            v(0.2, 0.6),
            v(0.2, 0.9),
            v(0.1, 0.9),
        ],
        style,
    })
}

/// The three test shapes: a red rectangle, a green ellipse, a blue L.
fn shapes() -> Vec<Annotation> {
    vec![
        rect((0.1, 0.1), (0.3, 0.3), filled(RED)),
        ellipse((0.5, 0.1), (0.9, 0.5), filled(GREEN)),
        l_shape(filled(BLUE)),
    ]
}

/// The plot rectangle of `chart` in export pixels at `scale`.
fn plot(chart: &Chart, scale: f32) -> RectF {
    let da = chart.config().data_area().expect("data area");
    RectF {
        x: da.x as f32 * scale,
        y: da.y as f32 * scale,
        width: da.width as f32 * scale,
        height: da.height as f32 * scale,
    }
}

/// The export pixel of ratio anchor `(rx, ry)` at `scale`.
fn at(chart: &Chart, scale: f32, rx: f32, ry: f32) -> (f32, f32) {
    let p = plot(chart, scale);
    (p.x + rx * p.width, p.y + ry * p.height)
}

fn pixel(img: &RasterImage, (x, y): (f32, f32)) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, img.width as i64 - 1) as u32;
    let y = (y.floor() as i64).clamp(0, img.height as i64 - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

fn is_red(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_green(p: [u8; 4]) -> bool {
    p[3] > 200 && p[1] > 200 && p[0] < 60 && p[2] < 60
}
fn is_blue(p: [u8; 4]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}
fn is_black(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] < 40 && p[1] < 40 && p[2] < 40
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_document_rgba(chart, series, scale).unwrap()
}

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            renderer::encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

/// A rectangle fills its box and nothing past it, an ellipse its centre
/// and 0.95 of both semi-axes but not 1.05, a concave polygon both arms
/// and not its notch, at 1x and 2x.
#[test]
fn shapes_fill_inside_and_nothing_outside_at_1x_and_2x() {
    let mut r = renderer();
    let chart = chart(shapes());
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &[], scale);
        save(&format!("shapes-{scale}x"), &img);
        let p = |rx: f32, ry: f32| pixel(&img, at(&chart, scale, rx, ry));
        let slack = 3.0 * scale / plot(&chart, scale).width;
        // The rectangle: inside, and just past each edge not.
        assert!(is_red(p(0.2, 0.2)), "{scale}x: the rectangle's centre");
        assert!(is_red(p(0.1 + slack, 0.1 + slack)), "{scale}x: its corner");
        assert!(
            !is_red(p(0.3 + slack, 0.2)),
            "{scale}x: past its right edge"
        );
        assert!(
            !is_red(p(0.2, 0.3 + slack * 1.4)),
            "{scale}x: past its bottom"
        );
        // The ellipse: the centre and 0.95 of the semi-axes, not 1.05.
        let (cx, cy, ax, ay) = (0.7, 0.3, 0.2, 0.2);
        assert!(is_green(p(cx, cy)), "{scale}x: the ellipse's centre");
        for (dx, dy) in [(0.95, 0.0), (-0.95, 0.0), (0.0, 0.95), (0.0, -0.95)] {
            assert!(
                is_green(p(cx + ax * dx, cy + ay * dy)),
                "{scale}x: inside the ellipse at {dx} {dy}"
            );
        }
        for (dx, dy) in [(1.05, 0.0), (-1.05, 0.0), (0.0, 1.05), (0.0, -1.05)] {
            assert!(
                !is_green(p(cx + ax * dx, cy + ay * dy)),
                "{scale}x: outside the ellipse at {dx} {dy}"
            );
        }
        assert!(
            !is_green(p(cx + ax * 0.9, cy + ay * 0.9)),
            "{scale}x: the box corner"
        );
        // The L: both arms, not the notch.
        assert!(is_blue(p(0.3, 0.55)), "{scale}x: the top arm");
        assert!(is_blue(p(0.15, 0.8)), "{scale}x: the left arm");
        assert!(!is_blue(p(0.35, 0.75)), "{scale}x: the notch");
    }
}

/// A shape straddling the plot's edge is clipped at it; one entirely
/// outside the plot is skipped and reported.
#[test]
fn a_shape_is_clipped_to_the_plot() {
    let mut r = renderer();
    let chart = chart(vec![
        rect((0.8, 0.2), (1.3, 0.4), filled(RED)),
        ellipse((1.1, 0.5), (1.4, 0.8), filled(GREEN)),
    ]);
    let img = export(&mut r, &chart, &[], 1.0);
    save("shapes-clipped", &img);
    let p = plot(&chart, 1.0);
    assert!(is_red(pixel(&img, at(&chart, 1.0, 0.95, 0.3))));
    assert!(!is_red(pixel(
        &img,
        (p.x + p.width + 3.0, at(&chart, 1.0, 0.0, 0.3).1)
    )));
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let map = view.annotation_map();
    assert!(map.find(0).is_some(), "the straddling rectangle is drawn");
    assert_eq!(map.find(1), None, "the outside ellipse is not");
    assert!(
        map.warnings().iter().any(|w| matches!(
            w,
            RenderWarning::AnnotationSkipped {
                index: 1,
                reason: AnnotationSkip::OutOfRange
            }
        )),
        "{:?}",
        map.warnings()
    );
}

/// A shape with `below` sits under the data, one without over it.
#[test]
fn a_below_shape_sits_under_the_data_and_an_above_one_over_it() {
    let mut r = renderer();
    let series = [line()];
    for below in [true, false] {
        let chart = chart(vec![rect(
            (0.2, 0.2),
            (0.8, 0.8),
            ShapeStyle {
                below,
                ..filled(RED)
            },
        )]);
        let img = export(&mut r, &chart, &series, 1.0);
        save(&format!("shapes-below-{below}"), &img);
        // The line at y = 5 is the plot's middle row.
        let on_line = pixel(&img, at(&chart, 1.0, 0.5, 0.5));
        if below {
            assert!(is_black(on_line), "below: the line shows over the fill");
        } else {
            assert!(is_red(on_line), "above: the fill covers the line");
        }
        assert!(is_red(pixel(&img, at(&chart, 1.0, 0.5, 0.3))));
    }
}

/// The hit map answers a shape anywhere inside it and not in a concave
/// notch; a drag moves the whole shape, a box handle one side, a vertex
/// handle one vertex, and a drag off the plot is refused.
#[test]
fn shapes_hit_drag_resize_and_move_their_vertices() {
    let r = renderer();
    let chart = chart(shapes());
    let area = chart.config().chart_area.0;
    let view = r.create_chart_view(&chart, area).unwrap();
    let map = view.annotation_map();
    let at1 = |rx: f32, ry: f32| {
        let (x, y) = at(&chart, 1.0, rx, ry);
        [x, y]
    };
    let index_at = |pos: [f32; 2]| {
        map.hit_test(pos)
            .and_then(|id| map.element(id))
            .map(|e| e.index)
    };
    assert_eq!(index_at(at1(0.2, 0.2)), Some(0), "the rectangle");
    assert_eq!(index_at(at1(0.7, 0.3)), Some(1), "the ellipse");
    assert_eq!(index_at(at1(0.3, 0.55)), Some(2), "the L's top arm");
    assert_eq!(index_at(at1(0.15, 0.8)), Some(2), "the L's left arm");
    assert_eq!(index_at(at1(0.35, 0.75)), None, "the L's notch");
    assert_eq!(index_at(at1(0.52, 0.12)), None, "the ellipse's box corner");

    // A whole-shape drag by (7, 5) moves the bounds by (7, 5).
    let l = map.find(2).unwrap();
    let before = map.bounds(l).unwrap();
    let mut layer = chart.config().annotations.clone();
    assert_eq!(
        map.drag_by(&mut layer, l, 7.0, 5.0),
        Some(NudgeResult::Moved)
    );
    let mut moved = chart.config().clone();
    moved.annotations = layer;
    let view2 = r.create_chart_view(&Chart::new(moved), area).unwrap();
    let after = view2
        .annotation_map()
        .bounds(view2.annotation_map().find(2).unwrap())
        .unwrap();
    assert!(
        (after.x - before.x - 7.0).abs() < 0.5
            && (after.y - before.y - 5.0).abs() < 0.5
            && (after.width - before.width).abs() < 0.5,
        "{before:?} -> {after:?}"
    );

    // The rectangle's east handle widens it by the drag.
    let rect_id = map.find(0).unwrap();
    let rect_before = map.bounds(rect_id).unwrap();
    let east = [
        rect_before.x + rect_before.width + 2.0,
        rect_before.y + rect_before.height * 0.5,
    ];
    assert_eq!(map.hit_resize_handle(rect_id, east), Some(ResizeHandle::E));
    let mut layer = chart.config().annotations.clone();
    assert_eq!(
        map.resize_by(&mut layer, rect_id, ResizeHandle::E, 20.0, 9.0),
        Some(NudgeResult::Moved)
    );
    let mut resized = chart.config().clone();
    resized.annotations = layer;
    let view3 = r.create_chart_view(&Chart::new(resized), area).unwrap();
    let rect_after = view3
        .annotation_map()
        .bounds(view3.annotation_map().find(0).unwrap())
        .unwrap();
    assert!(
        (rect_after.x - rect_before.x).abs() < 0.5
            && (rect_after.width - rect_before.width - 20.0).abs() < 0.5
            && (rect_after.height - rect_before.height).abs() < 0.5,
        "{rect_before:?} -> {rect_after:?}"
    );

    // The L's vertex 2 (the inner corner at 0.5, 0.6) is a vertex handle
    // and not a compass one; dragging it moves that vertex only.
    let corner = at1(0.5, 0.6);
    assert_eq!(map.hit_vertex(l, corner), Some(2));
    assert_eq!(map.hit_resize_handle(l, corner), None);
    let mut layer = chart.config().annotations.clone();
    assert_eq!(
        map.drag_vertex(&mut layer, l, 2, 0.0, 15.0),
        Some(NudgeResult::Moved)
    );
    let AnnotationKind::Polygon { vertices, .. } = &layer.items[2].kind else {
        unreachable!()
    };
    let Anchor::Ratio { x, y } = vertices[2] else {
        unreachable!()
    };
    let p = plot(&chart, 1.0);
    assert!(
        (x - 0.5).abs() < 1e-5 && (y - (0.6 + 15.0 / p.height)).abs() < 1e-4,
        "{x} {y}"
    );
    assert_eq!(vertices[1], Anchor::Ratio { x: 0.5, y: 0.5 });

    // Off the plot: refused and unchanged.
    let mut layer = chart.config().annotations.clone();
    let unchanged = layer.clone();
    assert!(matches!(
        map.drag_by(&mut layer, rect_id, 10_000.0, 0.0),
        Some(NudgeResult::Rejected(_))
    ));
    assert_eq!(layer, unchanged);
}

/// The vector export writes each shape as one closed path: the ellipse's
/// sixty-four chords read back as a polygon, the rectangle as a rectangle.
#[test]
fn the_vector_export_writes_closed_paths_for_shapes() {
    let mut r = renderer();
    let chart = chart(shapes());
    let svg = r.export_document_svg(&chart, &[], 1.0).unwrap();
    let file = svg_parts::read(&svg);
    let polygons: Vec<usize> = file
        .parts
        .iter()
        .filter_map(|p| match p {
            svg_parts::ReadPart::Part(DrawnPart::Polygon { points, .. }, None) => {
                Some(points.len())
            }
            _ => None,
        })
        .collect();
    assert!(
        polygons.contains(&64) && polygons.contains(&6),
        "the ellipse and the L as filled polygons: {polygons:?}"
    );
    assert!(
        file.parts
            .iter()
            .any(|p| matches!(p, svg_parts::ReadPart::Part(DrawnPart::Rect(_), None))),
        "the rectangle as a filled rectangle"
    );
}

/// On a polar chart a data-anchored shape is skipped with the
/// `NoValueAxis` warning while a ratio-anchored one draws.
#[test]
fn data_anchored_shapes_are_skipped_on_a_polar_chart() {
    let mut r = renderer();
    let mut chart = chart(vec![
        Annotation::new(AnnotationKind::Rect {
            from: Anchor::Data { x: 1.0, y: 1.0 },
            to: Anchor::Data { x: 3.0, y: 3.0 },
            style: filled(RED),
        }),
        ellipse((0.4, 0.4), (0.6, 0.6), filled(GREEN)),
    ]);
    chart.config_mut().polar = Some(Default::default());
    let img = export(&mut r, &chart, &[], 1.0);
    save("shapes-polar", &img);
    assert!(is_green(pixel(&img, at(&chart, 1.0, 0.5, 0.5))));
    let view = r
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    let map = view.annotation_map();
    assert_eq!(map.find(0), None);
    assert!(map.find(1).is_some());
    assert!(map.warnings().iter().any(|w| matches!(
        w,
        RenderWarning::AnnotationSkipped {
            index: 0,
            reason: AnnotationSkip::NoValueAxis
        }
    )));
}

/// The shapes round-trip through the config document with their default
/// style omitted.
#[test]
fn shapes_round_trip_through_the_config() {
    let chart = chart(shapes());
    let json = serde_json::to_string(chart.config()).unwrap();
    assert!(json.contains("\"Ellipse\"") && json.contains("\"Polygon\""));
    let back: renderer::Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back.annotations, chart.config().annotations);
}
