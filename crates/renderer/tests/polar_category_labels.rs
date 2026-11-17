//! A polar chart's category labels (#309): each is its own element, moved by
//! itself through its category's `label_offset` (in its spoke's frame), and
//! a selected label shows a faint dashed ring through the labels' start that
//! moves them all along their spokes. Covered here: the drawn label at 1x
//! and 2x, the hit map and the drags, the ring's drawing and grab, a polar
//! subplot, the PNG and SVG exports, the serde round trip and a released
//! radar document with a moved spoke label.
use test_gpu::BlockOn as _;

#[path = "support/svg_parts.rs"]
mod svg_parts;
#[path = "support/gpu.rs"]
mod test_gpu;

use renderer::axis_render::{
    AxisLayerKind, try_raster_chart_layer_to_rgba_with_selection_at, try_raster_chart_to_rgba,
};
use renderer::config::{Category, TickVisibility};
use renderer::layout::{ChartArea, DrawnPart, Element, NudgeResult, Rect, RectF};
use renderer::polar::{PolarOptions, screen_to_spoke_offset, spoke_offset_to_screen};
use renderer::sink::Parts;
use renderer::{
    Chart, Config, CpuTextMeasure, HitMap, Renderer, RendererDevice, SelectionGeometry,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 420;
const N: usize = 5;

/// A polar chart over five categories with only its angle labels drawn:
/// no axis strokes, ticks, titles, grid or legend.
fn config() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    cfg.chart_title.visible = false;
    cfg.legend.visible = false;
    for axis in [&mut cfg.bottom_x, &mut cfg.left_y] {
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    cfg.left_y.label_style.visible = false;
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    cfg.bottom_x.categories = Some(
        (0..N)
            .map(|i| Category::new(format!("c{i}"), format!("Label {i}")))
            .collect(),
    );
    cfg.set_polar(Some(PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    }));
    cfg
}

fn measure(cfg: &Config) -> CpuTextMeasure {
    CpuTextMeasure::for_style(&cfg.draw_style)
}

/// The drawn box of category `i`'s label.
fn label_box(cfg: &Config, i: usize) -> RectF {
    let mut rect = None;
    assert!(renderer::chrome::emit_part_of(
        cfg,
        &Element::CategoryLabel(i),
        &measure(cfg),
        &mut Parts(&mut |part| {
            if let DrawnPart::Text(r) = part {
                rect = Some(r);
            }
        }),
        1.0
    ));
    rect.unwrap_or_else(|| panic!("label {i} is not drawn"))
}

fn centre(r: RectF) -> (f32, f32) {
    (r.x + r.width * 0.5, r.y + r.height * 0.5)
}

fn hit(cfg: &Config, x: f32, y: f32) -> Option<String> {
    let map = HitMap::for_active_layer(cfg);
    map.hit_test(cfg, &measure(cfg), x, y)
        .and_then(|id| map.get(id))
        .map(|el| el.element_id())
}

/// The screen direction of category `i`'s spoke.
fn spoke(cfg: &Config, i: usize) -> [f32; 2] {
    let angle = cfg.polar_angle_map().unwrap().screen_angle(i as f64);
    [angle.cos() as f32, -(angle.sin() as f32)]
}

#[test]
fn the_spoke_frame_round_trips_a_screen_offset() {
    for i in 0..8 {
        let a = i as f32 * 0.7;
        let dir = [a.cos(), a.sin()];
        let offset = screen_to_spoke_offset(dir, [7.0, -3.0]);
        let back = spoke_offset_to_screen(dir, offset);
        assert!(
            (back[0] - 7.0).abs() < 1e-4 && (back[1] + 3.0).abs() < 1e-4,
            "{back:?}"
        );
    }
    // Along is outward, across a quarter turn counter-clockwise on screen.
    assert_eq!(spoke_offset_to_screen([1.0, 0.0], [2.0, 0.0]), [2.0, 0.0]);
    assert_eq!(spoke_offset_to_screen([1.0, 0.0], [0.0, 2.0]), [0.0, -2.0]);
}

/// A category's own offset moves only its label, by the screen amount its
/// spoke frame gives, on the raster at 1x and 2x.
#[test]
fn a_label_offset_moves_only_that_label() {
    let base = config();
    let mut moved = base.clone();
    moved.bottom_x.categories.as_mut().unwrap()[2].label_offset = [12.0, -8.0];
    let expected = spoke_offset_to_screen(spoke(&base, 2), [12.0, -8.0]);
    for i in 0..N {
        let (a, b) = (centre(label_box(&base, i)), centre(label_box(&moved, i)));
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        if i == 2 {
            assert!(
                (dx - expected[0]).abs() < 0.01 && (dy - expected[1]).abs() < 0.01,
                "label 2 moved by ({dx}, {dy}), not {expected:?}"
            );
        } else {
            assert_eq!((dx, dy), (0.0, 0.0), "label {i}");
        }
    }
    for scale in [1.0f32, 2.0] {
        let raster = |cfg: &Config| try_raster_chart_to_rgba(&cfg.scaled(scale)).unwrap();
        let (a, b) = (raster(&base), raster(&moved));
        let ink = |img: &[u8], r: RectF| {
            let mut n = 0;
            let w = (WIDTH as f32 * scale) as usize;
            for y in (r.y * scale) as usize..((r.y + r.height) * scale) as usize {
                for x in (r.x * scale) as usize..((r.x + r.width) * scale) as usize {
                    if img
                        .get((y * w + x) * 4 + 3)
                        .is_some_and(|&alpha| alpha > 100)
                    {
                        n += 1;
                    }
                }
            }
            n
        };
        let before = label_box(&base, 2);
        let after = label_box(&moved, 2);
        assert!(ink(&a, before) > 0, "{scale}x: label 2 drawn");
        assert!(ink(&b, after) > 0, "{scale}x: moved label 2 drawn");
        assert!(
            ink(&b, before) < ink(&a, before),
            "{scale}x: the old place keeps the moved label's ink"
        );
        // The other labels draw the same pixels.
        for i in [0, 4] {
            let r = label_box(&base, i);
            assert_eq!(ink(&a, r), ink(&b, r), "{scale}x: label {i}");
        }
    }
}

/// A click on a label selects that label; dragging it writes only its own
/// offset; the shared radial offset and the other labels stay.
#[test]
fn a_label_is_hit_and_dragged_by_itself() {
    let mut cfg = config();
    for i in 0..N {
        let (x, y) = centre(label_box(&cfg, i));
        assert_eq!(
            hit(&cfg, x, y).as_deref(),
            Some(format!("category_label_{i}").as_str())
        );
    }
    let map = HitMap::for_active_layer(&cfg);
    let (x, y) = centre(label_box(&cfg, 1));
    let id = map.hit_test(&cfg, &measure(&cfg), x, y).unwrap();
    let drag = map.get(id).unwrap().as_draggable().unwrap();
    let m = measure(&cfg);
    assert_eq!(
        drag.drag_from(&mut cfg, &m, [x, y], 10.0, 5.0),
        NudgeResult::Moved
    );
    let categories = cfg.bottom_x.categories.as_deref().unwrap();
    let screen = spoke_offset_to_screen(spoke(&cfg, 1), categories[1].label_offset);
    assert!(
        (screen[0] - 10.0).abs() < 1e-3 && (screen[1] - 5.0).abs() < 1e-3,
        "{screen:?}"
    );
    for (i, c) in categories.iter().enumerate() {
        if i != 1 {
            assert_eq!(c.label_offset, [0.0, 0.0], "category {i}");
        }
    }
    assert_eq!(cfg.bottom_x.label_style.label_offset_y, 0.0);
    // The label is hit where it now is.
    let (x, y) = centre(label_box(&cfg, 1));
    assert_eq!(hit(&cfg, x, y).as_deref(), Some("category_label_1"));
    // A drag that would take the label off the chart is refused whole.
    let before = cfg.clone();
    assert!(matches!(
        drag.drag_from(&mut cfg, &m, [x, y], 5000.0, 0.0),
        NudgeResult::Rejected(_)
    ));
    assert_eq!(cfg, before);
}

/// A selected label shows the dashed ring through the labels' start; a
/// press on the ring grabs it and the drag moves every label along its
/// spoke through the shared offset, keeping each label's own offset.
#[test]
fn the_ring_is_drawn_with_the_selection_and_moves_every_label() {
    let mut cfg = config();
    cfg.bottom_x.categories.as_mut().unwrap()[3].label_offset = [5.0, 0.0];
    let map = HitMap::for_active_layer(&cfg);
    let (x, y) = centre(label_box(&cfg, 0));
    let id = map.hit_test(&cfg, &measure(&cfg), x, y).unwrap();
    let element = map.get(id).unwrap();
    let selection = element.selection_box(&cfg, &measure(&cfg)).unwrap();
    let (center, radius) = selection.ring().expect("a selected label shows the ring");
    let geometry = cfg.polar_geometry().unwrap();
    assert_eq!(center, geometry.center);
    assert!(radius > geometry.radius, "{radius} {}", geometry.radius);
    // Every label starts outside the ring.
    for i in 0..N {
        let r = label_box(&cfg, i);
        let (cx, cy) = centre(r);
        assert!((cx - center[0]).hypot(cy - center[1]) > radius, "label {i}");
    }
    // The ring is drawn faint and dashed: ink along it, with gaps, at 1x
    // and 2x. At 2x the selection is laid out on the scaled config, as the
    // renderer draws it on a 2x surface.
    for scale in [1.0f32, 2.0] {
        let scaled = cfg.scaled(scale);
        let map = HitMap::for_active_layer(&scaled);
        let (x, y) = centre(label_box(&scaled, 0));
        let id = map.hit_test(&scaled, &measure(&scaled), x, y).unwrap();
        let selection = map
            .get(id)
            .unwrap()
            .selection_box(&scaled, &measure(&scaled))
            .unwrap();
        let (center, radius) = selection.ring().unwrap();
        let raster = |boxes: &[renderer::SelectionBox]| {
            try_raster_chart_layer_to_rgba_with_selection_at(
                &scaled,
                AxisLayerKind::All,
                boxes,
                scale,
            )
            .unwrap()
        };
        let (plain, selected) = (raster(&[]), raster(std::slice::from_ref(&selection)));
        let w = (WIDTH as f32 * scale) as usize;
        let (mut inked, mut gaps) = (0, 0);
        for step in 0..360 {
            let a = (step as f32).to_radians();
            let (px, py) = (
                (center[0] + radius * a.cos()) as usize,
                (center[1] + radius * a.sin()) as usize,
            );
            let at = (py * w + px) * 4 + 3;
            if selected[at] > plain[at] {
                inked += 1;
            } else {
                gaps += 1;
            }
        }
        assert!(inked > 90, "{scale}x: ring ink at {inked} of 360");
        assert!(gaps > 30, "{scale}x: the ring is dashed ({gaps} gaps)");
    }
    // A press on the ring, away from any label, grabs it.
    let resize = element.as_resizable().unwrap();
    let probe = |angle: f32| {
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    };
    let on_ring = (0..72)
        .map(|k| probe(k as f32 * std::f32::consts::TAU / 72.0))
        .find(|p| (0..N).all(|i| !label_box(&cfg, i).expanded(4.0).contains(p[0], p[1])))
        .expect("a point of the ring no label covers");
    let m = measure(&cfg);
    let handle = resize
        .hit_resize_handle(&cfg, &m, on_ring[0], on_ring[1])
        .expect("the ring is grabbed");
    assert!(
        resize
            .hit_resize_handle(&cfg, &m, center[0], center[1])
            .is_none(),
        "the centre is not the ring"
    );
    // Where the ring passes under a label, a press selects the label: it
    // does not grab the ring.
    for i in 0..N {
        let r = label_box(&cfg, i);
        let dir = spoke(&cfg, i);
        let under = [center[0] + dir[0] * radius, center[1] + dir[1] * radius];
        if r.expanded(1.0).contains(under[0], under[1]) {
            assert!(
                resize
                    .hit_resize_handle(&cfg, &m, under[0], under[1])
                    .is_none(),
                "label {i}: the ring under it was grabbed"
            );
        }
    }
    // Dragging the ring outward along the pointer's spoke moves every label
    // along its own spoke by the same amount.
    let out = [on_ring[0] - center[0], on_ring[1] - center[1]];
    let len = out[0].hypot(out[1]);
    let (dx, dy) = (out[0] / len * 6.0, out[1] / len * 6.0);
    let before: Vec<(f32, f32)> = (0..N).map(|i| centre(label_box(&cfg, i))).collect();
    assert_eq!(
        resize.resize_from(&mut cfg, &m, handle, on_ring, dx, dy),
        NudgeResult::Moved
    );
    assert!((cfg.bottom_x.label_style.label_offset_y - 6.0).abs() < 1e-3);
    assert_eq!(
        cfg.bottom_x.categories.as_deref().unwrap()[3].label_offset,
        [5.0, 0.0]
    );
    for (i, b) in before.iter().enumerate() {
        let a = centre(label_box(&cfg, i));
        let along = (a.0 - b.0) * spoke(&cfg, i)[0] + (a.1 - b.1) * spoke(&cfg, i)[1];
        assert!(
            (along - 6.0).abs() < 0.01,
            "label {i} moved {along} along its spoke"
        );
    }
    let (_, moved_radius) = element
        .selection_box(&cfg, &measure(&cfg))
        .unwrap()
        .ring()
        .unwrap();
    assert!((moved_radius - radius - 6.0).abs() < 1e-3);
}

/// In a polar subplot the labels are their own elements on the base layer
/// too, and the ring grabs inside the subplot.
#[test]
fn a_polar_subplot_has_its_own_category_labels_and_ring() {
    let mut parent = renderer::default::default_config();
    parent.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH * 2,
        height: HEIGHT,
    });
    let mut inner = config();
    inner.chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.subplots.push(renderer::Subplot {
        config: inner,
        series: vec![],
        link_x: None,
        link_y: None,
        marker: None,
        text_scale: Default::default(),
    });
    let chart = parent.subplot_config(0).unwrap();
    let (x, y) = centre(label_box(&chart, 2));
    assert_eq!(
        hit(&parent, x, y).as_deref(),
        Some("subplot_0:category_label_2")
    );
    let map = HitMap::for_active_layer(&parent);
    let id = map.hit_test(&parent, &measure(&parent), x, y).unwrap();
    let element = map.get(id).unwrap();
    let (center, radius) = element
        .selection_box(&parent, &measure(&parent))
        .unwrap()
        .ring()
        .unwrap();
    assert_eq!(center, chart.polar_geometry().unwrap().center);
    let m = measure(&parent);
    let resize = element.as_resizable().unwrap();
    let on_ring = (0..72)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 72.0;
            [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
        })
        .find(|p| (0..N).all(|i| !label_box(&chart, i).expanded(4.0).contains(p[0], p[1])))
        .unwrap();
    assert!(
        resize
            .hit_resize_handle(&parent, &m, on_ring[0], on_ring[1])
            .is_some()
    );
    // Dragging the label writes the subplot's category.
    let drag = element.as_draggable().unwrap();
    assert_eq!(
        drag.drag_from(&mut parent, &m, [x, y], 0.0, 9.0),
        NudgeResult::Moved
    );
    let moved = &parent.subplots[0]
        .config
        .bottom_x
        .categories
        .as_deref()
        .unwrap()[2];
    assert_ne!(moved.label_offset, [0.0, 0.0]);
    assert_eq!(
        parent.polar_category_label_counts().collect::<Vec<_>>(),
        [0, N]
    );
}

#[cfg(feature = "serde")]
#[test]
fn a_label_offset_round_trips_and_zero_is_omitted() {
    let mut cfg = config();
    let plain = serde_json::to_string(&cfg).unwrap();
    assert!(
        !plain.contains("label_offset\":["),
        "a zero offset is omitted"
    );
    cfg.bottom_x.categories.as_mut().unwrap()[4].label_offset = [1.5, -2.25];
    let json = serde_json::to_string(&cfg).unwrap();
    let back: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cfg);
    let old: Config = serde_json::from_str(&plain).unwrap();
    assert_eq!(serde_json::to_string(&old).unwrap(), plain);
}

/// The released radar document with a moved spoke label (model 0.12.1)
/// keeps that label's place relative to its unmoved position.
#[cfg(feature = "serde")]
#[test]
#[allow(deprecated)]
fn a_released_radar_document_keeps_its_moved_spoke_label() {
    let json = include_str!("fixtures/released/model-0.12.1-radar-edited.json");
    let document: renderer::RadarChart = serde_json::from_str(json).unwrap();
    let moved = document
        .decorations
        .offsets
        .iter()
        .find_map(|o| match &o.element {
            renderer::DecorationId::SpokeLabel { category_id } => {
                Some((category_id.clone(), o.offset))
            }
            _ => None,
        })
        .expect("the fixture moves a spoke label");
    let (config, _, _) = Config::from_radar(&document);
    let categories = config.bottom_x.categories.as_deref().unwrap();
    let index = categories.iter().position(|c| c.id == moved.0).unwrap();
    let screen = spoke_offset_to_screen(spoke(&config, index), categories[index].label_offset);
    assert!(
        (screen[0] - moved.1[0]).abs() < 1e-3 && (screen[1] - moved.1[1]).abs() < 1e-3,
        "{screen:?} {:?}",
        moved.1
    );
}

fn renderer() -> Renderer {
    let (device, queue) = test_gpu::device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        1024 * 1024,
    )
    .expect("renderer init")
}

/// The PNG export at 1x and 2x and the SVG export draw the moved label
/// where the walk places it.
#[test]
fn the_exports_draw_the_moved_label() {
    let mut r = renderer();
    let base = Chart::new(config());
    let mut moved = Chart::new(config());
    moved.config_mut().bottom_x.categories.as_mut().unwrap()[0].label_offset = [0.0, 20.0];
    for scale in [1.0f32, 2.0] {
        let export = |r: &mut Renderer, chart: &Chart| {
            r.export_rgba(
                renderer::ExportSource::chart(chart, &[]),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap()
        };
        let (a, b) = (export(&mut r, &base), export(&mut r, &moved));
        let box_at = label_box(moved.config(), 0);
        let ink = |img: &renderer::RasterImage, rect: RectF| {
            let mut n = 0;
            for y in (rect.y * scale) as u32..((rect.y + rect.height) * scale) as u32 {
                for x in (rect.x * scale) as u32..((rect.x + rect.width) * scale) as u32 {
                    let at = ((y * img.width + x) * 4 + 3) as usize;
                    if img.rgba.get(at).is_some_and(|&alpha| alpha > 100) {
                        n += 1;
                    }
                }
            }
            n
        };
        assert!(ink(&b, box_at) > 0, "{scale}x: the moved label is drawn");
        assert!(ink(&a, box_at) < ink(&b, box_at), "{scale}x");
    }
    let svg = |r: &mut Renderer, chart: &Chart| {
        let svg = r
            .export_svg(
                renderer::ExportSource::chart(chart, &[]),
                renderer::ExportOptions::document(1.0),
            )
            .block_on()
            .unwrap();
        let file = svg_parts::read(&svg);
        file.parts
            .iter()
            .filter_map(|p| match p {
                svg_parts::ReadPart::Text(t) => Some([t[4], t[5]]),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (svg(&mut r, &base), svg(&mut r, &moved));
    assert_eq!(a.len(), b.len());
    let shifted: Vec<_> = a
        .iter()
        .zip(&b)
        .filter(|(p, q)| (p[0] - q[0]).abs() > 0.01 || (p[1] - q[1]).abs() > 0.01)
        .collect();
    assert_eq!(shifted.len(), 1, "exactly one label moves in the SVG");
    let (p, q) = shifted[0];
    let expected = spoke_offset_to_screen(spoke(base.config(), 0), [0.0, 20.0]);
    assert!(
        (q[0] - p[0] - expected[0]).abs() < 0.05 && (q[1] - p[1] - expected[1]).abs() < 0.05,
        "{p:?} -> {q:?}, expected {expected:?}"
    );
}

/// A subplot whose angle axis is linked to its parent shares the categories
/// but keeps its own label offsets (#309): dragging one chart's label moves
/// neither the other chart's label nor makes the edit a range edit.
#[test]
fn linked_polar_charts_keep_their_own_label_offsets() {
    let mut parent = config();
    parent.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH * 2,
        height: HEIGHT,
    });
    let mut inner = config();
    inner.chart_area = ChartArea(Rect {
        x: WIDTH,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.subplots.push(renderer::Subplot {
        config: inner,
        series: vec![],
        link_x: Some(renderer::CellRef::Parent),
        link_y: None,
        marker: None,
        text_scale: Default::default(),
    });
    let m = measure(&parent);
    // Drag the subplot's label 1.
    let chart = parent.subplot_config(0).unwrap();
    let (x, y) = centre(label_box(&chart, 1));
    let map = HitMap::for_active_layer(&parent);
    let id = map.hit_test(&parent, &m, x, y).unwrap();
    assert_eq!(
        map.get(id).unwrap().element_id(),
        "subplot_0:category_label_1"
    );
    let drag = map.get(id).unwrap().as_draggable().unwrap();
    assert_eq!(
        drag.drag_from(&mut parent, &m, [x, y], 0.0, 7.0),
        NudgeResult::Moved
    );
    let own = |c: &Config| c.bottom_x.categories.as_deref().unwrap()[1].label_offset;
    let moved = parent.subplot_config(0).unwrap();
    assert_ne!(own(&moved), [0.0, 0.0], "the subplot's label moved");
    assert_eq!(own(&parent), [0.0, 0.0], "the parent's label stays");
    // Drag the parent's label 3: the subplot keeps its own offsets.
    let (x, y) = centre(label_box(&parent, 3));
    let map = HitMap::for_active_layer(&parent);
    let id = map.hit_test(&parent, &m, x, y).unwrap();
    assert_eq!(map.get(id).unwrap().element_id(), "category_label_3");
    let drag = map.get(id).unwrap().as_draggable().unwrap();
    assert_eq!(
        drag.drag_from(&mut parent, &m, [x, y], 7.0, 0.0),
        NudgeResult::Moved
    );
    let sub = parent.subplot_config(0).unwrap();
    assert_eq!(
        sub.bottom_x.categories.as_deref().unwrap()[3].label_offset,
        [0.0, 0.0]
    );
    assert_eq!(own(&sub), own(&moved), "the subplot's own offset is kept");
    // A label edit is not a range edit: the categories stay linked.
    let ids = |c: &Config| -> Vec<String> {
        c.bottom_x
            .categories
            .as_deref()
            .unwrap()
            .iter()
            .map(|c| c.id.clone())
            .collect()
    };
    assert_eq!(ids(&sub), ids(&parent));
}
