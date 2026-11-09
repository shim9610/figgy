use super::*;
use crate::annotation::{AxisMap, AxisSpace, PixelSpace};
use crate::config::{Config, PickedDataRef};
use crate::data_config::DataRenderType;
use crate::layout::BreakMap;
use crate::sink::{DrawnPart, Parts};
use crate::text::TextExtents;

struct Fixed;
impl MeasureText for Fixed {
    fn measure_rich(&self, rt: &RichText) -> TextExtents {
        TextExtents {
            width: rt.segments.len() as f32 * 8.0,
            ascent: 10.0,
            descent: 3.0,
        }
    }
}

/// x and y over 0..1 on a 200 × 100 px plot at (0, 0), y up.
fn space() -> PixelSpace {
    let axis = AxisSpace {
        map: AxisMap::Linear { min: 0.0, max: 1.0 },
        inverted: false,
        breaks: BreakMap::IDENTITY,
    };
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 100.0,
    };
    PixelSpace {
        panel: rect,
        plot: rect,
        x: axis,
        y: axis,
    }
}

fn vertex(id: &str, role: VertexRole) -> FeynmanVertex {
    FeynmanVertex::new(id, role)
}

fn line(id: &str, from: &str, to: &str, particle: ParticleLine) -> Propagator {
    Propagator::new(id, from, to, particle)
}

/// Bhabha's s channel: e⁻ e⁺ → γ* → μ⁻ μ⁺.
fn s_channel() -> FeynmanSeries {
    FeynmanSeries {
        vertices: vec![
            vertex("e-", VertexRole::Incoming),
            vertex("e+", VertexRole::Incoming),
            vertex("v1", VertexRole::Internal),
            vertex("v2", VertexRole::Internal),
            vertex("mu-", VertexRole::Outgoing),
            vertex("mu+", VertexRole::Outgoing),
        ],
        propagators: vec![
            line("p1", "e-", "v1", ParticleLine::Fermion),
            line("p2", "v1", "e+", ParticleLine::Fermion),
            line("g", "v1", "v2", ParticleLine::Photon),
            line("p3", "v2", "mu-", ParticleLine::Fermion),
            line("p4", "mu+", "v2", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    }
}

/// The span of a whole path.
fn whole() -> Spans {
    let mut spans = Spans::NONE;
    spans.push(0.0, 1.0);
    spans
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

#[test]
fn the_s_channel_places_its_legs_on_the_edges_and_its_vertices_at_quarters() {
    let positions = s_channel().positions().unwrap();
    assert!(near(positions[0], [0.0, 2.0 / 3.0]), "{:?}", positions[0]);
    assert!(near(positions[1], [0.0, 1.0 / 3.0]), "{:?}", positions[1]);
    assert!(near(positions[2], [0.25, 0.5]), "{:?}", positions[2]);
    assert!(near(positions[3], [0.75, 0.5]), "{:?}", positions[3]);
    assert!(near(positions[4], [1.0, 2.0 / 3.0]), "{:?}", positions[4]);
    assert!(near(positions[5], [1.0, 1.0 / 3.0]), "{:?}", positions[5]);
}

#[test]
fn the_layout_is_deterministic_and_a_commit_writes_only_the_unplaced() {
    let mut diagram = s_channel();
    diagram.vertices[2].at = Some([0.3, 0.9]);
    let first = diagram.positions().unwrap();
    assert_eq!(first, diagram.positions().unwrap(), "bit-identical");
    assert_eq!(first[2], [0.3, 0.9], "a pinned vertex stays");
    assert!(diagram.auto_layout().unwrap());
    for (vertex, p) in diagram.vertices.iter().zip(&first) {
        assert_eq!(vertex.at, Some(*p));
    }
    assert_eq!(
        diagram.positions().unwrap(),
        first,
        "the committed layout draws the same"
    );
    assert!(!diagram.auto_layout().unwrap(), "nothing left to place");
}

#[test]
fn bottom_to_top_puts_the_legs_at_the_bottom_and_the_top() {
    let mut diagram = s_channel();
    diagram.layout.direction = TimeDirection::BottomToTop;
    let positions = diagram.positions().unwrap();
    assert_eq!(positions[0][1], 0.0);
    assert_eq!(positions[4][1], 1.0);
    assert!(near(positions[2], [0.5, 0.25]), "{:?}", positions[2]);
}

/// Whether the open segments `a`–`b` and `c`–`d` cross.
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let side = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let (d1, d2) = (side(a, b, c), side(a, b, d));
    let (d3, d4) = (side(c, d, a), side(c, d, b));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

#[test]
fn a_planar_box_diagram_is_drawn_without_crossings() {
    let diagram = FeynmanSeries {
        vertices: vec![
            vertex("in1", VertexRole::Incoming),
            vertex("in2", VertexRole::Incoming),
            vertex("v1", VertexRole::Internal),
            vertex("v2", VertexRole::Internal),
            vertex("v3", VertexRole::Internal),
            vertex("v4", VertexRole::Internal),
            vertex("out1", VertexRole::Outgoing),
            vertex("out2", VertexRole::Outgoing),
        ],
        propagators: vec![
            line("a", "in1", "v1", ParticleLine::Fermion),
            line("b", "in2", "v2", ParticleLine::Fermion),
            line("c", "v1", "v3", ParticleLine::Fermion),
            line("d", "v2", "v4", ParticleLine::Fermion),
            line("e", "v1", "v2", ParticleLine::Photon),
            line("f", "v3", "v4", ParticleLine::Photon),
            line("g", "v3", "out1", ParticleLine::Fermion),
            line("h", "v4", "out2", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    };
    let p = diagram.positions().unwrap();
    let row = |id: &str| diagram.vertex_row(id).unwrap();
    let segments: Vec<([f64; 2], [f64; 2])> = diagram
        .propagators
        .iter()
        .map(|l| (p[row(&l.from)], p[row(&l.to)]))
        .collect();
    for (i, s) in segments.iter().enumerate() {
        for t in &segments[i + 1..] {
            assert!(!cross(s.0, s.1, t.0, t.1), "{s:?} crosses {t:?}");
        }
    }
}

#[test]
fn a_vacuum_bubble_is_placed_on_a_circle() {
    let diagram = FeynmanSeries {
        vertices: vec![
            vertex("a", VertexRole::Internal),
            vertex("b", VertexRole::Internal),
        ],
        propagators: vec![
            line("x", "a", "b", ParticleLine::Fermion),
            line("y", "b", "a", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    };
    let p = diagram.positions().unwrap();
    let distance = ((p[0][0] - p[1][0]).powi(2) + (p[0][1] - p[1][1]).powi(2)).sqrt();
    assert!((distance - 0.5).abs() < 1e-9, "{p:?}");
}

#[test]
fn vertices_that_land_on_one_point_are_spread() {
    let diagram = FeynmanSeries {
        vertices: vec![
            vertex("in", VertexRole::Incoming),
            vertex("a", VertexRole::Internal),
            vertex("b", VertexRole::Internal),
            vertex("out", VertexRole::Outgoing),
        ],
        propagators: vec![
            line("1", "in", "a", ParticleLine::Plain),
            line("2", "a", "out", ParticleLine::Plain),
            line("3", "in", "b", ParticleLine::Plain),
            line("4", "b", "out", ParticleLine::Plain),
        ],
        ..FeynmanSeries::default()
    };
    let p = diagram.positions().unwrap();
    assert_eq!(p[1], [0.5, 0.5]);
    assert!(near(p[2], [0.5, 0.6]), "{:?}", p[2]);
}

#[test]
fn validation_refuses_broken_diagrams() {
    assert_eq!(s_channel().validate(), Ok(()));
    let mut duplicate = s_channel();
    duplicate.vertices[1].id = "e-".into();
    assert!(duplicate.validate().is_err());
    let mut dangling = s_channel();
    dangling.propagators[0].to = "nowhere".into();
    assert!(dangling.validate().is_err());
    let mut infinite = s_channel();
    infinite.vertices[0].at = Some([f64::NAN, 0.0]);
    assert!(infinite.validate().is_err());
    let mut bent = s_channel();
    bent.propagators[2].bend = Some(175.0);
    assert!(bent.validate().is_err());
    let mut style = s_channel();
    style.style.wave.wavelength = 0.5;
    assert!(style.validate().is_err());

    let series = SeriesConfig {
        series_id: "d".into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::Feynman(s_channel()),
    };
    let config = crate::defaults::config();
    assert_eq!(
        config.validate_feynman(std::slice::from_ref(&series)),
        Ok(())
    );
    let mut columns = series.clone();
    columns.x_column = "x".into();
    assert!(config.validate_feynman(&[columns]).is_err());
    let mut categorical = config.clone();
    categorical.bottom_x.categories = Some(vec![crate::config::Category::new("a", "A")]);
    assert!(categorical.validate_feynman(&[series]).is_err());
}

#[test]
fn the_emit_draws_every_particle_and_marker() {
    let mut diagram = s_channel();
    diagram
        .propagators
        .push(line("glue", "v1", "v2", ParticleLine::Gluon));
    diagram
        .propagators
        .push(line("loop", "v2", "v2", ParticleLine::Scalar));
    diagram.vertices[2].marker = Some(VertexMarker::CircledCross);
    let mut parts = Vec::new();
    diagram
        .emit(
            &space(),
            None,
            &Fixed,
            &mut Parts(&mut |part| parts.push(part)),
        )
        .unwrap();
    let count = |f: fn(&DrawnPart) -> bool| parts.iter().filter(|p| f(p)).count();
    // Four fermions (straight segments) with an arrowhead each, the
    // photon, the gluon, the loop and the circled cross's ring as paths.
    assert_eq!(count(|p| matches!(p, DrawnPart::Triangle(_))), 4);
    assert_eq!(count(|p| matches!(p, DrawnPart::Path { .. })), 4);
    // The cross's two strokes, the four fermion lines; the dot at v2.
    assert_eq!(count(|p| matches!(p, DrawnPart::Segment { .. })), 6);
    assert_eq!(count(|p| matches!(p, DrawnPart::Polygon { .. })), 1);
}

#[test]
fn a_wave_spans_whole_half_periods_from_end_to_end() {
    let base = Base::Line {
        a: [0.0, 50.0],
        b: [100.0, 50.0],
    };
    let mut path = Vec::new();
    wave_path(
        &base,
        Wiggle {
            amplitude: 4.0,
            wavelength: 12.0,
        },
        0.0,
        &whole(),
        false,
        &mut path,
    )
    .unwrap();
    // 100 px / 6 px half periods: 17 half periods, sampled six times each.
    assert_eq!(path.len(), 1 + 17 * WAVE_SAMPLES);
    assert_eq!(path[0], PathSegment::Move((0.0, 50.0)));
    let PathSegment::Cubic(_, _, end) = path[path.len() - 1] else {
        panic!("a cubic");
    };
    assert!(
        (end.0 - 100.0).abs() < 1e-3 && (end.1 - 50.0).abs() < 1e-3,
        "{end:?}"
    );
}

#[test]
fn parallel_lines_fan_and_an_explicit_bend_arcs() {
    let mut diagram = FeynmanSeries {
        vertices: vec![
            vertex("a", VertexRole::Internal),
            vertex("b", VertexRole::Internal),
        ],
        propagators: vec![
            line("x", "a", "b", ParticleLine::Fermion),
            line("y", "b", "a", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    };
    diagram.vertices[0].at = Some([0.25, 0.5]);
    diagram.vertices[1].at = Some([0.75, 0.5]);
    let space = space();
    let placement = diagram.placed(&space, None).unwrap();
    let middle = |index: usize| placement.geometry(index).unwrap().base.point(0.5);
    // The two lines bulge to opposite sides of the chord at y 50 px.
    let (m0, m1) = (middle(0), middle(1));
    assert!((m0[1] - 50.0) * (m1[1] - 50.0) < 0.0, "{m0:?} {m1:?}");
    // A 90-degree bend from left to right bulges up by half the chord.
    drop(placement);
    diagram.propagators.truncate(1);
    diagram.propagators[0].bend = Some(90.0);
    let m = diagram
        .placed(&space, None)
        .unwrap()
        .geometry(0)
        .unwrap()
        .base
        .point(0.5);
    assert!(
        (m[0] - 100.0).abs() < 1e-3 && (m[1] - 0.0).abs() < 1e-3,
        "{m:?}"
    );
}

#[test]
fn any_number_of_parallel_lines_fan_apart_within_the_widest_bend() {
    for count in [2usize, 5, 6, 12, 40] {
        let mut diagram = FeynmanSeries {
            vertices: vec![
                vertex("a", VertexRole::Internal),
                vertex("b", VertexRole::Internal),
            ],
            propagators: (0..count)
                .map(|k| line(&format!("p{k}"), "a", "b", ParticleLine::Photon))
                .collect(),
            ..FeynmanSeries::default()
        };
        diagram.vertices[0].at = Some([0.25, 0.5]);
        diagram.vertices[1].at = Some([0.75, 0.5]);
        let space = space();
        let placement = diagram.placed(&space, None).unwrap();
        let mut bulges: Vec<f32> = (0..count)
            .map(|index| {
                let bend = placement.bend(index);
                assert!(bend.abs() <= MAX_BEND, "{count} lines: bend {bend}");
                placement.geometry(index).unwrap().base.point(0.5)[1]
            })
            .collect();
        bulges.sort_by(f32::total_cmp);
        assert!(
            bulges.windows(2).all(|w| w[1] - w[0] > 1e-3),
            "{count} lines overlap: {bulges:?}"
        );
        // Every line draws one bounded path.
        let mut paths = 0usize;
        drop(placement);
        diagram
            .emit(
                &space,
                None,
                &Fixed,
                &mut Parts(&mut |part| {
                    if let DrawnPart::Path { points, .. } = part {
                        // At 170 degrees an arc is about 17 chords long
                        // (100 px here): 6 cubics per 6 px half period.
                        assert!(points.len() < 3 * 17 * 100 + 64, "{} points", points.len());
                        paths += 1;
                    }
                }),
            )
            .unwrap();
        assert_eq!(paths, count);
    }
}

#[test]
fn a_pick_answers_the_part_under_the_cursor() {
    let mut diagram = s_channel();
    diagram.propagators[2].label = Some(RichText::plain("γ", Color::BLACK, 12.0, ""));
    let space = space();
    let placement = diagram.placed(&space, None).unwrap();
    let pick = |at: [f32; 2], reach: f32| diagram.pick(&space, &Fixed, at, reach).unwrap();
    let v1 = placement.px(2).unwrap();
    assert_eq!(pick(v1, 4.0), Some((FeynmanHit::Vertex(2), 0.0)));
    let g = placement.geometry(2).unwrap().base.point(0.3);
    assert_eq!(
        pick(g, 4.0).map(|(hit, _)| hit),
        Some(FeynmanHit::Propagator(2))
    );
    let label = placement
        .label(FeynmanLabelPart::Propagator, 2, &Fixed)
        .unwrap()
        .rect();
    let centre = [label.x + label.width / 2.0, label.y + label.height / 2.0];
    assert_eq!(
        pick(centre, 4.0).map(|(hit, _)| hit),
        Some(FeynmanHit::Label(FeynmanLabelPart::Propagator, 2))
    );
    assert_eq!(pick([199.0, 1.0], 2.0), None);
    // A leg end has no marker: within the pick distance it wins over its
    // own line, on the line and up to that far along it.
    let leg = placement.px(0).unwrap();
    let along = placement.geometry(0).unwrap().base.tangent(0.0);
    for step in [0.0, 2.0, 4.0] {
        let at = add(leg, scale(along, step));
        assert_eq!(
            pick(at, 4.0).map(|(hit, _)| hit),
            Some(FeynmanHit::Vertex(0)),
            "{step} px along"
        );
    }
    // Past the pick distance the line answers.
    let at = add(leg, scale(along, 12.0));
    assert_eq!(
        pick(at, 4.0).map(|(hit, _)| hit),
        Some(FeynmanHit::Propagator(0))
    );
}

#[test]
fn the_first_vertex_drag_commits_the_layout_then_moves_the_vertex() {
    let mut diagram = s_channel();
    let before = diagram.positions().unwrap();
    let space = space();
    assert_eq!(
        diagram.drag_vertex(2, 20.0, -10.0, &space).unwrap(),
        NudgeResult::Moved
    );
    for (i, vertex) in diagram.vertices.iter().enumerate() {
        let at = vertex.at.expect("committed");
        if i == 2 {
            // Through f32 pixels: within a millionth of the axis.
            let want = [before[2][0] + 0.1, before[2][1] + 0.1];
            assert!(
                (at[0] - want[0]).abs() < 1e-6 && (at[1] - want[1]).abs() < 1e-6,
                "{at:?}"
            );
        } else {
            assert_eq!(at, before[i], "vertex {i} stays");
        }
    }
    let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
    assert_eq!(diagram.drag_vertex(9, 1.0, 1.0, &space).unwrap(), reject);
    assert_eq!(
        diagram.drag_vertex(0, f32::NAN, 1.0, &space).unwrap(),
        reject
    );
}

#[test]
fn a_bend_drag_sets_the_arc_through_the_moved_middle() {
    let mut diagram = s_channel();
    diagram.auto_layout().unwrap();
    let space = space();
    // v1 (50, 50) to v2 (150, 50): drag the middle up 50 px, a half circle.
    assert_eq!(
        diagram.drag_bend(2, 0.0, -50.0, &space).unwrap(),
        NudgeResult::Moved
    );
    let bend = diagram.propagators[2].bend.unwrap();
    assert!((bend - 90.0).abs() < 1e-3, "{bend}");
    let mut looped = s_channel();
    looped
        .propagators
        .push(line("l", "v1", "v1", ParticleLine::Photon));
    assert_eq!(
        looped.drag_bend(5, 0.0, -5.0, &space).unwrap(),
        NudgeResult::Rejected(NudgeReject::OutOfBounds)
    );
}

#[test]
fn labels_drag_per_part() {
    let mut diagram = s_channel();
    diagram.propagators[0].momentum = Some(MomentumArrow {
        label: RichText::plain("p", Color::BLACK, 12.0, ""),
        side: MomentumSide::Right,
        reversed: false,
        label_offset: [0.0; 2],
    });
    assert_eq!(
        diagram.drag_label(FeynmanLabelPart::Vertex, 2, 3.0, 4.0),
        NudgeResult::Moved
    );
    assert_eq!(diagram.vertices[2].label_offset, [3.0, 4.0]);
    assert_eq!(
        diagram.drag_label(FeynmanLabelPart::Momentum, 0, 1.0, 1.0),
        NudgeResult::Moved
    );
    assert_eq!(
        diagram.label_offset(FeynmanLabelPart::Momentum, 0),
        Some([1.0, 1.0])
    );
    let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
    assert_eq!(
        diagram.drag_label(FeynmanLabelPart::Momentum, 1, 1.0, 1.0),
        reject
    );
    assert_eq!(
        diagram.drag_label(FeynmanLabelPart::Vertex, 0, 5000.0, 0.0),
        reject
    );
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

#[test]
fn step_selection_walks_the_rows_and_the_series() {
    let list = [series("a", s_channel()), series("b", s_channel())];
    let mut config: Config = crate::defaults::config();
    config.picked_data = Some(crate::config::DataSelectionsConfig {
        refs: vec![PickedDataRef::FeynmanVertex {
            source_id: None,
            series_id: "a".into(),
            vertex_index: 1,
        }],
        ..Default::default()
    });
    let none = |_: &str| None;
    assert_eq!(config.step_selection(&list, 2, 0, none), Ok(true));
    let selected = |c: &Config| c.picked_data.as_ref().unwrap().refs[0].clone();
    assert_eq!(
        selected(&config),
        PickedDataRef::FeynmanVertex {
            source_id: None,
            series_id: "a".into(),
            vertex_index: 3,
        }
    );
    assert_eq!(config.step_selection(&list, 99, 1, none), Ok(true));
    assert_eq!(
        selected(&config),
        PickedDataRef::FeynmanVertex {
            source_id: None,
            series_id: "b".into(),
            vertex_index: 5,
        }
    );
    config.picked_data.as_mut().unwrap().refs[0] = PickedDataRef::FeynmanPropagator {
        source_id: None,
        series_id: "b".into(),
        propagator_index: 4,
    };
    assert_eq!(config.step_selection(&list, -1, -1, none), Ok(true));
    assert_eq!(
        selected(&config),
        PickedDataRef::FeynmanPropagator {
            source_id: None,
            series_id: "a".into(),
            propagator_index: 3,
        }
    );
    config.picked_data.as_mut().unwrap().refs[0] = PickedDataRef::FeynmanLabel {
        source_id: None,
        series_id: "a".into(),
        part: FeynmanLabelPart::Vertex,
        index: 0,
    };
    assert_eq!(config.step_selection(&list, 1, 0, none), Ok(false));
}

#[cfg(feature = "serde")]
#[test]
fn a_document_round_trips_with_its_defaults_omitted() {
    let mut diagram = s_channel();
    diagram.vertices[2].at = Some([0.25, 0.5]);
    diagram.propagators[2].bend = Some(20.0);
    let json = serde_json::to_string(&series("d", diagram.clone())).unwrap();
    assert!(!json.contains("\"style\""), "{json}");
    assert!(!json.contains("\"layout\""), "{json}");
    assert!(!json.contains("\"Internal\""), "{json}");
    assert!(json.contains("\"at\":[0.25,0.5]"), "{json}");
    let back: SeriesConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.feynman(), Some(&diagram));
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    let selection = PickedDataRef::FeynmanLabel {
        source_id: None,
        series_id: "d".into(),
        part: FeynmanLabelPart::Momentum,
        index: 2,
    };
    let json = serde_json::to_string(&selection).unwrap();
    assert_eq!(
        json,
        r#"{"kind":"feynman_label","series_id":"d","part":"momentum","index":2}"#
    );
    assert_eq!(
        serde_json::from_str::<PickedDataRef>(&json).unwrap(),
        selection
    );
}

/// x and y over `min..min + span` on a 200 × 100 px plot at (0, 0).
fn zoomed(min: [f64; 2], span: f64) -> PixelSpace {
    let axis = |min: f64| AxisSpace {
        map: AxisMap::Linear {
            min,
            max: min + span,
        },
        inverted: false,
        breaks: BreakMap::IDENTITY,
    };
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 100.0,
    };
    PixelSpace {
        panel: rect,
        plot: rect,
        x: axis(min[0]),
        y: axis(min[1]),
    }
}

/// One line of each kind, each between its own two vertices from `x0` to
/// `x1`, `dy` apart in y from `y`; the gluon and the scalar slightly bent,
/// so they still cross the middle of a zoomed view.
fn every_particle_between(x0: f64, x1: f64, y: f64, dy: f64) -> FeynmanSeries {
    let particles = [
        ParticleLine::Photon,
        ParticleLine::Gluon,
        ParticleLine::Scalar,
        ParticleLine::Graviton,
        ParticleLine::DoubleLine,
        ParticleLine::Fermion,
    ];
    let mut diagram = FeynmanSeries::default();
    for (k, particle) in particles.into_iter().enumerate() {
        let (a, b) = (format!("a{k}"), format!("b{k}"));
        let y = y + dy * k as f64;
        diagram.vertices.push(FeynmanVertex {
            at: Some([x0, y]),
            ..vertex(&a, VertexRole::Internal)
        });
        diagram.vertices.push(FeynmanVertex {
            at: Some([x1, y]),
            ..vertex(&b, VertexRole::Internal)
        });
        diagram
            .propagators
            .push(line(&format!("p{k}"), &a, &b, particle));
    }
    diagram.propagators[1].bend = Some(1e-5);
    diagram.propagators[2].bend = Some(-1e-5);
    diagram
}

/// The path segments and parts an emit writes.
fn emitted(diagram: &FeynmanSeries, space: &PixelSpace) -> (usize, usize) {
    let (mut segments, mut parts) = (0usize, 0usize);
    diagram
        .emit(
            space,
            None,
            &Fixed,
            &mut Parts(&mut |part| {
                parts += 1;
                if let DrawnPart::Path { points, .. } = part {
                    segments += points.len();
                }
            }),
        )
        .unwrap();
    (segments, parts)
}

// The class "work that grows with a line's length on screen": a wave, a
// coil, a dashed line and an arc sampled or measured along their whole
// length. Each is now clipped to the plot first, so zooming in a million
// times draws and picks with the work of an ordinary view.
#[test]
fn zooming_in_a_million_times_draws_and_picks_with_bounded_work() {
    let (ordinary, _) = emitted(&every_particle_between(0.2, 0.8, 0.2, 0.1), &space());
    // A millionth of the axis about the middle: the lines run about 1.2e8
    // px across a 200 px plot, 10 px apart.
    let diagram = every_particle_between(0.2, 0.8, 0.5 - 2.5e-7, 1e-7);
    let space = zoomed([0.5 - 5e-7, 0.5 - 5e-7], 1e-6);
    let (segments, parts) = emitted(&diagram, &space);
    assert!(parts > 0, "something is drawn");
    assert!(
        segments <= 4 * ordinary.max(1000),
        "{segments} path points zoomed in, {ordinary} in the ordinary view"
    );
    let started = std::time::Instant::now();
    // Each line picks where it crosses the plot: the photon 25 px below the
    // middle, the fermion 25 px above it (y up, 10 px apart).
    for (k, y) in [(0usize, 75.0f32), (5, 25.0)] {
        let hit = diagram.pick(&space, &Fixed, [100.0, y], 4.0).unwrap();
        assert_eq!(
            hit.map(|(hit, _)| hit),
            Some(FeynmanHit::Propagator(k)),
            "line {k}"
        );
    }
    assert!(
        started.elapsed().as_secs_f64() < 1.0,
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_line_of_no_finite_length_draws_nothing_and_does_not_panic() {
    // The far end lies past f32's range on screen: its length is infinite.
    let diagram = every_particle_between(0.5, 1e37, 0.5, 0.1);
    let (segments, _) = emitted(&diagram, &space());
    assert!(segments < 100_000, "{segments}");
    assert!(diagram.pick(&space(), &Fixed, [100.0, 50.0], 4.0).is_ok());
}

#[test]
fn thousands_of_parallel_lines_each_take_their_own_arc() {
    // An even count puts the two middle lines at the smallest bends: they
    // must still part, however small.
    for count in [2_970usize, 5_000] {
        let mut diagram = FeynmanSeries {
            vertices: vec![
                vertex("a", VertexRole::Internal),
                vertex("b", VertexRole::Internal),
            ],
            propagators: (0..count)
                .map(|k| line(&format!("p{k}"), "a", "b", ParticleLine::Plain))
                .collect(),
            ..FeynmanSeries::default()
        };
        diagram.vertices[0].at = Some([0.25, 0.5]);
        diagram.vertices[1].at = Some([0.75, 0.5]);
        let space = space();
        let placed = diagram.placed(&space, None).unwrap();
        let mut bulges: Vec<f64> = (0..count)
            .map(|index| {
                let geometry = placed.geometry(index).unwrap();
                assert!(
                    matches!(geometry.base, Base::Bow { .. }),
                    "{count}: {index} bends"
                );
                // The bulge in the chord frame, in f64: y is down on screen.
                let Base::Bow { a, b, half } = geometry.base else {
                    unreachable!()
                };
                let c = f64::from(len(sub(b, a)));
                let (_, y) = Base::bow_xy(f64::from(half), 0.5);
                c * y
            })
            .collect();
        bulges.sort_by(f64::total_cmp);
        assert!(
            bulges.windows(2).all(|w| w[1] > w[0]),
            "{count} lines: two share an arc"
        );
    }
}

// The class "a vertex's pick reach beating lines": the reach is the
// request's pick distance, and it beats only the vertex's own lines.
#[test]
fn a_short_line_and_a_line_near_another_vertex_still_pick() {
    let mut diagram = FeynmanSeries {
        vertices: vec![
            vertex("a", VertexRole::Internal),
            vertex("b", VertexRole::Internal),
            vertex("c", VertexRole::Internal),
            vertex("d", VertexRole::Internal),
        ],
        propagators: vec![
            // 12 px between dots of radius 3.
            line("short", "a", "b", ParticleLine::Plain),
            // Passes 5 px under the unrelated vertex c, 2 px past its dot.
            line("long", "d", "c", ParticleLine::Plain),
        ],
        ..FeynmanSeries::default()
    };
    diagram.vertices[0].at = Some([0.5, 0.8]);
    diagram.vertices[1].at = Some([0.56, 0.8]);
    diagram.vertices[2].at = Some([0.5, 0.35]);
    diagram.vertices[3].at = Some([0.1, 0.3]);
    diagram.propagators[1].to = "c".into();
    // Re-aim the long line past c: from d to a point beyond c, 3 px low.
    diagram.vertices.push(vertex("e", VertexRole::Internal));
    diagram.vertices[4].at = Some([0.9, 0.3]);
    diagram.propagators[1].to = "e".into();
    let space = space();
    let placed = diagram.placed(&space, None).unwrap();
    let pick = |at: [f32; 2]| {
        diagram
            .pick(&space, &Fixed, at, 4.0)
            .unwrap()
            .map(|(hit, _)| hit)
    };
    let middle = placed.geometry(0).unwrap().base.point(0.5);
    assert_eq!(
        pick(middle),
        Some(FeynmanHit::Propagator(0)),
        "the short line's middle"
    );
    let under_c = add(placed.px(2).unwrap(), [0.0, 5.0]);
    assert_eq!(
        pick(under_c),
        Some(FeynmanHit::Propagator(1)),
        "the line under c"
    );
    assert_eq!(
        pick(placed.px(2).unwrap()),
        Some(FeynmanHit::Vertex(2)),
        "c itself"
    );
}

// The class "a numerical setting that depends on the data's magnitude":
// the solver works in frame units.
#[test]
fn the_layout_solves_at_any_magnitude_and_on_long_ladders() {
    for magnitude in [1e-200, 1e-6, 1.0, 1e6, 1e200] {
        let mut diagram = s_channel();
        diagram.vertices[0].at = Some([0.0, magnitude]);
        diagram.vertices[4].at = Some([4.0 * magnitude, magnitude]);
        let p = diagram.positions().unwrap();
        assert!(
            p.iter().all(|q| q[0].is_finite() && q[1].is_finite()),
            "{magnitude}: {p:?}"
        );
        // v1 sits at the mean of its neighbours e-, e+ and v2.
        let mean = [
            (p[0][0] + p[1][0] + p[3][0]) / 3.0,
            (p[0][1] + p[1][1] + p[3][1]) / 3.0,
        ];
        let scale = magnitude * 4.0;
        assert!(
            ((p[2][0] - mean[0]) / scale).abs() < 1e-9
                && ((p[2][1] - mean[1]) / scale).abs() < 1e-9,
            "{magnitude}: {:?} vs {mean:?}",
            p[2]
        );
    }
    // Long ladders: the stall rule does not stop a valid solve.
    for rungs in [10usize, 100, 1_000, 3_000] {
        let mut diagram = FeynmanSeries::default();
        for (id, role) in [
            ("ia", VertexRole::Incoming),
            ("ib", VertexRole::Incoming),
            ("oa", VertexRole::Outgoing),
            ("ob", VertexRole::Outgoing),
        ] {
            diagram.vertices.push(vertex(id, role));
        }
        for k in 0..rungs {
            diagram
                .vertices
                .push(vertex(&format!("a{k}"), VertexRole::Internal));
            diagram
                .vertices
                .push(vertex(&format!("b{k}"), VertexRole::Internal));
            diagram.propagators.push(line(
                &format!("r{k}"),
                &format!("a{k}"),
                &format!("b{k}"),
                ParticleLine::Plain,
            ));
            if k > 0 {
                for rail in ["a", "b"] {
                    diagram.propagators.push(line(
                        &format!("{rail}{k}-"),
                        &format!("{rail}{}", k - 1),
                        &format!("{rail}{k}"),
                        ParticleLine::Plain,
                    ));
                }
            }
        }
        let last = rungs - 1;
        for (id, from, to) in [
            ("la", "ia".to_string(), "a0".to_string()),
            ("lb", "ib".to_string(), "b0".to_string()),
            ("ra", format!("a{last}"), "oa".to_string()),
            ("rb", format!("b{last}"), "ob".to_string()),
        ] {
            diagram
                .propagators
                .push(line(id, &from, &to, ParticleLine::Plain));
        }
        assert!(diagram.positions().is_ok(), "{rungs} rungs");
    }
}

#[test]
fn a_feynman_label_moves_by_part_and_row_not_by_a_value_label_index() {
    // D7: a label is named by its list and its row, so adding a vertex
    // never renames a propagator's label. The value-label drag, which
    // takes one index, refuses the series; the native drag is drag_label.
    let mut s = series("d", s_channel());
    let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
    assert_eq!(s.drag_value_label(0, 6, 1.0, 1.0), reject);
    let diagram = s.feynman_mut().unwrap();
    assert_eq!(
        diagram.drag_label(FeynmanLabelPart::Propagator, 2, 3.0, 4.0),
        NudgeResult::Moved
    );
    diagram.vertices.push(vertex("extra", VertexRole::Internal));
    assert_eq!(
        diagram.label_offset(FeynmanLabelPart::Propagator, 2),
        Some([3.0, 4.0]),
        "a new vertex leaves the propagator label's identity"
    );
}

// SPAN_SLOTS (#234, item 13): straight lines and arcs of every bend,
// crossing, grazing, entering, leaving and missing a plot, at every
// direction and every scale from a pixel to a billion pixels long. Each
// path's spans fit the slots without the widening fallback
// (`Spans::overflowed`), and no path has more than the five visible pieces
// a circle can make in a rectangle.
#[test]
fn spans_never_overflow_for_lines_and_arcs_crossing_a_plot() {
    let plot = RectF {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 100.0,
    };
    let mut paths = 0usize;
    let mut most = 0usize;
    let centres = [
        [100.0f32, 50.0],
        [0.0, 0.0],
        [200.0, 100.0],
        [-30.0, 50.0],
        [100.0, 140.0],
    ];
    for centre in centres {
        for scale in [1.0f32, 30.0, 300.0, 3e4, 3e6, 1e9] {
            for k in 0..24 {
                let angle = k as f32 * std::f32::consts::TAU / 24.0;
                let d = [scale * angle.cos(), scale * angle.sin()];
                let (a, b) = (sub(centre, d), add(centre, scale_v(d, 0.7)));
                for half in [0.0f32, 1e-9, 1e-4, 0.05, 0.4, 1.0, 1.6, 2.3, 2.96] {
                    for sign in [1.0f32, -1.0] {
                        let base = if half == 0.0 {
                            Base::Line { a, b }
                        } else {
                            Base::Bow {
                                a,
                                b,
                                half: sign * half,
                            }
                        };
                        let spans = base.spans_within(plot);
                        assert!(!spans.overflowed, "{base:?}");
                        most = most.max(spans.iter().count());
                        paths += 1;
                    }
                }
            }
        }
    }
    // Loops of every size about every place.
    for centre in centres {
        for radius in [0.5f32, 18.0, 90.0, 500.0] {
            let spans = Base::Ring {
                centre,
                radius,
                start: 0.3,
            }
            .spans_within(plot);
            assert!(!spans.overflowed);
            most = most.max(spans.iter().count());
            paths += 1;
        }
    }
    assert!(most <= 5, "{most} spans in one path");
    assert!(paths > 10_000, "{paths} paths");
}

fn scale_v(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}
