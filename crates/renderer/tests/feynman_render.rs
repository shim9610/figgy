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
    diagram.auto_layout();
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
    let positions = diagram.positions();
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
