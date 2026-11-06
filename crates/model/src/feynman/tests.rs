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
    let config = crate::default::default_config();
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
    // A leg end has no marker: within the grab it wins over its line, on
    // the line and a few pixels along it.
    let leg = placement.px(0).unwrap();
    let along = placement.geometry(0).unwrap().base.tangent(0.0);
    for step in [0.0, 3.0, 5.0] {
        let at = add(leg, scale(along, step));
        assert_eq!(
            pick(at, 4.0).map(|(hit, _)| hit),
            Some(FeynmanHit::Vertex(0)),
            "{step} px along"
        );
    }
    // Past the grab the line answers.
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
        diagram.drag_vertex(2, 20.0, -10.0, &space),
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
    assert_eq!(diagram.drag_vertex(9, 1.0, 1.0, &space), reject);
    assert_eq!(diagram.drag_vertex(0, f32::NAN, 1.0, &space), reject);
}

#[test]
fn a_bend_drag_sets_the_arc_through_the_moved_middle() {
    let mut diagram = s_channel();
    diagram.auto_layout().unwrap();
    let space = space();
    // v1 (50, 50) to v2 (150, 50): drag the middle up 50 px, a half circle.
    assert_eq!(diagram.drag_bend(2, 0.0, -50.0, &space), NudgeResult::Moved);
    let bend = diagram.propagators[2].bend.unwrap();
    assert!((bend - 90.0).abs() < 1e-3, "{bend}");
    let mut looped = s_channel();
    looped
        .propagators
        .push(line("l", "v1", "v1", ParticleLine::Photon));
    assert_eq!(
        looped.drag_bend(5, 0.0, -5.0, &space),
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
    let mut config: Config = crate::default::default_config();
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
