//! Feynman diagrams (#234): a DataLess series drawn on the CPU and
//! composited in the data slot, written as vector parts in SVG and PDF.
//! Every pixel test exports at 1x and 2x.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::feynman::{
    FeynmanSeries, FeynmanVertex, ParticleLine, Propagator, VertexMarker, VertexRole,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{Chart, Color, DataRenderType, RasterImage, Renderer, RendererDevice, SeriesConfig};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 360;

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

fn series(id: &str, diagram: FeynmanSeries) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::Feynman(diagram),
    }
}

/// A plain chart over 0..1 on both axes, no grid, title, legend or axis
/// lines.
fn chart() -> Chart {
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
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    chart
}

fn vertex(id: &str, role: VertexRole, at: Option<[f64; 2]>) -> FeynmanVertex {
    FeynmanVertex {
        at,
        ..FeynmanVertex::new(id, role)
    }
}

/// Bhabha's s channel, unplaced.
fn s_channel() -> FeynmanSeries {
    FeynmanSeries {
        vertices: vec![
            vertex("e-", VertexRole::Incoming, None),
            vertex("e+", VertexRole::Incoming, None),
            vertex("v1", VertexRole::Internal, None),
            vertex("v2", VertexRole::Internal, None),
            vertex("mu-", VertexRole::Outgoing, None),
            vertex("mu+", VertexRole::Outgoing, None),
        ],
        propagators: vec![
            Propagator::new("p1", "e-", "v1", ParticleLine::Fermion),
            Propagator::new("p2", "v1", "e+", ParticleLine::Fermion),
            Propagator::new("g", "v1", "v2", ParticleLine::Photon),
            Propagator::new("p3", "v2", "mu-", ParticleLine::Fermion),
            Propagator::new("p4", "mu+", "v2", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    }
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(chart, series),
        renderer::ExportOptions::panel(scale),
    )
    .block_on()
    .unwrap()
}

/// The export pixel of data point `(x, y)`.
fn at(chart: &Chart, x: f64, y: f64, scale: f32) -> (u32, u32) {
    let da = chart.config().data_area().expect("data area");
    let px = da.x as f64 + x * da.width as f64;
    let py = da.y as f64 + (1.0 - y) * da.height as f64;
    (
        (px * scale as f64).round() as u32,
        (py * scale as f64).round() as u32,
    )
}

fn ink(p: &[u8]) -> bool {
    p[3] > 100
}

/// Whether ink lies within `r` export pixels of `(x, y)`.
fn near(img: &RasterImage, (x, y): (u32, u32), r: u32) -> bool {
    (y.saturating_sub(r)..=(y + r).min(img.height - 1)).any(|py| {
        (x.saturating_sub(r)..=(x + r).min(img.width - 1)).any(|px| {
            let i = ((py * img.width + px) * 4) as usize;
            ink(&img.rgba[i..i + 4])
        })
    })
}

const PARTICLES: [ParticleLine; 12] = [
    ParticleLine::Plain,
    ParticleLine::Fermion,
    ParticleLine::AntiFermion,
    ParticleLine::Photon,
    ParticleLine::ChargedBoson,
    ParticleLine::Gluon,
    ParticleLine::Scalar,
    ParticleLine::ChargedScalar,
    ParticleLine::Ghost,
    ParticleLine::Majorana,
    ParticleLine::Graviton,
    ParticleLine::DoubleLine,
];

/// Every particle on its own row, from x 0.25 to 0.75, between hidden
/// markers.
fn rows() -> FeynmanSeries {
    let mut diagram = FeynmanSeries::default();
    for (k, particle) in PARTICLES.iter().enumerate() {
        let y = 0.06 + k as f64 * 0.08;
        for (end, x) in [("a", 0.25), ("b", 0.75)] {
            diagram.vertices.push(FeynmanVertex {
                marker: Some(VertexMarker::Hidden),
                ..vertex(&format!("{k}{end}"), VertexRole::Internal, Some([x, y]))
            });
        }
        diagram.propagators.push(Propagator::new(
            format!("{k}"),
            format!("{k}a"),
            format!("{k}b"),
            *particle,
        ));
    }
    diagram
}

#[test]
fn every_particle_draws_between_its_vertices() {
    let mut r = renderer();
    let chart = chart();
    let list = [series("rows", rows())];
    for scale in [1.0f32, 2.0] {
        let image = export(&mut r, &chart, &list, scale);
        for (k, particle) in PARTICLES.iter().enumerate() {
            let y = 0.06 + k as f64 * 0.08;
            // Ink along the line (within its wave or coil) ...
            for x in [0.3, 0.5, 0.7] {
                let reach = (11.0 * scale) as u32;
                assert!(
                    near(&image, at(&chart, x, y, scale), reach),
                    "{scale}x {particle:?} at {x}"
                );
            }
            // ... and none past its ends.
            assert!(
                !near(&image, at(&chart, 0.1, y, scale), (4.0 * scale) as u32),
                "{scale}x {particle:?} before its start"
            );
        }
    }
}

#[test]
fn a_provisional_layout_draws_as_its_commit() {
    let mut r = renderer();
    let chart = chart();
    let id = r
        .register_chart(chart.config().clone(), vec![series("d", s_channel())])
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let provisional = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        // The internal vertices sit at the quarters, dots drawn.
        assert!(near(&provisional, at(&chart, 0.25, 0.5, scale), 1));
        assert!(near(&provisional, at(&chart, 0.75, 0.5, scale), 1));
        let committed = if scale == 1.0 {
            assert!(r.auto_layout(id, "d").unwrap());
            let diagram = r.chart_series(id).unwrap()[0].feynman().unwrap().clone();
            assert!(diagram.vertices.iter().all(|v| v.at.is_some()), "committed");
            assert!(!r.auto_layout(id, "d").unwrap(), "nothing left to place");
            r.export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap()
        } else {
            r.export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap()
        };
        assert!(
            provisional.rgba == committed.rgba,
            "{scale}x: the commit draws what was drawn"
        );
    }
    assert!(r.auto_layout(id, "missing").is_err());
}

#[test]
fn an_ssot_edit_of_the_diagram_redraws_it() {
    let mut r = renderer();
    let chart = chart();
    let mut diagram = s_channel();
    diagram.auto_layout().unwrap();
    let id = r
        .register_chart(chart.config().clone(), vec![series("d", diagram.clone())])
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let before = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(near(&before, at(&chart, 0.25, 0.5, scale), 1));
        let mut moved = diagram.clone();
        moved.vertices[2].at = Some([0.25, 0.9]);
        r.set_chart_series(id, vec![series("d", moved)]).unwrap();
        let after = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(
            near(&after, at(&chart, 0.25, 0.9, scale), 1),
            "{scale}x new"
        );
        assert!(
            !near(&after, at(&chart, 0.25, 0.5, scale), 1),
            "{scale}x old place cleared"
        );
        r.set_chart_series(id, vec![series("d", diagram.clone())])
            .unwrap();
    }
}

#[test]
fn auto_fit_all_fits_the_vertices() {
    let mut r = renderer();
    let mut chart = chart();
    chart.set_x_range(-10.0, 10.0);
    chart.set_y_range(-10.0, 10.0);
    let id = r
        .register_chart(chart.config().clone(), vec![series("d", s_channel())])
        .unwrap();
    let ticket = r.begin_auto_fit_all(id, 0.0).unwrap();
    let resolved = ticket.resolve_blocking().unwrap();
    r.commit_auto_fit_all(resolved).unwrap();
    let config = r.chart_config(id).unwrap();
    let (x, y) = (&config.bottom_x, &config.left_y);
    assert!(
        x.min <= 0.0 && x.min > -1.0 && x.max >= 1.0 && x.max < 2.0,
        "{x:?}"
    );
    assert!(
        y.min <= 1.0 / 3.0 + 1e-9 && y.min > -1.0 && y.max >= 2.0 / 3.0 - 1e-9 && y.max < 2.0,
        "{:?}",
        (y.min, y.max)
    );
}

#[test]
fn svg_and_pdf_carry_the_diagram_as_vector_curves() {
    let mut r = renderer();
    let chart = chart();
    let list = [series("d", s_channel())];
    for scale in [1.0f32, 2.0] {
        let svg = r
            .export_svg(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert!(!svg.contains("<image"), "{scale}x: no data image");
        // The photon is one path of cubics; the fermions straight strokes.
        let photon = svg
            .lines()
            .find(|line| line.contains("<path") && line.contains('C'))
            .expect("a curved path");
        assert!(photon.matches('C').count() > 10, "{scale}x: {photon}");
        let pdf = r
            .export_pdf(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(!String::from_utf8_lossy(&pdf).contains("/Subtype /Image"));
    }
}

#[cfg(feature = "serde")]
#[test]
fn a_document_with_the_diagram_round_trips_and_draws_the_same() {
    let mut r = renderer();
    let chart = chart();
    let list = vec![series("d", s_channel())];
    let json = serde_json::to_string(&list).unwrap();
    let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, list);
    for scale in [1.0f32, 2.0] {
        assert!(
            export(&mut r, &chart, &list, scale).rgba == export(&mut r, &chart, &back, scale).rgba
        );
    }
}

#[test]
fn a_streamed_display_refuses_the_diagram() {
    // The series reads no column, so a stream has nothing to feed (rule 8):
    // the model's rule refuses columns on it.
    let mut columns = series("d", s_channel());
    columns.x_column = "x".into();
    let chart = chart();
    let mut r = renderer();
    assert!(
        r.register_chart(chart.config().clone(), vec![columns])
            .is_err()
    );
}

/// A ladder of `rungs` rungs between two rails, legs at both ends.
fn ladder(rungs: usize) -> FeynmanSeries {
    let mut diagram = FeynmanSeries::default();
    for (id, role) in [
        ("in_a", VertexRole::Incoming),
        ("in_b", VertexRole::Incoming),
        ("out_a", VertexRole::Outgoing),
        ("out_b", VertexRole::Outgoing),
    ] {
        diagram.vertices.push(vertex(id, role, None));
    }
    for k in 0..rungs {
        for rail in ["a", "b"] {
            diagram
                .vertices
                .push(vertex(&format!("{rail}{k}"), VertexRole::Internal, None));
        }
        let particle = if k % 10 == 0 {
            ParticleLine::Photon
        } else {
            ParticleLine::Plain
        };
        diagram.propagators.push(Propagator::new(
            format!("r{k}"),
            format!("a{k}"),
            format!("b{k}"),
            particle,
        ));
        if k > 0 {
            for rail in ["a", "b"] {
                diagram.propagators.push(Propagator::new(
                    format!("{rail}{k}-"),
                    format!("{rail}{}", k - 1),
                    format!("{rail}{k}"),
                    ParticleLine::Fermion,
                ));
            }
        }
    }
    let last = rungs - 1;
    for (leg, from, to) in [
        ("la", "in_a".to_string(), "a0".to_string()),
        ("lb", "in_b".to_string(), "b0".to_string()),
        ("ra", format!("a{last}"), "out_a".to_string()),
        ("rb", format!("b{last}"), "out_b".to_string()),
    ] {
        diagram
            .propagators
            .push(Propagator::new(leg, from, to, ParticleLine::Fermion));
    }
    diagram
}

#[test]
fn thousands_of_rungs_lay_out_and_draw_with_no_count_limit() {
    // Rule 10: no count limit. 5,000 rungs: 10,004 vertices and 15,002
    // propagators lay out, draw and export; the times are printed, never
    // capped.
    let diagram = ladder(5_000);
    assert_eq!(diagram.vertices.len(), 10_004);
    assert_eq!(diagram.propagators.len(), 15_002);
    let start = std::time::Instant::now();
    let positions = diagram.positions().expect("the layout");
    let layout = start.elapsed();
    assert!(
        positions
            .iter()
            .all(|p| p[0].is_finite() && p[1].is_finite())
    );
    // The barycentric layout pulls a long ladder's rails together in its
    // middle (each rung pulls its ends to their mean); vertices that meet
    // are spread apart, so every one keeps a place of its own.
    let a = positions[diagram.vertex_row("a2500").unwrap()];
    let b = positions[diagram.vertex_row("b2500").unwrap()];
    assert!(a != b, "{a:?} {b:?}");
    let mut r = renderer();
    let chart = chart();
    let list = [series("ladder", diagram)];
    for scale in [1.0f32, 2.0] {
        let start = std::time::Instant::now();
        let image = export(&mut r, &chart, &list, scale);
        eprintln!(
            "Feynman ladder: layout {layout:?}, 10,004 vertices + 15,002 propagators exported at \
             {scale}x in {:?}",
            start.elapsed()
        );
        assert!(
            near(&image, at(&chart, a[0], a[1], scale), 2),
            "{scale}x rail"
        );
    }
}

#[test]
fn line_colours_and_widths_follow_their_overrides() {
    let mut r = renderer();
    let chart = chart();
    let mut diagram = rows();
    diagram.propagators.truncate(1);
    diagram.vertices.truncate(2);
    diagram.propagators[0].color = Some(Color::from_rgb8(255, 0, 0));
    diagram.propagators[0].width_px = Some(6.0);
    let list = [series("d", diagram)];
    for scale in [1.0f32, 2.0] {
        let image = export(&mut r, &chart, &list, scale);
        let (x, y) = at(&chart, 0.5, 0.06, scale);
        let i = ((y * image.width + x) * 4) as usize;
        let p = &image.rgba[i..i + 4];
        assert!(
            p[0] > 200 && p[1] < 60 && p[2] < 60 && p[3] > 200,
            "{scale}x {p:?}"
        );
        // Six logical pixels wide: ink two pixels above the centre line.
        assert!(
            near(&image, (x, y - (2.0 * scale) as u32), 0),
            "{scale}x width"
        );
    }
}

// Interactions (rule 6).

fn pick(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
    at: (u32, u32),
) -> Option<renderer::PickedData> {
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 4.0 * scale,
    };
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    hit
}

/// The s channel committed, with a label on the photon.
fn labelled() -> FeynmanSeries {
    let mut diagram = s_channel();
    diagram.auto_layout().unwrap();
    diagram.propagators[2].label = Some(renderer::text::RichText::plain(
        "γ*",
        Color::BLACK,
        14.0,
        "",
    ));
    diagram
}

#[test]
fn vertices_propagators_and_labels_pick_with_their_rows() {
    let mut r = renderer();
    let chart = chart();
    let list = [series("d", labelled())];
    for scale in [1.0f32, 2.0] {
        let hit = pick(&mut r, &chart, &list, scale, at(&chart, 0.25, 0.5, scale));
        assert!(
            matches!(
                hit,
                Some(renderer::PickedData::FeynmanVertex {
                    vertex_index: 2,
                    ..
                })
            ),
            "{scale}x vertex: {hit:?}"
        );
        // On the photon's axis, between its wiggles' crests.
        let hit = pick(&mut r, &chart, &list, scale, at(&chart, 0.4, 0.5, scale));
        assert!(
            matches!(
                hit,
                Some(renderer::PickedData::FeynmanPropagator {
                    propagator_index: 2,
                    ..
                })
            ),
            "{scale}x photon: {hit:?}"
        );
        // The photon's label sits above its middle.
        let (x, y) = at(&chart, 0.5, 0.5, scale);
        let hit = pick(&mut r, &chart, &list, scale, (x, y - (16.0 * scale) as u32));
        assert!(
            matches!(
                hit,
                Some(renderer::PickedData::FeynmanLabel {
                    part: renderer::FeynmanLabelPart::Propagator,
                    index: 2,
                    ..
                })
            ),
            "{scale}x label: {hit:?}"
        );
        let hit = pick(&mut r, &chart, &list, scale, at(&chart, 0.5, 0.95, scale));
        assert_eq!(hit, None, "{scale}x empty");
    }
}

/// Whether the picked colour (the default gold) lies in `rect`, export
/// pixels.
fn gold_in(img: &RasterImage, rect: (u32, u32, u32, u32)) -> bool {
    let (x0, y0, x1, y1) = rect;
    (y0..=y1.min(img.height - 1)).any(|y| {
        (x0..=x1.min(img.width - 1)).any(|x| {
            let i = ((y * img.width + x) * 4) as usize;
            let p = &img.rgba[i..i + 4];
            p[3] > 100 && p[0] > 200 && p[1] > 150 && p[2] < 90
        })
    })
}

/// The square of half side `r` about `(x, y)`.
fn around((x, y): (u32, u32), r: u32) -> (u32, u32, u32, u32) {
    (x.saturating_sub(r), y.saturating_sub(r), x + r, y + r)
}

/// The diagram the highlight tests pick from: the s channel committed,
/// a label on the photon, a momentum arrow under it and a label on v1.
fn every_label() -> FeynmanSeries {
    let mut diagram = labelled();
    diagram.vertices[2].label = Some(renderer::text::RichText::plain("v", Color::BLACK, 16.0, ""));
    diagram.propagators[2].momentum = Some(renderer::feynman::MomentumArrow {
        label: renderer::text::RichText::plain("q", Color::BLACK, 16.0, ""),
        side: renderer::feynman::MomentumSide::Right,
        reversed: false,
        label_offset: [0.0; 2],
    });
    diagram
}

#[test]
fn every_picked_part_is_drawn_in_the_picked_style() {
    use renderer::PickedDataRef as Ref;
    let mut r = renderer();
    let mut chart = chart();
    let diagram = every_label();
    let list = [series("d", diagram.clone())];
    // The label blocks in drawing order: the photon's, its momentum's,
    // then v1's, in logical pixels.
    let config = chart.config().clone();
    let space = renderer::annotation::pixel_space_for_config(&config);
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    let mut texts = Vec::new();
    diagram
        .emit(
            &space,
            None,
            &measure,
            &mut renderer::sink::Parts(&mut |part| {
                if let renderer::DrawnPart::Text(rect) = part {
                    texts.push(rect);
                }
            }),
        )
        .unwrap();
    assert_eq!(texts.len(), 3);
    let label = |k: usize, scale: f32| {
        let t: renderer::layout::RectF = texts[k];
        let pad = 5.0;
        (
            ((t.x - pad) * scale).max(0.0) as u32,
            ((t.y - pad) * scale).max(0.0) as u32,
            ((t.x + t.width + pad) * scale) as u32,
            ((t.y + t.height + pad) * scale) as u32,
        )
    };
    let label_ref = |part, index| Ref::FeynmanLabel {
        source_id: None,
        series_id: "d".into(),
        part,
        index,
    };
    for scale in [1.0f32, 2.0] {
        let r6 = (6.0 * scale) as u32;
        let cases: Vec<(&str, Ref, (u32, u32, u32, u32))> = vec![
            (
                "an internal vertex",
                Ref::FeynmanVertex {
                    source_id: None,
                    series_id: "d".into(),
                    vertex_index: 2,
                },
                around(at(&chart, 0.25, 0.5, scale), r6),
            ),
            (
                "a leg end",
                Ref::FeynmanVertex {
                    source_id: None,
                    series_id: "d".into(),
                    vertex_index: 4,
                },
                around(at(&chart, 1.0, 2.0 / 3.0, scale), r6),
            ),
            (
                "a propagator",
                Ref::FeynmanPropagator {
                    source_id: None,
                    series_id: "d".into(),
                    propagator_index: 0,
                },
                around(at(&chart, 0.125, (2.0 / 3.0 + 0.5) / 2.0, scale), r6),
            ),
            (
                "the propagator label",
                label_ref(renderer::FeynmanLabelPart::Propagator, 2),
                label(0, scale),
            ),
            (
                "the momentum label",
                label_ref(renderer::FeynmanLabelPart::Momentum, 2),
                label(1, scale),
            ),
            (
                "the vertex label",
                label_ref(renderer::FeynmanLabelPart::Vertex, 2),
                label(2, scale),
            ),
        ];
        chart.config_mut().picked_data = None;
        let plain = export(&mut r, &chart, &list, scale);
        for (name, reference, place) in cases {
            assert!(!gold_in(&plain, place), "{scale}x {name}: unpicked");
            chart.config_mut().picked_data = Some(renderer::DataSelectionsConfig {
                refs: vec![reference],
                ..Default::default()
            });
            let picked = export(&mut r, &chart, &list, scale);
            assert!(gold_in(&picked, place), "{scale}x {name}: picked");
            chart.config_mut().picked_data = None;
        }
    }
}

#[test]
fn plot_area_and_decoration_drags_keep_the_diagram_on_its_data() {
    use renderer::layout::{Element, NudgeResult, Side};
    let mut r = renderer();
    let mut base = chart();
    {
        // Decorations to drag: a title, a legend and labelled axes.
        let text = |s: &str| renderer::text::RichText::plain(s, Color::BLACK, 12.0, "");
        let config = base.config_mut();
        config.chart_title.visible = true;
        config.chart_title.text = text("Bhabha scattering");
        config.legend.visible = true;
        config.legend.content = text("s channel");
        for axis in [&mut config.bottom_x, &mut config.left_y] {
            axis.line_visible = true;
            axis.title_option.visible = true;
            axis.title_option.text = text("axis");
            axis.label_style.label_visible = true;
        }
    }
    let list = [series("d", labelled())];
    let measure = renderer::CpuTextMeasure::for_style(&base.config().draw_style);
    let edits: [(&str, Element, f32, f32); 8] = [
        ("plot-area drag", Element::DataArea, 24.0, 12.0),
        (
            "plot-area resize",
            Element::DataAreaEdge(Side::Right),
            -40.0,
            0.0,
        ),
        (
            "plot-area resize, top",
            Element::DataAreaEdge(Side::Top),
            0.0,
            20.0,
        ),
        ("title drag", Element::ChartTitle, 10.0, 4.0),
        ("legend drag", Element::Legend, -30.0, 10.0),
        ("axis drag", Element::Axis(Side::Bottom), 0.0, 6.0),
        ("axis title drag", Element::AxisTitle(Side::Left), -4.0, 8.0),
        (
            "tick label drag",
            Element::AxisLabel(Side::Bottom),
            0.0,
            3.0,
        ),
    ];
    for (name, element, dx, dy) in edits {
        let moves_plot = matches!(element, Element::DataArea | Element::DataAreaEdge(_));
        let mut config = base.config().clone();
        assert_eq!(
            config.nudge(element, &measure, dx, dy),
            NudgeResult::Moved,
            "{name}"
        );
        let chart = Chart::new(config);
        for scale in [1.0f32, 2.0] {
            // The diagram is drawn on its data where the edited layout puts
            // it: the vertex v1 at (0.25, 0.5), the photon's middle.
            let image = export(&mut r, &chart, &list, scale);
            assert!(
                near(&image, at(&chart, 0.25, 0.5, scale), 1),
                "{scale}x {name}: the vertex"
            );
            assert!(
                near(&image, at(&chart, 0.5, 0.5, scale), (5.0 * scale) as u32),
                "{scale}x {name}: the photon"
            );
            // It picks where it is drawn.
            let hit = pick(&mut r, &chart, &list, scale, at(&chart, 0.25, 0.5, scale));
            assert!(
                matches!(
                    hit,
                    Some(renderer::PickedData::FeynmanVertex {
                        vertex_index: 2,
                        ..
                    })
                ),
                "{scale}x {name}: {hit:?}"
            );
        }
        if moves_plot {
            assert_ne!(
                chart.config().data_area(),
                base.config().data_area(),
                "{name} moves the plot"
            );
        }
    }
}

#[test]
fn a_vertex_drag_moves_its_lines_and_the_first_drag_commits() {
    let mut r = renderer();
    let chart = chart();
    let id = r
        .register_chart(chart.config().clone(), vec![series("d", s_channel())])
        .unwrap();
    let da = chart.config().data_area().unwrap().0;
    // Up a fifth of the plot.
    let dy = -(da.height as f32) * 0.2;
    let moved = r
        .drag_feynman(id, "d", renderer::FeynmanHit::Vertex(2), 0.0, dy)
        .unwrap();
    assert_eq!(moved, renderer::layout::NudgeResult::Moved);
    let diagram = r.chart_series(id).unwrap()[0].feynman().unwrap().clone();
    assert!(diagram.vertices.iter().all(|v| v.at.is_some()), "committed");
    let v1 = diagram.vertices[2].at.unwrap();
    assert!(
        (v1[0] - 0.25).abs() < 1e-6 && (v1[1] - 0.7).abs() < 1e-3,
        "{v1:?}"
    );
    for scale in [1.0f32, 2.0] {
        let image = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(
            near(&image, at(&chart, 0.25, 0.7, scale), 1),
            "{scale}x vertex"
        );
        assert!(
            !near(&image, at(&chart, 0.25, 0.5, scale), 1),
            "{scale}x old"
        );
        // The fermion from e⁻ (0, 2/3) follows it.
        assert!(
            near(&image, at(&chart, 0.125, (2.0 / 3.0 + 0.7) / 2.0, scale), 2),
            "{scale}x line"
        );
    }
    // A bend and a label drag.
    assert_eq!(
        r.drag_feynman(id, "d", renderer::FeynmanHit::Propagator(2), 0.0, -30.0)
            .unwrap(),
        renderer::layout::NudgeResult::Moved
    );
    assert!(
        r.chart_series(id).unwrap()[0]
            .feynman()
            .unwrap()
            .propagators[2]
            .bend
            .is_some()
    );
    // A label moves through the value-label drag, in logical pixels: the
    // host edits its series and registers it again. The panel drag
    // refuses it.
    assert!(
        r.drag_feynman(
            id,
            "d",
            renderer::FeynmanHit::Label(renderer::FeynmanLabelPart::Vertex, 0),
            3.0,
            4.0
        )
        .is_err()
    );
    let mut list = r.chart_series(id).unwrap().to_vec();
    assert_eq!(
        list[0]
            .feynman_mut()
            .unwrap()
            .drag_label(renderer::FeynmanLabelPart::Vertex, 0, 3.0, 4.0),
        renderer::layout::NudgeResult::Moved
    );
    r.set_chart_series(id, list).unwrap();
    assert_eq!(
        r.chart_series(id).unwrap()[0].feynman().unwrap().vertices[0].label_offset,
        [3.0, 4.0]
    );
    assert!(
        r.drag_feynman(id, "missing", renderer::FeynmanHit::Vertex(0), 1.0, 1.0)
            .is_err()
    );
}

#[test]
fn a_pick_inside_a_break_gap_answers_nothing() {
    use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark};
    let mut r = renderer();
    let mut chart = chart();
    chart.config_mut().left_y.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(0.55, 0.62)],
        style: AxisBreakStyle::Continuous,
        gap_px: 40.0,
        mark: BreakMark::default(),
        frame_each_panel: false,
        sizing: Default::default(),
    });
    // A vertical photon from (0.5, 0.2) to (0.5, 0.9) crosses the cut.
    let diagram = FeynmanSeries {
        vertices: vec![
            vertex("a", VertexRole::Internal, Some([0.5, 0.2])),
            vertex("b", VertexRole::Internal, Some([0.5, 0.9])),
        ],
        propagators: vec![Propagator::new("g", "a", "b", ParticleLine::Plain)],
        ..FeynmanSeries::default()
    };
    let list = [series("d", diagram)];
    for scale in [1.0f32, 2.0] {
        let space = renderer::annotation::pixel_space_for_config(&chart.config().scaled(scale));
        use renderer::AnnotationSpace as _;
        let below = space
            .value_to_px(renderer::AnnotationAxis::Y, 0.5499)
            .unwrap();
        let above = space
            .value_to_px(renderer::AnnotationAxis::Y, 0.6201)
            .unwrap();
        let x = space.value_to_px(renderer::AnnotationAxis::X, 0.5).unwrap();
        let mid = ((below + above) / 2.0).round() as u32;
        let hit = pick(&mut r, &chart, &list, scale, (x.round() as u32, mid));
        assert_eq!(hit, None, "{scale}x gap");
        let hit = pick(
            &mut r,
            &chart,
            &list,
            scale,
            (x.round() as u32, below.round() as u32 + 8),
        );
        assert!(
            matches!(hit, Some(renderer::PickedData::FeynmanPropagator { .. })),
            "{scale}x below the gap: {hit:?}"
        );
    }
}

#[test]
fn a_diagram_draws_under_the_annotation_layer() {
    let mut r = renderer();
    let mut chart = chart();
    // An annotation line across the middle, over the diagram.
    chart
        .config_mut()
        .annotations
        .items
        .push(renderer::Annotation::new(
            renderer::AnnotationKind::ReferenceLine {
                axis: renderer::AnnotationAxis::Y,
                value: 0.2,
                style: renderer::AnnotationLineStyle {
                    color: Color::from_rgb8(0, 0, 255),
                    width: 3.0,
                    style: renderer::line::LineStylePreset::Solid,
                },
                label: None,
            },
        ));
    let list = [series("d", labelled())];
    for scale in [1.0f32, 2.0] {
        let image = export(&mut r, &chart, &list, scale);
        assert!(
            near(&image, at(&chart, 0.25, 0.5, scale), 1),
            "{scale}x vertex"
        );
        let (x, y) = at(&chart, 0.9, 0.2, scale);
        let i = ((y * image.width + x) * 4) as usize;
        assert!(image.rgba[i + 2] > 200, "{scale}x annotation");
    }
}

// Checks of the paths the tests above reach only indirectly (#234).

#[path = "support/pdf_structure.rs"]
mod pdf_structure;
#[path = "support/svg_parts.rs"]
mod svg_parts;

#[test]
fn vertex_and_momentum_labels_pick_with_their_part() {
    let mut r = renderer();
    let chart = chart();
    let mut diagram = labelled();
    diagram.vertices[2].label = Some(renderer::text::RichText::plain("v", Color::BLACK, 16.0, ""));
    diagram.propagators[2].momentum = Some(renderer::feynman::MomentumArrow {
        label: renderer::text::RichText::plain("q", Color::BLACK, 16.0, ""),
        side: renderer::feynman::MomentumSide::Right,
        reversed: false,
        label_offset: [0.0; 2],
    });
    let list = [series("d", diagram.clone())];
    let config = chart.config();
    let space = renderer::annotation::pixel_space_for_config(config);
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    // The emit's text blocks in drawing order: the propagator label (row
    // 2), the momentum label (row 2), then the vertex label (row 0).
    let mut texts = Vec::new();
    diagram
        .emit(
            &space,
            None,
            &measure,
            &mut renderer::sink::Parts(&mut |part| {
                if let renderer::DrawnPart::Text(rect) = part {
                    texts.push(rect);
                }
            }),
        )
        .unwrap();
    assert_eq!(texts.len(), 3);
    for (part, index, rect) in [
        (renderer::FeynmanLabelPart::Vertex, 2, texts[2]),
        (renderer::FeynmanLabelPart::Momentum, 2, texts[1]),
    ] {
        for scale in [1.0f32, 2.0] {
            let centre = (
                ((rect.x + rect.width / 2.0) * scale).round() as u32,
                ((rect.y + rect.height / 2.0) * scale).round() as u32,
            );
            let hit = pick(&mut r, &chart, &list, scale, centre);
            assert!(
                matches!(
                    hit,
                    Some(renderer::PickedData::FeynmanLabel { part: p, index: i, .. })
                        if p == part && i == index
                ),
                "{scale}x {part:?}: {hit:?}"
            );
        }
    }
}

#[test]
fn a_diagram_draws_and_drags_in_a_subplot_and_an_overlay() {
    let mut r = renderer();
    let mut chart = chart();
    // The parent on the top half, a subplot on the bottom half; the
    // overlay's y axis runs over 0..2.
    chart.config_mut().chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT / 2,
    });
    let mut sub = chart.config().clone();
    sub.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT / 2,
        width: WIDTH,
        height: HEIGHT / 2,
    });
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 2.0;
    overlay.series = vec!["over".into()];
    let config = chart.config_mut();
    config.overlays.push(overlay);
    config.subplots.push(renderer::Subplot {
        config: sub,
        series: vec!["below".into()],
        link_x: None,
        link_y: None,
        marker: None,
    });
    let pair = |a: [f64; 2], b: [f64; 2]| FeynmanSeries {
        vertices: vec![
            vertex("a", VertexRole::Internal, Some(a)),
            vertex("b", VertexRole::Internal, Some(b)),
        ],
        propagators: vec![Propagator::new("l", "a", "b", ParticleLine::Plain)],
        ..FeynmanSeries::default()
    };
    let list = vec![
        series("over", pair([0.2, 1.0], [0.4, 1.0])),
        series("below", pair([0.6, 0.5], [0.8, 0.5])),
    ];
    let id = r
        .register_chart(chart.config().clone(), list.clone())
        .unwrap();
    // Each series is placed in its own panel's space.
    let document = chart.config();
    let over_space =
        renderer::annotation::pixel_space_for_config(&document.overlay_panel(0).unwrap());
    let below_space =
        renderer::annotation::pixel_space_for_config(&document.subplot_config(0).unwrap());
    use renderer::AnnotationSpace as _;
    let point = |space: &renderer::annotation::PixelSpace, p: [f64; 2]| {
        (
            space
                .value_to_px(renderer::AnnotationAxis::X, p[0])
                .unwrap(),
            space
                .value_to_px(renderer::AnnotationAxis::Y, p[1])
                .unwrap(),
        )
    };
    let over_mid = point(&over_space, [0.3, 1.0]);
    let below_mid = point(&below_space, [0.7, 0.5]);
    assert!(
        below_mid.1 > (HEIGHT / 2) as f32,
        "{below_mid:?} in the subplot"
    );
    for scale in [1.0f32, 2.0] {
        let image = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        for (name, (x, y)) in [("overlay", over_mid), ("subplot", below_mid)] {
            let at = ((x * scale).round() as u32, (y * scale).round() as u32);
            assert!(near(&image, at, 2), "{scale}x {name} line at {at:?}");
        }
    }
    // A drag converts its delta in the subplot's own space: 20 px up is
    // 20 px of the subplot's y axis.
    let plot = below_space.plot_rect();
    assert_eq!(
        r.drag_feynman(id, "below", renderer::FeynmanHit::Vertex(0), 0.0, -20.0)
            .unwrap(),
        renderer::layout::NudgeResult::Moved
    );
    let moved = r
        .chart_series(id)
        .unwrap()
        .iter()
        .find(|s| s.series_id == "below")
        .and_then(|s| s.feynman())
        .unwrap()
        .vertices[0]
        .at
        .unwrap();
    let want = 0.5 + 20.0 / f64::from(plot.height);
    assert!((moved[1] - want).abs() < 1e-3, "{moved:?} want {want}");
    // In the overlay, 20 px is 20 px of its 0..2 axis.
    let plot = over_space.plot_rect();
    r.drag_feynman(id, "over", renderer::FeynmanHit::Vertex(0), 0.0, -20.0)
        .unwrap();
    let moved = r
        .chart_series(id)
        .unwrap()
        .iter()
        .find(|s| s.series_id == "over")
        .and_then(|s| s.feynman())
        .unwrap()
        .vertices[0]
        .at
        .unwrap();
    let want = 1.0 + 2.0 * 20.0 / f64::from(plot.height);
    assert!((moved[1] - want).abs() < 1e-3, "{moved:?} want {want}");
}

/// A scalar between two vertices, bent 60 degrees: a dashed arc.
fn bent_scalar() -> FeynmanSeries {
    let mut diagram = FeynmanSeries {
        vertices: vec![
            vertex("a", VertexRole::Internal, Some([0.2, 0.4])),
            vertex("b", VertexRole::Internal, Some([0.8, 0.4])),
        ],
        propagators: vec![Propagator::new("s", "a", "b", ParticleLine::Scalar)],
        ..FeynmanSeries::default()
    };
    diagram.propagators[0].bend = Some(60.0);
    diagram.propagators[0].color = Some(Color::from_rgb8(255, 0, 0));
    diagram
}

#[test]
fn a_bent_scalar_is_one_dashed_curve_in_every_output() {
    let mut r = renderer();
    let chart = chart();
    let list = [series("d", bent_scalar())];
    for scale in [1.0f32, 2.0] {
        // SVG: one path, cubic segments, one dash pattern along it.
        let svg = r
            .export_svg(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let paths: Vec<&str> = svg
            .lines()
            .filter(|line| line.contains("stroke=\"#ff0000\""))
            .collect();
        assert_eq!(paths.len(), 1, "{scale}x one path: {paths:?}");
        assert!(paths[0].contains("stroke-dasharray"), "{scale}x dashed");
        assert_eq!(paths[0].matches('M').count(), 1, "{scale}x one subpath");
        assert!(paths[0].matches('C').count() >= 2, "{scale}x cubics");
        // PDF: the same curve, read back as one curved stroke.
        let pdf = r
            .export_pdf(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let file = pdf_structure::read(&pdf);
        let size = (
            (WIDTH as f32 * scale).round() as u32,
            (HEIGHT as f32 * scale).round() as u32,
        );
        let curves = pdf_structure::content_parts(&file, size)
            .into_iter()
            .filter(|part| matches!(part, svg_parts::ReadPart::Curve { .. }))
            .count();
        assert!(curves >= 1, "{scale}x PDF curve");
        // The raster: dashes along the arc, ink and gaps in turn, the gaps
        // shorter than the dashes (6 px on, 4 px off).
        let image = export(&mut r, &chart, &list, scale);
        // The arc as the emit draws it, in export pixels: a move, then
        // cubics of (control, control, end) each.
        let config = chart.config().scaled(scale);
        let space = renderer::annotation::pixel_space_for_config(&config);
        let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
        let mut arc = Vec::new();
        bent_scalar()
            .emit(
                &space,
                None,
                &measure,
                &mut renderer::sink::Parts(&mut |part| {
                    if let renderer::DrawnPart::Path { points, .. } = part {
                        arc = points;
                    }
                }),
            )
            .unwrap();
        let cubics = (arc.len() - 1) / 3;
        assert!(cubics >= 2, "{scale}x arc: {} points", arc.len());
        let point = |t: f32| {
            let k = ((t * cubics as f32) as usize).min(cubics - 1);
            let u = t * cubics as f32 - k as f32;
            let [p0, c1, c2, p1] = [arc[3 * k], arc[3 * k + 1], arc[3 * k + 2], arc[3 * k + 3]];
            let w = [
                (1.0 - u).powi(3),
                3.0 * u * (1.0 - u).powi(2),
                3.0 * u * u * (1.0 - u),
                u.powi(3),
            ];
            [
                w[0] * p0[0] + w[1] * c1[0] + w[2] * c2[0] + w[3] * p1[0],
                w[0] * p0[1] + w[1] * c1[1] + w[2] * c2[1] + w[3] * p1[1],
            ]
        };
        let samples = 400;
        let inked: Vec<bool> = (0..=samples)
            .map(|k| {
                let p = point(k as f32 / samples as f32);
                let (x, y) = (p[0].round() as u32, p[1].round() as u32);
                let i = ((y * image.width + x) * 4) as usize;
                image.rgba[i + 3] > 100
            })
            .collect();
        let switches = inked.windows(2).filter(|w| w[0] != w[1]).count();
        let ink = inked.iter().filter(|i| **i).count() as f32 / inked.len() as f32;
        assert!(switches > 10, "{scale}x dashes along the arc: {switches}");
        assert!((0.45..0.8).contains(&ink), "{scale}x ink share {ink}");
    }
}

#[test]
fn the_vector_parts_are_clipped_to_each_break_piece() {
    use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark};
    let mut r = renderer();
    let mut chart = chart();
    chart.config_mut().bottom_x.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(0.45, 0.55)],
        style: AxisBreakStyle::Continuous,
        gap_px: 24.0,
        mark: BreakMark::default(),
        frame_each_panel: false,
        sizing: Default::default(),
    });
    let mut diagram = bent_scalar();
    diagram.propagators[0].bend = None;
    diagram.propagators[0].particle = ParticleLine::Photon;
    let list = [series("d", diagram)];
    for scale in [1.0f32, 2.0] {
        let svg = r
            .export_svg(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        // Once per piece, each inside its own clip.
        assert_eq!(
            svg.matches("stroke=\"#ff0000\"").count(),
            2,
            "{scale}x a photon per piece"
        );
        let mut clipped = 0;
        let mut open: Vec<bool> = Vec::new();
        for line in svg.lines() {
            if line.starts_with("<g clip-path") {
                open.push(true);
            } else if line.starts_with("<g") {
                open.push(false);
            } else if line.starts_with("</g>") {
                open.pop();
            }
            if line.contains("stroke=\"#ff0000\"") && open.contains(&true) {
                clipped += 1;
            }
        }
        assert_eq!(clipped, 2, "{scale}x both inside a clip");
    }
}

#[test]
fn a_leg_end_picks_through_the_renderer_and_drags() {
    let mut r = renderer();
    let chart = chart();
    let list = [series("d", labelled())];
    let diagram = labelled();
    // mu- (row 4) ends the line from v2 (0.25, 0.5)… at (1, 2/3): 2 px
    // back along that line from the leg end, inside the plot.
    let leg = diagram.vertices[4].at.unwrap();
    let v2 = diagram.vertices[3].at.unwrap();
    for scale in [1.0f32, 2.0] {
        let da = chart.config().data_area().unwrap().0;
        let px = |p: [f64; 2]| {
            [
                da.x as f64 + p[0] * da.width as f64,
                da.y as f64 + (1.0 - p[1]) * da.height as f64,
            ]
        };
        let (a, b) = (px(leg), px(v2));
        let d = [b[0] - a[0], b[1] - a[1]];
        let l = d[0].hypot(d[1]);
        let near_leg = (
            ((a[0] + d[0] / l * 2.0) * scale as f64).round() as u32,
            ((a[1] + d[1] / l * 2.0) * scale as f64).round() as u32,
        );
        let hit = pick(&mut r, &chart, &list, scale, near_leg);
        assert!(
            matches!(
                hit,
                Some(renderer::PickedData::FeynmanVertex {
                    vertex_index: 4,
                    ..
                })
            ),
            "{scale}x: {hit:?}"
        );
    }
    let id = r
        .register_chart(chart.config().clone(), list.to_vec())
        .unwrap();
    let height = chart.config().data_area().unwrap().0.height as f64;
    assert_eq!(
        r.drag_feynman(id, "d", renderer::FeynmanHit::Vertex(4), 0.0, -10.0)
            .unwrap(),
        renderer::layout::NudgeResult::Moved
    );
    let moved = r.chart_series(id).unwrap()[0].feynman().unwrap().vertices[4]
        .at
        .unwrap();
    assert!(
        (moved[1] - (leg[1] + 10.0 / height)).abs() < 1e-3 && (moved[0] - leg[0]).abs() < 1e-6,
        "{moved:?}"
    );
}

#[test]
fn a_diagram_zoomed_a_million_times_draws_and_picks_at_1x_and_2x() {
    // One straight line of each kind, each crossing the view: a photon, a
    // gluon, a dashed scalar and a fermion, 0.2..0.8 in x, 1e-7 apart in y.
    let particles = [
        ParticleLine::Photon,
        ParticleLine::Gluon,
        ParticleLine::Scalar,
        ParticleLine::Fermion,
    ];
    let mut diagram = FeynmanSeries::default();
    for (k, particle) in particles.into_iter().enumerate() {
        let y = 0.5 - 1.5e-7 + 1e-7 * k as f64;
        diagram.vertices.push(vertex(
            &format!("a{k}"),
            VertexRole::Internal,
            Some([0.2, y]),
        ));
        diagram.vertices.push(vertex(
            &format!("b{k}"),
            VertexRole::Internal,
            Some([0.8, y]),
        ));
        diagram.propagators.push(Propagator::new(
            format!("p{k}"),
            format!("a{k}"),
            format!("b{k}"),
            particle,
        ));
    }
    let list = [series("d", diagram)];
    let mut r = renderer();
    let mut chart = chart();
    chart.set_x_range(0.5 - 5e-7, 0.5 + 5e-7);
    chart.set_y_range(0.5 - 5e-7, 0.5 + 5e-7);
    for scale in [1.0f32, 2.0] {
        let started = std::time::Instant::now();
        let image = export(&mut r, &chart, &list, scale);
        let drawn = started.elapsed();
        assert!(drawn.as_secs_f64() < 30.0, "{scale}x drew in {drawn:?}");
        for k in 0..particles.len() {
            let y = 0.5 - 1.5e-7 + 1e-7 * k as f64;
            let (x, py) = at(&chart, 0.5, (y - (0.5 - 5e-7)) / 1e-6, scale);
            // The data-unit helper maps 0..1: rescale to the zoomed axis.
            assert!(
                near(&image, (x, py), (6.0 * scale) as u32),
                "{scale}x line {k} at the middle"
            );
            let hit = pick(&mut r, &chart, &list, scale, (x, py));
            assert!(
                matches!(
                    hit,
                    Some(renderer::PickedData::FeynmanPropagator { propagator_index, .. })
                        if propagator_index == k
                ),
                "{scale}x line {k}: {hit:?}"
            );
        }
    }
}
