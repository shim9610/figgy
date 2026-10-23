//! The resident series-body cache: each panel's fills and bodies are drawn
//! into their own texture and drawn again only when the data layer changes.
//! Chrome, annotation, selection and pick edits, and edits of another layer,
//! never draw them again. The pixel oracle is the same prepared frame painted
//! with the cache stripped, i.e. the direct draw.
use super::super::*;
use super::{declaration, read_draw_target};
use crate::annotation::{Anchor, Annotation, AnnotationKind};
use crate::config::{PickedPointRef, PickedPointsConfig, SketchOptions};
use crate::layout::ChartArea;
use crate::text::RichText;
use crate::{LayerRef, OverlayLayer};

const PANEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 320,
    height: 240,
};
const POINTS: usize = 200;

fn column(data: Vec<f32>) -> crate::Column<f32> {
    let min = data.iter().copied().fold(f32::INFINITY, f32::min);
    let max = data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    crate::Column { data, min, max }
}

fn wave(phase: f32, scale: f32) -> crate::Column<f32> {
    column(
        (0..POINTS)
            .map(|i| scale * (0.5 + 0.4 * (i as f32 * 0.07 + phase).sin()))
            .collect(),
    )
}

fn renderer(format: wgpu::TextureFormat, samples: u32) -> Renderer {
    let (device, queue) = data_render::shared_device().unwrap();
    let mut r = Renderer::try_new_with_sample_count(
        RendererDevice::new(device, queue),
        format,
        1 << 20,
        samples,
    )
    .unwrap();
    let x = column(
        (0..POINTS)
            .map(|i| i as f32 / (POINTS - 1) as f32)
            .collect(),
    );
    r.add_column("x", &x).unwrap();
    r.add_column("a", &wave(0.0, 1.0)).unwrap();
    r.add_column("b", &wave(1.3, 60.0)).unwrap();
    r.add_column("c", &wave(2.1, 1.0)).unwrap();
    r
}

fn text(s: &str) -> RichText {
    RichText::plain(s, Color::BLACK, 11.0, "")
}

fn base_config() -> Config {
    let mut c = crate::default::default_config();
    c.chart_area = ChartArea(PANEL);
    c.chart_title.visible = true;
    c.chart_title.text = text("Bodies");
    c.chart_title.top_margin = 20.0;
    c.legend.visible = true;
    c.legend.content = text("wave");
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        a.min = 0.0;
        a.max = 1.0;
        a.major_spacing = 0.25;
        a.minor_count = 1;
        a.out_margin = 30.0;
        a.label_style.font_size = 10.0;
    }
    c
}

/// A line with markers: both body kinds and a pickable point.
fn series(id: &str, y: &str) -> SeriesConfig {
    let mut s = declaration(id, "x", y);
    let DataRenderType::Line { line } = s.render_type.clone() else {
        unreachable!("the declaration is a line")
    };
    s.render_type = DataRenderType::ScatterLine {
        scatter: crate::data_config::DataScatterStyleConfig {
            point_color: Color::new(0.1, 0.3, 0.9, 0.8),
            point_shape: crate::ScatterShape::Circle,
            point_size: 6.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        },
        line,
    };
    s
}

fn overlay(min: f64, max: f64, series: &str) -> OverlayLayer {
    let mut overlay = OverlayLayer::default();
    overlay.y_axis.min = min;
    overlay.y_axis.max = max;
    overlay.y_axis.major_spacing = (max - min) / 4.0;
    overlay.y_axis.minor_count = 1;
    overlay.y_axis.out_margin = 40.0;
    overlay.y_axis.label_style.font_size = 10.0;
    overlay.series = vec![series.into()];
    overlay
}

fn target(r: &Renderer, samples: u32, size: (u32, u32)) -> wgpu::Texture {
    r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resident display test target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: r.surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Paint `frame` into a fresh white target of `size`.
fn try_paint(
    r: &Renderer,
    frame: &PreparedFrame,
    samples: u32,
    size: (u32, u32),
) -> Result<Vec<u8>> {
    let target = target(r, samples, size);
    let view = target.create_view(&Default::default());
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let painted = {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        r.paint_prepared(&mut pass, size, frame)
    };
    r.queue.submit([encoder.finish()]);
    painted.map(|()| read_draw_target(r, &target))
}

fn prepare(r: &mut Renderer, id: ChartId, view: &ChartView) -> PreparedFrame {
    r.prepare_registered(&[RegisteredChartDrawItem { chart_id: id, view }])
        .unwrap()
}

/// Paint `frame` with its cached bodies, then again with them stripped (the
/// direct draw), and require the two to agree up to UNORM rounding: at most
/// four levels anywhere (six in an sRGB target), and more than one level in
/// at most 0.5% of the channels. Returns the cached pixels.
fn painted_like_direct(r: &Renderer, mut frame: PreparedFrame, samples: u32) -> Vec<u8> {
    assert!(
        frame
            .items
            .iter()
            .all(|item| item.resident_display.is_some()),
        "every Precise panel carries cached bodies"
    );
    let size = (PANEL.width, PANEL.height);
    let cached = try_paint(r, &frame, samples, size).unwrap();
    for item in &mut frame.items {
        item.resident_display = None;
    }
    let direct = try_paint(r, &frame, samples, size).unwrap();
    let max_error = cached
        .iter()
        .zip(&direct)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    let off_by_more_than_one = cached
        .iter()
        .zip(&direct)
        .filter(|(a, b)| a.abs_diff(**b) > 1)
        .count();
    // The bodies are stored premultiplied in UNORM and composited once more,
    // so a pixel under several translucent primitives is rounded twice per
    // layer: a few levels at a few pixels, never a shape.
    // An sRGB target stores the premultiplied composite encoded, which
    // spreads the same linear rounding over a few more levels in dark
    // channels.
    let limit = if r.surface_format.is_srgb() { 6 } else { 4 };
    assert!(
        max_error <= limit,
        "cached bodies differ from the direct draw by {max_error}"
    );
    assert!(
        off_by_more_than_one * 200 <= cached.len(),
        "{off_by_more_than_one} of {} channels differ by more than one level",
        cached.len()
    );
    cached
}

/// Refresh every panel of `id` with the element `selected` names selected
/// (`None` clears the selection).
fn refresh(r: &mut Renderer, id: ChartId, view: &mut ChartView, selected: Option<&str>) {
    let config = r.chart_config(id).unwrap().clone();
    r.refresh_axis_with_selection(view, &Chart::new(config), PANEL, selected)
        .unwrap();
}

fn edit(r: &mut Renderer, id: ChartId, view: &mut ChartView, apply: impl FnOnce(&mut Config)) {
    let mut config = r.chart_config(id).unwrap().clone();
    apply(&mut config);
    r.set_chart_config(id, config).unwrap();
    refresh(r, id, view, None);
}

fn font_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::text_render::FONT_REGISTRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// A repaint with nothing changed blits the bodies drawn by the first frame:
// no draw on any later frame, at 1x and 4x, in a UNORM and an sRGB target, and
// the blit matches the direct draw.
#[test]
fn an_unchanged_repaint_never_draws_the_bodies_again() {
    let _fonts = font_lock();
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        for samples in [1, 4] {
            let mut r = renderer(format, samples);
            let id = r
                .register_chart(base_config(), vec![series("s0", "a")])
                .unwrap();
            let view = r
                .create_chart_view(&Chart::new(base_config()), PANEL)
                .unwrap();
            let frame = prepare(&mut r, id, &view);
            let drawn = r.resident_display_serial(&view);
            assert!(drawn.is_some(), "{format:?} {samples}x: first frame draws");
            let first = painted_like_direct(&r, frame, samples);
            for repaint in 0..3 {
                let frame = prepare(&mut r, id, &view);
                assert_eq!(
                    r.resident_display_serial(&view),
                    drawn,
                    "{format:?} {samples}x: repaint {repaint} drew the bodies again"
                );
                assert_eq!(r.resident_display_serial, drawn.unwrap());
                assert_eq!(painted_like_direct(&r, frame, samples), first);
            }
        }
    }
}

// Every edit outside the data layer keeps the cached bodies: titles, legend,
// axis chrome, grid, an element's selection box, annotations and their
// visibility, and point picks (drawn live above the bodies). Each edit is
// visible, so the check is not vacuous, and each frame matches the direct
// draw of the edited state.
#[test]
fn chrome_annotation_selection_and_pick_edits_never_draw_the_bodies_again() {
    let _fonts = font_lock();
    let mut r = renderer(wgpu::TextureFormat::Rgba8Unorm, 1);
    let id = r
        .register_chart(base_config(), vec![series("s0", "a")])
        .unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(base_config()), PANEL)
        .unwrap();
    let frame = prepare(&mut r, id, &view);
    let drawn = r.resident_display_serial(&view).unwrap();
    let mut previous = painted_like_direct(&r, frame, 1);

    let picked = |index: usize, ring: Color| PickedPointsConfig {
        visible: true,
        refs: vec![PickedPointRef {
            source_id: None,
            series_id: "s0".into(),
            point_index: index,
        }],
        ring_color: ring,
        ..PickedPointsConfig::default()
    };
    let cases: Vec<(&str, Box<dyn Fn(&mut Config)>)> = vec![
        (
            "title text",
            Box::new(|c| c.chart_title.text = text("Edited")),
        ),
        ("legend moved", Box::new(|c| c.legend.offset_x -= 40.0)),
        ("legend hidden", Box::new(|c| c.legend.visible = false)),
        (
            "axis line colour",
            Box::new(|c| c.bottom_x.line_color = Color::new(0.9, 0.1, 0.1, 1.0)),
        ),
        ("major grid off", Box::new(|c| c.grid.show_major_x = false)),
        (
            "annotation added",
            Box::new(|c| {
                c.annotations
                    .items
                    .push(Annotation::new(AnnotationKind::Text {
                        anchor: Anchor::Ratio { x: 0.2, y: 0.2 },
                        text: text("note"),
                        box_style: None,
                    }))
            }),
        ),
        (
            "annotations hidden",
            Box::new(|c| c.annotations.visible = false),
        ),
        (
            "point picked",
            Box::new(move |c| c.picked_points = Some(picked(100, Color::new(1.0, 0.5, 0.0, 1.0)))),
        ),
        (
            "pick moved",
            Box::new(move |c| c.picked_points = Some(picked(40, Color::new(1.0, 0.5, 0.0, 1.0)))),
        ),
        (
            "pick ring colour",
            Box::new(move |c| c.picked_points = Some(picked(40, Color::new(0.0, 0.8, 0.2, 1.0)))),
        ),
    ];
    for (name, apply) in cases {
        edit(&mut r, id, &mut view, |c| apply(c));
        let frame = prepare(&mut r, id, &view);
        assert_eq!(
            r.resident_display_serial(&view),
            Some(drawn),
            "{name}: the bodies were drawn again"
        );
        let pixels = painted_like_direct(&r, frame, 1);
        assert_ne!(pixels, previous, "{name}: the edit is not visible");
        previous = pixels;
    }

    // An element's selection box lives in the decoration layer.
    let config = r.chart_config(id).unwrap().clone();
    assert!(
        r.selection_box(&Chart::new(config), "data_area").is_some(),
        "the data area has a selection box"
    );
    refresh(&mut r, id, &mut view, Some("data_area"));
    let frame = prepare(&mut r, id, &view);
    assert_eq!(
        r.resident_display_serial(&view),
        Some(drawn),
        "selection box: the bodies were drawn again"
    );
    assert_ne!(painted_like_direct(&r, frame, 1), previous);
}

// Every edit of the data layer draws the bodies exactly once, matches the
// direct draw, and the next repaint draws nothing.
#[test]
fn data_edits_draw_the_bodies_exactly_once() {
    let _fonts = font_lock();
    let mut r = renderer(wgpu::TextureFormat::Rgba8Unorm, 1);
    let id = r
        .register_chart(base_config(), vec![series("s0", "a")])
        .unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(base_config()), PANEL)
        .unwrap();
    let frame = prepare(&mut r, id, &view);
    let mut previous = painted_like_direct(&r, frame, 1);

    enum Edit {
        Config(fn(&mut Config)),
        Series(fn(&mut SeriesConfig)),
        Column,
    }
    let cases = [
        (
            "x range panned",
            Edit::Config(|c| {
                let mut chart = Chart::new(c.clone());
                chart.set_x_range(0.1, 1.1);
                *c = chart.config().clone();
            }),
        ),
        (
            "y range zoomed",
            Edit::Config(|c| {
                let mut chart = Chart::new(c.clone());
                chart.set_y_range(0.2, 0.8);
                *c = chart.config().clone();
            }),
        ),
        (
            "data area resized",
            Edit::Config(|c| c.left_y.out_margin = 60.0),
        ),
        (
            "line width",
            Edit::Series(|s| {
                if let DataRenderType::ScatterLine { line, .. } = &mut s.render_type {
                    line.line_width = 5.0;
                }
            }),
        ),
        ("column replaced", Edit::Column),
    ];
    for (name, change) in cases {
        let before = r.resident_display_serial;
        match change {
            Edit::Config(apply) => edit(&mut r, id, &mut view, apply),
            Edit::Series(apply) => {
                let mut list = r.chart_series(id).unwrap().to_vec();
                apply(&mut list[0]);
                r.set_chart_series(id, list).unwrap();
                refresh(&mut r, id, &mut view, None);
            }
            Edit::Column => {
                r.upsert_column("a", &wave(0.9, 1.0)).unwrap();
            }
        }
        let frame = prepare(&mut r, id, &view);
        assert_eq!(
            r.resident_display_serial,
            before + 1,
            "{name}: expected exactly one draw"
        );
        assert_eq!(r.resident_display_serial(&view), Some(before + 1));
        let pixels = painted_like_direct(&r, frame, 1);
        assert_ne!(pixels, previous, "{name}: the edit is not visible");
        previous = pixels;
        let frame = prepare(&mut r, id, &view);
        assert_eq!(
            r.resident_display_serial,
            before + 1,
            "{name}: the repaint after it drew again"
        );
        assert_eq!(painted_like_direct(&r, frame, 1), previous);
    }
}

// Each panel of a layered document caches its own bodies: an edit of one
// overlay's axis or series draws that overlay's bodies only, a chrome edit
// draws none, and the shared X range draws every layer.
#[test]
fn each_layer_keeps_its_own_bodies() {
    let _fonts = font_lock();
    let mut config = base_config();
    config.overlays.push(overlay(0.0, 60.0, "s1"));
    let mut second = overlay(-1.5, 1.5, "s2");
    second.y_axis.line_offset = 44.0;
    config.overlays.push(second);
    let mut r = renderer(wgpu::TextureFormat::Rgba8Unorm, 1);
    let id = r
        .register_chart(
            config.clone(),
            vec![series("s0", "a"), series("s1", "b"), series("s2", "c")],
        )
        .unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(config.clone()), PANEL)
        .unwrap();
    let frame = prepare(&mut r, id, &view);
    assert_eq!(r.resident_display_count(), 3, "one entry per panel");
    painted_like_direct(&r, frame, 1);
    let serials = |r: &Renderer, view: &ChartView| {
        [
            r.resident_display_serial(view).unwrap(),
            r.resident_display_serial(&view.overlay_views[0]).unwrap(),
            r.resident_display_serial(&view.overlay_views[1]).unwrap(),
        ]
    };
    let mut held = serials(&r, &view);

    enum Edit {
        Config(fn(&mut Config)),
        Series(usize),
    }
    let cases = [
        (
            "overlay 0 range",
            Edit::Config(|c| {
                c.overlays[0].y_axis.min = -10.0;
                c.overlays[0].y_axis.max = 70.0;
            }),
            [false, true, false],
        ),
        ("overlay 1 series", Edit::Series(2), [false, false, true]),
        ("base series", Edit::Series(0), [true, false, false]),
        (
            "base title",
            Edit::Config(|c| c.chart_title.text = text("Layers")),
            [false, false, false],
        ),
        (
            "active layer",
            Edit::Config(|c| c.active_layer = LayerRef::Overlay(1)),
            [false, false, false],
        ),
        (
            "shared x range",
            Edit::Config(|c| {
                let mut chart = Chart::new(c.clone());
                chart.set_x_range(0.2, 0.9);
                *c = chart.config().clone();
            }),
            [true, true, true],
        ),
    ];
    for (name, change, expected) in cases {
        match change {
            Edit::Config(apply) => edit(&mut r, id, &mut view, apply),
            Edit::Series(index) => {
                let mut list = r.chart_series(id).unwrap().to_vec();
                if let DataRenderType::ScatterLine { line, .. } = &mut list[index].render_type {
                    line.line_width += 2.0;
                }
                r.set_chart_series(id, list).unwrap();
                refresh(&mut r, id, &mut view, None);
            }
        }
        let frame = prepare(&mut r, id, &view);
        let now = serials(&r, &view);
        let drawn = [now[0] != held[0], now[1] != held[1], now[2] != held[2]];
        assert_eq!(drawn, expected, "{name}: panels drawn again");
        painted_like_direct(&r, frame, 1);
        held = now;
    }
}

// The cache follows the panel, not the target: a target that clips the panel
// draws the bodies directly, pixel for pixel; a styled panel has no cache;
// a token painted after its bodies were drawn again is stale; dropping the
// view retires its entry.
#[test]
fn clipped_styled_stale_and_dropped_panels() {
    let _fonts = font_lock();
    let mut r = renderer(wgpu::TextureFormat::Rgba8Unorm, 1);
    let id = r
        .register_chart(base_config(), vec![series("s0", "a")])
        .unwrap();
    let view = r
        .create_chart_view(&Chart::new(base_config()), PANEL)
        .unwrap();

    // Clipped by a smaller target: the cached frame paints what the direct
    // draw paints, exactly.
    {
        let mut frame = prepare(&mut r, id, &view);
        let clipped = try_paint(&r, &frame, 1, (200, 150)).unwrap();
        for item in &mut frame.items {
            item.resident_display = None;
        }
        assert_eq!(clipped, try_paint(&r, &frame, 1, (200, 150)).unwrap());
    }

    // A frame prepared before the bodies were drawn again cannot paint.
    {
        let stale = prepare(&mut r, id, &view);
        let mut moved = r.chart_config(id).unwrap().clone();
        moved.bottom_x.min = 0.3;
        r.set_chart_config(id, moved).unwrap();
        let fresh = prepare(&mut r, id, &view);
        assert!(matches!(
            try_paint(&r, &stale, 1, (PANEL.width, PANEL.height)),
            Err(FiggyError::StalePreparedFrame { .. })
        ));
        painted_like_direct(&r, fresh, 1);
    }

    // The view goes away with its entry once no token holds it: the frame
    // boundary retires the texture without waiting for another prepare.
    let charged = r
        .gpu_memory_usage()
        .live_bytes_of(crate::gpu_memory::GpuResourceKind::PanelTexture);
    drop(view);
    r.end_gpu_frame();
    assert_eq!(r.resident_display_count(), 0);
    assert!(
        r.gpu_memory_usage()
            .live_bytes_of(crate::gpu_memory::GpuResourceKind::PanelTexture)
            < charged,
        "the bodies' texture is no longer charged as live"
    );
    let other = r
        .create_chart_view(&Chart::new(base_config()), PANEL)
        .unwrap();
    prepare(&mut r, id, &other);
    assert_eq!(r.resident_display_count(), 1);

    // A styled panel draws directly and holds no entry.
    let mut sketch = r.chart_config(id).unwrap().clone();
    sketch.draw_style = crate::config::DrawStyle::Sketch(SketchOptions::default());
    let styled = r
        .register_chart(sketch.clone(), vec![series("s9", "a")])
        .unwrap();
    let styled_view = r.create_chart_view(&Chart::new(sketch), PANEL).unwrap();
    let frame = prepare(&mut r, styled, &styled_view);
    assert!(
        frame
            .items
            .iter()
            .all(|item| item.resident_display.is_none())
    );
    assert_eq!(r.resident_display_serial(&styled_view), None);
}

/// Deterministic xorshift for the randomized sequences.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A value in `[lo, hi)` on a 1/64 grid, so ranges stay readable.
    fn step(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.below(64) as f64 / 64.0
    }
}

/// Everything a chart is drawn from, kept by the test so a fresh renderer can
/// draw the same state from nothing.
struct Scene {
    columns: Vec<(&'static str, Vec<f32>)>,
    config: Config,
    series: Vec<SeriesConfig>,
    /// The element id selected on the document's active layer.
    selection: Option<&'static str>,
}

impl Scene {
    fn new() -> Self {
        let x = (0..POINTS)
            .map(|i| i as f32 / (POINTS - 1) as f32)
            .collect();
        let mut config = base_config();
        config.overlays.push(overlay(0.0, 60.0, "s1"));
        Self {
            columns: vec![
                ("x", x),
                ("a", wave(0.0, 1.0).data),
                ("b", wave(1.3, 60.0).data),
                ("z", wave(2.1, 1.0).data),
            ],
            config,
            series: vec![series("s0", "a"), series("s1", "b")],
            selection: None,
        }
    }

    fn upload(&self, r: &mut Renderer) {
        for (id, data) in &self.columns {
            r.upsert_column(*id, &column_of(data)).unwrap();
        }
    }
}

fn column_of(data: &[f32]) -> crate::Column<f32> {
    column(data.to_vec())
}

/// A renderer that has seen nothing but `scene`: the oracle.
fn fresh_pixels(scene: &Scene, format: wgpu::TextureFormat, samples: u32) -> Vec<u8> {
    let mut r = renderer(format, samples);
    scene.upload(&mut r);
    let id = r
        .register_chart(scene.config.clone(), scene.series.clone())
        .unwrap();
    let mut view = r
        .create_chart_view(&Chart::new(scene.config.clone()), PANEL)
        .unwrap();
    r.refresh_axis_with_selection(
        &mut view,
        &Chart::new(scene.config.clone()),
        PANEL,
        scene.selection,
    )
    .unwrap();
    let frame = prepare(&mut r, id, &view);
    try_paint(&r, &frame, samples, (PANEL.width, PANEL.height)).unwrap()
}

/// One random edit of `scene`. Returns its name, which panels (base,
/// overlay) must draw their bodies again (every other panel must not), and
/// the column it replaced, if any.
fn random_edit(rng: &mut Rng, scene: &mut Scene) -> (&'static str, [bool; 2], Option<usize>) {
    let (name, expected) = random_scene_edit(rng, scene);
    let column = match name {
        "base column" => Some(1),
        "overlay column" => Some(2),
        "unread column" => Some(3),
        _ => None,
    };
    (name, expected, column)
}

fn random_scene_edit(rng: &mut Rng, scene: &mut Scene) -> (&'static str, [bool; 2]) {
    let c = &mut scene.config;
    let ring = |rng: &mut Rng| Color::new(rng.step(0.0, 1.0) as f32, 0.4, 0.2, 1.0);
    match rng.below(18) {
        0 => {
            c.chart_title.text = text(&format!("Title {}", rng.next() % 1000));
            ("title", [false, false])
        }
        1 => {
            c.legend.offset_x = -(rng.step(10.0, 120.0) as f32);
            ("legend offset", [false, false])
        }
        2 => {
            c.bottom_x.line_color = ring(rng);
            ("axis colour", [false, false])
        }
        3 => {
            c.grid.show_major_y = !c.grid.show_major_y;
            ("grid", [false, false])
        }
        4 => {
            if c.annotations.items.is_empty() || rng.below(2) == 0 {
                c.annotations
                    .items
                    .push(Annotation::new(AnnotationKind::Text {
                        anchor: Anchor::Ratio {
                            x: rng.step(0.1, 0.8) as f32,
                            y: rng.step(0.1, 0.8) as f32,
                        },
                        text: text("note"),
                        box_style: None,
                    }));
            } else {
                c.annotations.items.pop();
            }
            ("annotation", [false, false])
        }
        5 => {
            c.annotations.visible = !c.annotations.visible;
            ("annotation visibility", [false, false])
        }
        6 => {
            c.picked_points = (rng.below(3) != 0).then(|| PickedPointsConfig {
                visible: true,
                refs: vec![PickedPointRef {
                    source_id: None,
                    series_id: if rng.below(2) == 0 { "s0" } else { "s1" }.into(),
                    point_index: rng.below(POINTS as u64) as usize,
                }],
                ring_color: ring(rng),
                ..PickedPointsConfig::default()
            });
            ("pick", [false, false])
        }
        7 => {
            // An element every active layer registers, resolved on the
            // active layer the scene has at the refresh.
            const SELECTABLE: [&str; 5] = [
                "data_area",
                "legend",
                "chart_title",
                "axis_bottom",
                "tick_labels_bottom",
            ];
            scene.selection = match scene.selection {
                None => Some(SELECTABLE[rng.below(SELECTABLE.len() as u64) as usize]),
                Some(_) => None,
            };
            ("selection box", [false, false])
        }
        8 => {
            c.active_layer = match c.active_layer {
                LayerRef::Base => LayerRef::Overlay(0),
                _ => LayerRef::Base,
            };
            ("active layer", [false, false])
        }
        9 => {
            let min = c.bottom_x.min + rng.step(0.02, 0.2);
            let mut chart = Chart::new(c.clone());
            chart.set_x_range(min, min + 1.0);
            *c = chart.config().clone();
            ("x range (shared)", [true, true])
        }
        10 => {
            let min = c.left_y.min + rng.step(0.02, 0.2);
            let mut chart = Chart::new(c.clone());
            chart.set_y_range(min, min + 1.0);
            *c = chart.config().clone();
            ("base y range", [true, false])
        }
        11 => {
            let axis = &mut c.overlays[0].y_axis;
            axis.min += rng.step(1.0, 10.0);
            axis.max = axis.min + 60.0;
            ("overlay y range", [false, true])
        }
        12 => {
            if let DataRenderType::ScatterLine { line, .. } = &mut scene.series[0].render_type {
                line.line_width = if line.line_width > 3.0 { 1.5 } else { 4.0 };
            }
            ("base series", [true, false])
        }
        13 => {
            if let DataRenderType::ScatterLine { scatter, .. } = &mut scene.series[1].render_type {
                scatter.point_size = if scatter.point_size > 7.0 { 5.0 } else { 9.0 };
            }
            ("overlay series", [false, true])
        }
        14 => {
            scene.columns[1].1 = wave(rng.step(0.0, 6.0) as f32, 1.0).data;
            ("base column", [true, false])
        }
        15 => {
            scene.columns[2].1 = wave(rng.step(0.0, 6.0) as f32, 60.0).data;
            ("overlay column", [false, true])
        }
        16 => {
            scene.columns[3].1 = wave(rng.step(0.0, 6.0) as f32, 1.0).data;
            ("unread column", [false, false])
        }
        _ => {
            c.left_y.out_margin = if c.left_y.out_margin > 40.0 {
                30.0
            } else {
                56.0
            };
            ("plot area (shared)", [true, true])
        }
    }
}

// Random edit sequences over a document with an overlay layer. After every
// edit each panel draws its bodies again exactly when the edit reaches its
// data layer, and the frame is bit-identical to a renderer that draws the
// final state from nothing. An unchanged repaint follows every few edits.
#[test]
fn random_edit_sequences_match_a_render_from_nothing_bit_for_bit() {
    let _fonts = font_lock();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    for (seed, samples, steps) in [
        (0x9E37_79B9_7F4A_7C15u64, 1, 40),
        (0xD1B5_4A32, 1, 40),
        (0x5EED, 4, 24),
    ] {
        let mut rng = Rng(seed);
        let mut scene = Scene::new();
        let mut r = renderer(format, samples);
        scene.upload(&mut r);
        let id = r
            .register_chart(scene.config.clone(), scene.series.clone())
            .unwrap();
        let mut view = r
            .create_chart_view(&Chart::new(scene.config.clone()), PANEL)
            .unwrap();
        prepare(&mut r, id, &view);
        let serials = |r: &Renderer, view: &ChartView| {
            [
                r.resident_display_serial(view).unwrap(),
                r.resident_display_serial(&view.overlay_views[0]).unwrap(),
            ]
        };
        let mut held = serials(&r, &view);
        for step in 0..steps {
            let (name, expected, column) = random_edit(&mut rng, &mut scene);
            let context = format!("seed {seed:#x} {samples}x step {step} ({name})");
            if let Some(index) = column {
                let (id, data) = &scene.columns[index];
                r.upsert_column(*id, &column_of(data)).unwrap();
            }
            r.set_chart_series(id, scene.series.clone()).unwrap();
            r.set_chart_config(id, scene.config.clone()).unwrap();
            r.refresh_axis_with_selection(
                &mut view,
                &Chart::new(scene.config.clone()),
                PANEL,
                scene.selection,
            )
            .unwrap();
            let frame = prepare(&mut r, id, &view);
            let now = serials(&r, &view);
            let drawn = [now[0] != held[0], now[1] != held[1]];
            assert_eq!(drawn, expected, "{context}: panels drawn again");
            held = now;
            let pixels = try_paint(&r, &frame, samples, (PANEL.width, PANEL.height)).unwrap();
            drop(frame);
            assert!(
                pixels == fresh_pixels(&scene, format, samples),
                "{context}: differs from a render from nothing"
            );
            if step % 5 == 4 {
                let frame = prepare(&mut r, id, &view);
                assert_eq!(serials(&r, &view), held, "{context}: repaint drew again");
                assert!(
                    try_paint(&r, &frame, samples, (PANEL.width, PANEL.height)).unwrap() == pixels,
                    "{context}: repaint changed the frame"
                );
            }
        }
    }
}

#[cfg(feature = "serde")]
#[path = "resident_display_mutation_tests.rs"]
mod mutation;
