# Feynman diagrams

A Feynman diagram draws an interaction as a graph: external legs for the
incoming and outgoing particles, internal vertices, and propagators whose
stroke names the particle. `DataRenderType::Feynman` is a DataLess series
([#234](https://github.com/shim9610/figgy/issues/234)): the topology is the
data and lives in the SSOT, serialized with the document. No column is
registered; `x_column` and `y_column` stay empty.

## Input and first render

The host lists the vertices and the propagators that join them. A vertex
position is optional: a vertex without `at` is drawn where the automatic
layout puts it.

```rust,no_run
use renderer::{
    Chart, Renderer, SeriesConfig,
    data_config::DataRenderType,
    default,
    feynman::{FeynmanSeries, FeynmanVertex, ParticleLine, Propagator, VertexRole},
};

fn draw(renderer: &mut Renderer) -> Result<(), Box<dyn std::error::Error>> {
    let diagram = FeynmanSeries {
        vertices: vec![
            FeynmanVertex::new("e-", VertexRole::Incoming),
            FeynmanVertex::new("e+", VertexRole::Incoming),
            FeynmanVertex::new("v1", VertexRole::Internal),
            FeynmanVertex::new("v2", VertexRole::Internal),
            FeynmanVertex::new("mu-", VertexRole::Outgoing),
            FeynmanVertex::new("mu+", VertexRole::Outgoing),
        ],
        propagators: vec![
            Propagator::new("a", "e-", "v1", ParticleLine::Fermion),
            Propagator::new("b", "v1", "e+", ParticleLine::Fermion),
            Propagator::new("g", "v1", "v2", ParticleLine::Photon),
            Propagator::new("c", "v2", "mu-", ParticleLine::Fermion),
            Propagator::new("d", "mu+", "v2", ParticleLine::Fermion),
        ],
        ..FeynmanSeries::default()
    };
    let series = SeriesConfig {
        series_id: "s-channel".into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::Feynman(diagram),
    };
    let mut chart = Chart::new(default::default_config());
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let id = renderer.register_chart(chart.config().clone(), vec![series])?;
    // Fix the diagram where it is drawn, and read the positions back.
    renderer.auto_layout(id, "s-channel")?;
    let placed = renderer.chart_series(id)?[0].feynman().unwrap().clone();
    assert!(placed.vertices.iter().all(|v| v.at.is_some()));
    Ok(())
}
```

## Who computes what

The host computes the data: the topology, each propagator's particle, each
leg's role, every label and momentum string, and any position it wants
fixed. Nothing physical is computed: charge, momentum conservation and the
allowed couplings are the host's.

The renderer computes appearance only: the placement of unplaced vertices,
pixel positions, the stroke geometry (a wave's or a coil's phase, fitted so
that whole periods span each line), the fan of parallel lines, arrowheads
along the drawn path, and label places.

## The layout

`FeynmanSeries::positions` places every vertex without `at`, and
`auto_layout` writes those places into `at`. The layout is deterministic:
the same SSOT gives bit-identical positions.

- Placed vertices are pinned.
- Unplaced incoming legs are spaced evenly on the early edge of the frame
  and outgoing legs on the late edge, in row order (`layout.direction`:
  `LeftToRight` puts them left and right, `BottomToTop` at the bottom and
  the top).
- Every other unplaced vertex goes to the mean of its neighbours (the
  barycentric, or Tutte, embedding), solved by conjugate gradients. A
  planar diagram with its legs on the boundary is drawn without crossings.
- A group that no placed vertex or leg reaches (a vacuum bubble) is placed
  on a circle. Vertices that land on one point are spread across the time
  direction.
- The frame is the bounding box of the placed vertices, or the unit square
  when fewer than two span one.

The mean pulls long chains together: a ladder of many rungs draws its two
rails close in the middle. Pin a few vertices, or drag them apart, to shape
such a diagram.

Committing the layout is the auto-fit pattern: the renderer writes a
display parameter into the registered SSOT, and the host reads it back.
`Renderer::auto_layout(chart, series_id)` commits a registered series; a
host that owns its `SeriesConfig` calls `FeynmanSeries::auto_layout`. The
first vertex drag commits the whole layout before it moves the vertex, so
no other vertex jumps. The WASM wrapper commits the layout when a series is
registered, so `get_series` returns placed vertices.

## Lines and markers

| `ParticleLine` | Stroke |
|---|---|
| `Plain` | A solid line |
| `Fermion`, `AntiFermion` | A solid line with an arrow along it, or against it |
| `Photon` | A wave |
| `ChargedBoson` | A wave with an arrow |
| `Gluon` | A coil, on the left of travel |
| `Scalar`, `ChargedScalar` | A dashed line, with an arrow when charged |
| `Ghost` | A dotted line with an arrow |
| `Majorana` | A solid line with two arrows pointing apart |
| `Graviton` | A double wave |
| `DoubleLine` | Two parallel lines |

A propagator from a vertex to itself is a loop of `style.loop_radius`, turned
away from the vertex's neighbours. Lines between the same two vertices fan
out by `style.fan_step` degrees, unless a line sets its own `bend`. Waves,
coils and arcs are cubic Bézier paths (`Sink::path`), so a dash runs along
the whole line and the SVG and PDF keep the curves.

A vertex draws a dot when internal and nothing at a leg's end, unless its
`marker` says otherwise: `Hidden`, `Dot`, `Blob { radius_px, fill }`,
`Cross` or `CircledCross`. Labels are rich text, so TeX such as `$e^-$`
keeps its scripts. A vertex label sits away from the vertex's neighbours, a
propagator label on the side its arc bulges to, and a momentum arrow
(`MomentumArrow { label, side, reversed }`) runs beside the line.

## Interaction

| Interaction | Behaviour |
|---|---|
| Hover, click | A typed data pick answers `PickedData::FeynmanVertex`, `FeynmanPropagator` or `FeynmanLabel { part, index }` on the CPU, through the placement the raster draws with. Labels win over markers, markers over lines |
| Picked highlight | The picked vertex, line or label is drawn `style.picked_width_scale` times as wide |
| `step_selection` | `dx` steps the vertices or the propagators, `dy` the Feynman series of the chart; a label does not step |
| Drag | `Renderer::drag_feynman(chart, series_id, FeynmanDrag, dx, dy)`: `Vertex(i)` moves a vertex through the axes, `Bend(i)` sets a line's bend to the arc through its moved middle, `Label(part, i)` moves a label. The web wrapper's `press_feynman` starts the same drags |
| Picks in a break gap | Answer nothing |
| Decorations, plot area, annotations | Unchanged: the series is data in the data slot, under the annotation overlay |
| Export | PNG at any scale; SVG and PDF write the series as vector curves between the GPU data under it and the value labels over it |
| Serialization | Every field is additive; defaults are omitted |
| Streaming | A streamed display refuses the series |

There is no count limit on vertices, propagators or labels. A 5,000-rung
ladder (10,004 vertices, 15,002 propagators) lays out in about 0.5 s and
draws at 1x in about 0.8 s in the debug test profile.

## Examples and tests

The playground scene `SceneKind::Feynman` draws Bhabha scattering's s
channel and a photon self-energy with a counterterm.
`tests/feynman_render.rs` covers drawing, the layout commit, picks, drags,
the vector export and the cost; the model's `feynman` tests cover the
layout, the emit, the pick and the edits.
