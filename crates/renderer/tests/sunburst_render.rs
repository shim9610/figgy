//! A sunburst (`RadialSlice::children`, #140) through the public radial
//! API: the rings' geometry, the pick, the selection, the hover, the labels,
//! the rules at `prepare` and the eight-ring chart.
//!
//! House pattern (mirrors `radial_render.rs`): each test builds its own
//! renderer on the shared device, and every assertion names the pixel a node
//! lands on through the geometry the model documents (ring `k` of `d` is the
//! band `k / d .. (k + 1) / d` of the radius on a pie, a node the sector
//! of its ring over its share of its parent's span), so a node drawn on the
//! wrong ring or over the wrong span fails rather than passes.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;
use renderer::radial::*;
use renderer::{Color, RasterImage, RendererDevice};
use std::f32::consts::TAU;
use std::sync::{Arc, OnceLock};

// A test may register fonts or poll the shared GPU and execute another test's
// completion callbacks. Serialize these shared resources, not production work.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gpu() -> RendererDevice {
    static GPU: OnceLock<RendererDevice> = OnceLock::new();
    GPU.get_or_init(|| {
        let (d, q) = test_gpu::device();
        RendererDevice::new(Arc::new(d), Arc::new(q))
    })
    .clone()
}
fn renderer() -> RadialRenderer {
    RadialRenderer::new(gpu(), wgpu::TextureFormat::Rgba8Unorm).unwrap()
}

const SIZE: (u32, u32) = (600, 600);

/// A node of the tree, by its ring and position, with its colour and the
/// fractions of the turn it spans.
struct Node {
    target: RadialTarget,
    color: [u8; 3],
    span: (f64, f64),
}

const OPS: [u8; 3] = [220, 60, 40];
const RD: [u8; 3] = [80, 80, 80];
const FLEET: [u8; 3] = [60, 120, 220];
const CREW: [u8; 3] = [40, 170, 70];
const LAB: [u8; 3] = [200, 120, 200];
const FUEL: [u8; 3] = [240, 180, 40];
const TYRES: [u8; 3] = [120, 60, 200];

fn node(label: &str, value: f64, color: [u8; 3], children: Vec<RadialSlice>) -> RadialSlice {
    RadialSlice {
        children,
        ..RadialSlice::new(label, value, Color::from_rgb8(color[0], color[1], color[2]))
    }
}

/// The three-ring budget: two departments, their teams (one empty, one
/// parent with a remainder), one team's two projects. Flat, no labels.
fn chart() -> RadialChart {
    RadialChart {
        labels: RadialLabels::None,
        slices: vec![
            node(
                "Ops",
                60.0,
                OPS,
                vec![
                    node(
                        "Fleet",
                        40.0,
                        FLEET,
                        vec![
                            node("Fuel", 25.0, FUEL, vec![]),
                            node("Tyres", 15.0, TYRES, vec![]),
                        ],
                    ),
                    node("Depot", 0.0, [0, 0, 0], vec![]),
                    node("Crew", 10.0, CREW, vec![]),
                ],
            ),
            node("R&D", 40.0, RD, vec![node("Lab", 40.0, LAB, vec![])]),
        ],
        ..Default::default()
    }
}

/// Every node with a value, its ring, position, colour and span.
fn nodes() -> Vec<Node> {
    let t = |group, index| RadialTarget { group, index };
    vec![
        Node {
            target: t(0, 0),
            color: OPS,
            span: (0.0, 0.6),
        },
        Node {
            target: t(0, 1),
            color: RD,
            span: (0.6, 1.0),
        },
        Node {
            target: t(1, 0),
            color: FLEET,
            span: (0.0, 0.4),
        },
        Node {
            target: t(1, 2),
            color: CREW,
            span: (0.4, 0.5),
        },
        Node {
            target: t(1, 3),
            color: LAB,
            span: (0.6, 1.0),
        },
        Node {
            target: t(2, 0),
            color: FUEL,
            span: (0.0, 0.25),
        },
        Node {
            target: t(2, 1),
            color: TYRES,
            span: (0.25, 0.4),
        },
    ]
}

/// The screen angle of fraction `f` of the turn from the chart's start
/// angle (clockwise, y down).
fn angle(chart: &RadialChart, f: f64) -> f32 {
    chart.start_angle_degrees.to_radians() + f as f32 * TAU
}

fn pixel(img: &RasterImage, x: f32, y: f32) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, i64::from(img.width) - 1) as u32;
    let y = (y.floor() as i64).clamp(0, i64::from(img.height) - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

fn is(p: [u8; 4], color: [u8; 3]) -> bool {
    p[3] > 250 && p[..3].iter().zip(color).all(|(a, b)| a.abs_diff(b) <= 3)
}

fn is_background(p: [u8; 4]) -> bool {
    is(p, [255, 255, 255])
}

/// The centre and outer radius of the frame's one circle in logical
/// pixels: the canvas is square and the plot area centred, so the centre
/// is the canvas centre; the radius is measured along the row through the
/// centre, out to the last painted pixel on the right (inside a painted
/// node's span).
fn circle(chart: &RadialChart, img: &RasterImage, scale: f32) -> ((f32, f32), f32) {
    let (cx, cy) = (SIZE.0 as f32 / 2.0, SIZE.1 as f32 / 2.0);
    let a = angle(chart, 0.3);
    let mut r = 0.0f32;
    let mut probe = 1.0f32;
    while probe < cx {
        let (x, y) = (cx + probe * a.cos(), cy + probe * a.sin());
        if !is_background(pixel(img, x * scale, y * scale)) {
            r = probe;
        }
        probe += 0.5;
    }
    ((cx, cy), r)
}

/// The logical point at fraction `f` of the turn and `r` of the radius.
fn at(chart: &RadialChart, center: (f32, f32), radius: f32, f: f64, r: f32) -> (f32, f32) {
    let a = angle(chart, f);
    (
        center.0 + radius * r * a.cos(),
        center.1 + radius * r * a.sin(),
    )
}

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        let png = renderer::encode_png(image).unwrap();
        std::fs::write(format!("{path}/{name}.png"), png).unwrap();
    }
}

/// Each node is its colour at the middle of its band along its bisector
/// and just inside both of its angular ends; a child ring ends where its
/// parent's span ends, the remainder past the last child is background, and
/// the ring radii are thirds of the radius, at 1x and 2x.
#[test]
fn each_node_is_its_colour_on_its_ring_over_its_span() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    for scale in [1.0f32, 2.0] {
        let img = r.export_rgba(&c, SIZE, scale).unwrap();
        save(&format!("sunburst-{scale}x"), &img);
        let (center, radius) = circle(&c, &img, scale);
        assert!(radius > 200.0, "radius {radius} at {scale}x");
        let px = |f: f64, rr: f32| {
            let (x, y) = at(&c, center, radius, f, rr);
            pixel(&img, x * scale, y * scale)
        };
        for n in nodes() {
            let ring = n.target.group as f32;
            let mid_r = (ring * 2.0 + 1.0) / 6.0;
            let mid_f = (n.span.0 + n.span.1) / 2.0;
            assert!(
                is(px(mid_f, mid_r), n.color),
                "{:?} at its middle: {:?} at {scale}x",
                n.target,
                px(mid_f, mid_r)
            );
            // Inside both angular ends (a hundredth of the turn in) and at
            // both radial ends of the band (a twentieth of a ring in).
            let d = 0.01;
            for f in [n.span.0 + d, n.span.1 - d] {
                // A tenth of a ring in; the centre ring's inner end past
                // the few pixels where every slice meets.
                for rr in [
                    (ring / 3.0 + 1.0 / 30.0).max(0.1),
                    (ring + 1.0) / 3.0 - 1.0 / 30.0,
                ] {
                    assert!(
                        is(px(f, rr), n.color),
                        "{:?} at ({f:.3}, {rr:.3}): {:?} at {scale}x",
                        n.target,
                        px(f, rr)
                    );
                }
            }
        }
        // Past Fleet's end on ring 2 (Fleet's remainder, 0.4..0.6 of the
        // turn) and past Crew on ring 1 (Ops's remainder, 0.5..0.6) the rings
        // are empty; so is ring 2 over R&D, whose Lab has no children.
        for (f, ring) in [(0.5, 2.0), (0.55, 1.0), (0.8, 2.0)] {
            let p = px(f, (ring * 2.0 + 1.0) / 6.0);
            assert!(
                is_background(p),
                "ring {ring} at {f} is empty: {p:?} at {scale}x"
            );
        }
        // The ring boundaries: just inside a third is Ops, just outside is
        // Fleet; just inside two thirds is Fleet, just outside is Fuel.
        assert!(is(px(0.1, 1.0 / 3.0 - 0.02), OPS));
        assert!(is(px(0.1, 1.0 / 3.0 + 0.02), FLEET));
        assert!(is(px(0.1, 2.0 / 3.0 - 0.02), FLEET));
        assert!(is(px(0.1, 2.0 / 3.0 + 0.02), FUEL));
        // Nothing past the rim.
        assert!(is_background(px(0.1, 1.03)));
    }
}

/// A donut's hole stays: the rings share the band between the hole and the
/// rim, so ring 0 is a ring and the centre is background.
#[test]
fn a_donut_sunburst_keeps_its_hole() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.kind = RadialKind::Donut { inner_radius: 0.4 };
    let img = r.export_rgba(&c, SIZE, 1.0).unwrap();
    save("sunburst-donut", &img);
    let (center, radius) = circle(&c, &img, 1.0);
    let px = |f: f64, rr: f32| {
        let (x, y) = at(&c, center, radius, f, rr);
        pixel(&img, x, y)
    };
    assert!(is_background(px(0.1, 0.0)), "the centre is the hole");
    assert!(is_background(px(0.1, 0.38)), "just inside the hole");
    // Ring k of 3 spans 0.4 + 0.2 k .. 0.4 + 0.2 (k + 1).
    assert!(is(px(0.1, 0.5), OPS));
    assert!(is(px(0.1, 0.7), FLEET));
    assert!(is(px(0.1, 0.9), FUEL));
    assert!(is(px(0.8, 0.5), RD));
    assert!(is(px(0.8, 0.7), LAB));
    assert!(is_background(px(0.8, 0.9)));
}

/// The pick answers the node under the pixel, ring and position, where the
/// fill painted it, nothing in a remainder or the background; the GPU pick
/// and the CPU twin agree.
#[test]
fn the_pick_answers_the_ring_and_the_position() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let c = chart();
    let img = r.export_rgba(&c, SIZE, 1.0).unwrap();
    let (center, radius) = circle(&c, &img, 1.0);
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let pick = |p: (f32, f32)| {
        let gpu = frame.pick([p.0, p.1]).unwrap().resolve_blocking().unwrap();
        assert_eq!(gpu, frame.cpu_twin_pick([p.0, p.1], false), "twin at {p:?}");
        gpu
    };
    for n in nodes() {
        let ring = n.target.group as f32;
        let mid_r = (ring * 2.0 + 1.0) / 6.0;
        let mid_f = (n.span.0 + n.span.1) / 2.0;
        assert_eq!(
            pick(at(&c, center, radius, mid_f, mid_r)),
            Some(n.target),
            "the middle of {:?}",
            n.target
        );
        assert_eq!(
            pick(at(&c, center, radius, n.span.0 + 0.01, ring / 3.0 + 0.02)),
            Some(n.target),
            "the inner start corner of {:?}",
            n.target
        );
    }
    assert_eq!(
        pick(at(&c, center, radius, 0.55, 0.5)),
        None,
        "Ops's remainder"
    );
    assert_eq!(
        pick(at(&c, center, radius, 0.8, 5.0 / 6.0)),
        None,
        "ring 2 over Lab"
    );
    assert_eq!(
        pick(at(&c, center, radius, 0.1, 1.05)),
        None,
        "past the rim"
    );
}

/// Stepping the selection visits the centre ring's nodes, then each ring
/// outward, nodes with a value only, and the emphasis of the selected node
/// changes pixels inside its own band and nowhere else.
#[test]
fn step_selection_walks_the_rings_outward_and_emphasizes_one_node() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.style.outline.emphasis = true;
    c.style.outline.width = 3.0;
    c.style.hover_brightness = 0.3;
    let rest = r.export_rgba(&c, SIZE, 1.0).unwrap();
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let expected: Vec<RadialTarget> = nodes().iter().map(|n| n.target).collect();
    for want in expected.iter().chain(&expected[..1]) {
        c.step_selection(false);
        let target = c.interaction.selected.unwrap();
        assert_eq!(target, *want);
        let image = r.export_rgba(&c, SIZE, 1.0).unwrap();
        let mut changed = 0usize;
        for (i, (a, b)) in rest
            .rgba
            .chunks_exact(4)
            .zip(image.rgba.chunks_exact(4))
            .enumerate()
        {
            if a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 8) {
                continue;
            }
            changed += 1;
            let (x, y) = (i as u32 % image.width, i as u32 / image.width);
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            // Within the node's own footprint, or within the outline's
            // antialiased fringe of it (a neighbouring pixel is the node).
            let on = |dx: f32, dy: f32| {
                frame.cpu_twin_pick([p[0] + dx, p[1] + dy], true) == Some(target)
            };
            assert!(
                on(0.0, 0.0) || on(1.5, 0.0) || on(-1.5, 0.0) || on(0.0, 1.5) || on(0.0, -1.5),
                "{target:?}: pixel ({x}, {y}) changed outside the node"
            );
        }
        assert!(changed > 40, "{target:?} changed {changed} pixels");
    }
}

/// A hovered node brightens inside its own band only.
#[test]
fn hover_brightens_one_node() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.style.hover_brightness = 0.3;
    let rest = r.export_rgba(&c, SIZE, 1.0).unwrap();
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let target = RadialTarget { group: 2, index: 1 };
    c.interaction.hovered = Some(target);
    c.interaction.hover_progress = 1.0;
    let image = r.export_rgba(&c, SIZE, 1.0).unwrap();
    save("sunburst-hover", &image);
    let mut changed = 0usize;
    for (i, (a, b)) in rest
        .rgba
        .chunks_exact(4)
        .zip(image.rgba.chunks_exact(4))
        .enumerate()
    {
        if a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 8) {
            continue;
        }
        changed += 1;
        let (x, y) = (i as u32 % image.width, i as u32 / image.width);
        // The node's own footprint, or the antialiased fringe at its edge
        // (a neighbouring pixel is the node).
        let p = [x as f32 + 0.5, y as f32 + 0.5];
        let on =
            |dx: f32, dy: f32| frame.cpu_twin_pick([p[0] + dx, p[1] + dy], true) == Some(target);
        assert!(
            on(0.0, 0.0) || on(1.5, 0.0) || on(-1.5, 0.0) || on(0.0, 1.5) || on(0.0, -1.5),
            "pixel ({x}, {y}) changed outside Tyres"
        );
        if on(0.0, 0.0) {
            assert!(
                b[0] >= a[0] && b[1] >= a[1] && b[2] >= a[2],
                "brighter at ({x}, {y})"
            );
        }
    }
    assert!(changed > 200, "the hover changed {changed} pixels");
}

/// Outside labels belong to the outermost ring: their text sits past the
/// rim with a leader; an inner ring's label draws inside its band. Every
/// node with a value has a label decoration, in the hit map and listed.
#[test]
fn outside_labels_are_the_last_rings_and_inner_labels_sit_inside() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let mut c = chart();
    c.labels = RadialLabels::Outside;
    c.label_format = RadialLabelFormat::Name;
    c.format_labels();
    let img = r.export_rgba(&c, SIZE, 1.0).unwrap();
    save("sunburst-labels", &img);
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    let (center, radius) = circle(&c, &img, 1.0);
    let ids: Vec<renderer::DecorationId> = frame.decoration_ids(&c).map(|(_, id)| id).collect();
    for n in nodes() {
        let id = renderer::DecorationId::RadialLabel(n.target);
        assert!(ids.contains(&id), "{:?} is listed", n.target);
        let hit = frame.decoration(&id).expect("label hit");
        let bounds = frame.decoration_bounds(&c, hit).expect("label bounds");
        let (x, y) = (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        let distance = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
        if n.target.group == 2 {
            // An outside label's bounds include its leader from the rim, so
            // its far edge is past the rim.
            let far = [bounds.x, bounds.x + bounds.width]
                .iter()
                .flat_map(|x| [bounds.y, bounds.y + bounds.height].map(|y| (*x, y)))
                .map(|(x, y)| ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt())
                .fold(0.0f32, f32::max);
            assert!(
                far > radius * 1.05,
                "{:?} is outside: far {far} vs {radius}",
                n.target
            );
        } else {
            assert!(
                distance < radius * (n.target.group as f32 + 1.0) / 3.0,
                "{:?} is inside its band: {distance} vs {radius}",
                n.target
            );
        }
    }
    // Hidden: a node whose own placement is None has no decoration.
    c.slice_mut(RadialTarget { group: 1, index: 2 })
        .unwrap()
        .labels = Some(RadialLabels::None);
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    assert!(
        frame
            .decoration(&renderer::DecorationId::RadialLabel(RadialTarget {
                group: 1,
                index: 2
            }))
            .is_none()
    );
}

/// `prepare` names the rules: a tilt, a detail circle, children past their
/// parent, a ninth ring and a 65-node ring are refused with the model's
/// reasons.
#[test]
fn prepare_names_each_rejection() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    let reason = |r: &mut RadialRenderer, c: &RadialChart| match r.prepare(c, SIZE, 1.0) {
        Err(RadialError::Invalid(reason)) => reason.to_string(),
        other => panic!("prepared: {:?}", other.map(|_| ())),
    };
    let mut tilted = chart();
    tilted.style = RadialStyle::three_dimensional();
    assert_eq!(
        reason(&mut r, &tilted),
        "a sunburst draws flat: no tilt or depth"
    );
    let mut split = chart();
    split.split = Some(RadialSplit {
        slice_index: 1,
        kind: RadialKind::Pie,
        children: vec![RadialSlice::new("Lab", 40.0, Color::WHITE)],
    });
    assert_eq!(reason(&mut r, &split), "a sunburst has no detail circle");
    let mut over = chart();
    over.slices[0].children[2].value = 30.0;
    assert_eq!(
        reason(&mut r, &over),
        "a node's children sum to at most its value"
    );
    let mut deep = chart();
    {
        let mut n = &mut deep.slices[1].children[0];
        for _ in 0..MAX_SUNBURST_DEPTH - 1 {
            n.children = vec![RadialSlice::new("deeper", n.value, Color::WHITE)];
            n = &mut n.children[0];
        }
    }
    assert_eq!(reason(&mut r, &deep), "a sunburst has at most 8 rings");
    let mut wide = chart();
    wide.slices[1].children[0].children = (0..65)
        .map(|i| RadialSlice::new(format!("p{i}"), 0.5, Color::WHITE))
        .collect();
    assert_eq!(
        reason(&mut r, &wide),
        "a sunburst ring has at most 64 nodes"
    );
    // The well-formed chart prepares.
    r.prepare(&chart(), SIZE, 1.0).unwrap();
}

/// The limits' chart: eight rings of 64 nodes each draws and picks its last
/// ring (rule 10 for a DataLess chart, approved on #140).
#[test]
fn eight_rings_of_sixty_four_nodes_draw_and_pick() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = renderer();
    // Each root has a chain of one child per ring, so every ring holds 64.
    fn chain(depth: usize, i: usize) -> RadialSlice {
        let hue = (i * 37 % 64) as u8;
        let mut node = RadialSlice::new(
            format!("n{depth}-{i}"),
            64.0,
            Color::from_rgb8(40 + hue * 3, 200 - hue * 2, 60 + depth as u8 * 20),
        );
        if depth + 1 < MAX_SUNBURST_DEPTH {
            node.children = vec![chain(depth + 1, i)];
        }
        node
    }
    let c = RadialChart {
        labels: RadialLabels::None,
        slices: (0..MAX_RADIAL_SLICES).map(|i| chain(0, i)).collect(),
        ..Default::default()
    };
    assert_eq!(c.validate(), Ok(()));
    assert_eq!(c.group_count(), MAX_SUNBURST_DEPTH);
    assert_eq!(c.node_count(), MAX_SUNBURST_DEPTH * MAX_RADIAL_SLICES);
    let started = std::time::Instant::now();
    let img = r.export_rgba(&c, SIZE, 1.0).unwrap();
    eprintln!("8 x 64 sunburst exported in {:?}", started.elapsed());
    save("sunburst-8x64", &img);
    let (center, radius) = circle(&c, &img, 1.0);
    let frame = r.prepare(&c, SIZE, 1.0).unwrap();
    // The middle of the last ring over the 10th root's span.
    let f = (9.5) / 64.0;
    let rr = (7.0 * 2.0 + 1.0) / 16.0;
    let p = at(&c, center, radius, f, rr);
    let started = std::time::Instant::now();
    let picked = frame.pick([p.0, p.1]).unwrap().resolve_blocking().unwrap();
    eprintln!("picked in {:?}", started.elapsed());
    assert_eq!(picked, Some(RadialTarget { group: 7, index: 9 }));
    assert_eq!(frame.cpu_twin_pick([p.0, p.1], false), picked);
}
