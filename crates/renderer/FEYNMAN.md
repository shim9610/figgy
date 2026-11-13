# Feynman diagrams

A Feynman diagram draws an interaction as a graph: external legs for the
incoming and outgoing particles, internal vertices, and propagators whose
stroke names the particle. `DataRenderType::Feynman` takes no columns: the
diagram (its vertices and propagators) is part of the series configuration
and is saved with the document. `x_column` and `y_column` stay empty.

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

## What the host supplies

The host supplies the topology, each propagator's particle, each leg's role,
every label and momentum string, and any position it wants fixed. Nothing
physical is checked: charge, momentum conservation and the allowed
couplings are up to the host.

The renderer places the vertices that have no position and draws the
strokes: a wave or a coil is fitted so that whole periods span each line,
parallel lines fan out, arrowheads follow the drawn path, and the labels
are placed.

## The layout

`FeynmanSeries::positions` places every vertex without `at`, and
`auto_layout` writes those places into `at`. The layout is deterministic:
the same diagram always gives the same positions.

- Placed vertices are pinned.
- Unplaced incoming legs are spaced evenly on the early edge of the frame
  and outgoing legs on the late edge, in row order (`layout.direction`:
  `LeftToRight` puts them left and right, `BottomToTop` at the bottom and
  the top).
- Every other unplaced vertex goes to the mean of its neighbours (the
  barycentric, or Tutte, embedding). If that solve does not converge, the
  renderer reports `InvalidSeriesConfig` for the series. A planar diagram
  with its legs on the boundary is drawn without crossings.
- A group that no placed vertex or leg reaches (a vacuum bubble) is placed
  on a circle. Vertices that land on one point are spread across the time
  direction.
- The frame is the bounding box of the placed vertices, or the unit square
  when fewer than two span one.

The mean pulls long chains together: a ladder of many rungs draws its two
rails close in the middle. Pin a few vertices, or drag them apart, to shape
such a diagram.

A diagram is usually drawn without axes: `Config::apply_axis_preset(AxisPreset::Hidden)`
switches off the axis lines, ticks, tick labels and grid and keeps the
layout, so the diagram's data units stay where they were.

Committing the layout works like an auto-fit: the renderer writes the
positions into the registered series, and the host reads them back
(`Renderer::chart_series`).
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
out by `style.fan_step` degrees, unless a line sets its own `bend`; a fan of
many lines narrows its step so its outer lines stay within 170 degrees, so
any number of parallel lines draws, each on its own arc. Waves,
coils and arcs are cubic Bézier curves, so a dash runs along the whole line
and SVG and PDF export keep the curves.

A vertex draws a dot when internal and nothing at a leg's end, unless its
`marker` says otherwise: `Hidden`, `Dot`, `Blob { radius_px, fill }`,
`Cross` or `CircledCross`. Labels are rich text, so TeX such as `$e^-$`
keeps its scripts. A vertex label sits away from the vertex's neighbours, a
propagator label on the side its arc bulges to, and a momentum arrow
(`MomentumArrow { label, side, reversed }`) runs beside the line.

## Interaction

| Interaction | Behaviour |
|---|---|
| Hover, click | The data pick answers `PickedData::FeynmanVertex`, `FeynmanPropagator` or `FeynmanLabel { part, index }` where the part is drawn. A vertex answers within the pick distance of its marker and there wins over its own lines (within a third of a line between the markers), so a leg end without a marker picks and a short line still picks in its middle; other parts compete by distance, a tie going to labels over markers over lines. If the layout fails, the pick returns that error |
| Picked highlight | The picked vertex or line is drawn in `style.picked.color`, `style.picked.width_extra_px` wider; a picked leg end shows a dot; a picked label is boxed in that colour |
| `step_selection` | `dx` steps the vertices or the propagators, `dy` the Feynman series of the chart; a label does not step |
| Drag | `Renderer::drag_feynman(chart, series_id, FeynmanHit, dx, dy)`: `Vertex(i)` moves a vertex through the axes, `Propagator(i)` sets a line's bend to the arc through its moved middle. A label moves in logical pixels through the value-label drag: `FeynmanSeries::drag_label` on the host's series, which `drag_feynman` refuses. The web wrapper's `press_feynman` starts the vertex and bend drags, and a label's through the value-label pointer |
| Picks in a break gap or outside the plot | Answer nothing: the series is drawn clipped to the plot |
| Decorations, plot area, annotations | As on any Cartesian chart: the series draws in the data area, under the annotations |
| Export | PNG at any scale; SVG and PDF write the series as vector curves, over the other data and under the value labels |
| Serialization | The series round-trips through serde; default values are omitted |
| Streaming | A streamed chart rejects the series |

There is no count limit on vertices, propagators or labels. A 5,000-rung
ladder (10,004 vertices, 15,002 propagators) lays out in about 0.5 s and
draws at 1x in about 0.8 s in the debug test profile. A pick of that ladder
takes about 7 ms in a release build. An uncommitted layout is recomputed on
every pick, so commit it (`auto_layout`) before hovering a large diagram.

## Drawn within the plot

Every line is clipped to the plot, enlarged by the line's own width. A wave keeps its
phase and a dashed line its dash pattern where the clip cuts them. Drawing
and picking cost depend on the plot's size, not on how long the lines are
on screen: a diagram zoomed in a million times draws and picks as fast as
an ordinary view, and a line whose length on screen is not finite draws
nothing. Positions on screen are `f32`: a vertex about 10⁹ pixels off
screen lies on a 64-pixel grid there, so the visible part of its lines can
shift by up to that much.

## Examples and tests

[feynman.rs](examples/feynman.rs) draws six processes on cards
(`gallery-feynman.png`). The playground's *Feynman diagrams* scene draws four
well-known processes: Higgs production by gluon fusion through a top loop,
the electron's anomalous magnetic moment (the one-loop vertex correction),
beta decay at the quark level through a W⁻, and a three-jet event in
electron–positron annihilation, the gluon radiated by the quark. Drag a
vertex, bend a line or move a label there.

```sh
cargo test --locked -p figgy-renderer --test feynman_render
```
