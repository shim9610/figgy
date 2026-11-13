# Chart examples

Run these commands from the workspace root with the pinned Rust toolchain.
Headless examples need a working GPU adapter, but no window or browser.
They use the default adapter selection and do not force a software driver. On a
machine without a GPU, install a software Vulkan driver such as Mesa lavapipe and
select it as described in
[the gallery notes](../GALLERY.md#generate-pngs-and-animation-frames). Each
headless example takes an output path as its first argument and writes PNG files
there.

- [Gallery images](#gallery-images): every image in the README, with the
  function that draws it.
- [All headless examples](#all-headless-examples): what each one writes,
  including outputs that are not in the README.
- [Interactive examples](#interactive-examples): native windows and live labs.
- [Review and measurement tools](#review-and-measurement-tools).

## Gallery images

Every README image is a file in
[`assets/`](https://github.com/shim9610/figgy/tree/master/crates/renderer/assets). The "Drawn by" column
names the example and the function that draws the image. Examples that build a
list of cases in `main` are cited by case name; the case name is also the output
file name. To refresh an image, run the example's command from
[the next table](#all-headless-examples), inspect the output and copy it into
`assets/` under the asset name.

### Cartesian charts

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-line.png" width="160" alt="Sine curve"> | `gallery-line.png` | [line.rs](line.rs), `main` | The complete minimal setup: two columns, one line series, one PNG |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-errorbars.png" width="160" alt="Response curves with error bars"> | `gallery-errorbars.png` | [error_bars.rs](error_bars.rs), `render` | Lines, markers and symmetric error bars from X, Y and error columns |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-histogram.png" width="160" alt="Vertical histogram"> | `gallery-histogram.png` | [histogram.rs](histogram.rs), `render` | 40 supplied bins and their counts |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-histogram-horizontal.png" width="160" alt="Horizontal histogram"> | `gallery-histogram-horizontal.png` | [histogram.rs](histogram.rs), `render` | The same bins with horizontal bars |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-contours.png" width="160" alt="Heatmap with labelled contours"> | `gallery-contours.png` | [heatmap_contours.rs](heatmap_contours.rs), `contours` | An interpolated matrix fill, labelled isolines with line gaps, a colourbar |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-wave.gif" width="160" alt="Travelling waves"> | `gallery-wave.gif` | [animated_lines.rs](animated_lines.rs), `render` | Columns replaced every frame; writes `wave/NNN.png` frames, encoded as described in [the gallery notes](../GALLERY.md#encode-the-gifs) |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-orbit.gif" width="160" alt="Lissajous orbit"> | `gallery-orbit.gif` | [animated_lines.rs](animated_lines.rs), `render` | A moving trail and a highlighted point; writes `orbit/NNN.png` frames |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-polar.png" width="160" alt="Antenna pattern in polar coordinates"> | `gallery-polar.png` | [polar_plot.rs](polar_plot.rs), case `01-antenna-pattern` | A line filled to the centre in polar coordinates, degree labels |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-polar-compass.png" width="160" alt="Compass rose"> | `gallery-polar-compass.png` | [polar_plot.rs](polar_plot.rs), case `02-compass-rose` | Clockwise from the top, arc segments, radius labels on the 45° ray |

### Cartesian features

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-wind.png" width="160" alt="Wind field over a heatmap"> | `gallery-wind.png` | [vector_field.rs](vector_field.rs), `wind` | Grid vectors from two matrices over a speed heatmap |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-currents.png" width="160" alt="Surface currents"> | `gallery-currents.png` | [vector_field.rs](vector_field.rs), `currents` | Point vectors coloured by a speed column, scaled with `fit_scale` |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-colormap.png" width="160" alt="Colour-mapped particles"> | `gallery-colormap.png` | [value_maps.rs](value_maps.rs), `colormap_scatter` | `point_color_map`: points coloured by an age column through the colourbar |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-bubbles.png" width="160" alt="Bubble chart"> | `gallery-bubbles.png` | [value_maps.rs](value_maps.rs), `bubbles` | `point_size_map`: marker area follows a population column |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-forecast.png" width="160" alt="Forecast bands"> | `gallery-forecast.png` | [area_step.rs](area_step.rs), `forecast` | `LineFill::ToSeries` bands; NaN samples break each line |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-thermostat.png" width="160" alt="Step schedule"> | `gallery-thermostat.png` | [area_step.rs](area_step.rs), `thermostat` | A `StepPost` line filled to a baseline |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-twin-axes.png" width="160" alt="Twin Y axes"> | `gallery-twin-axes.png` | [overlay_axes.rs](overlay_axes.rs), `twin_axes` | An overlay layer with its own right Y axis, exported as a document |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-subplots.png" width="160" alt="Small multiples"> | `gallery-subplots.png` | [subplots.rs](subplots.rs), `small_multiples` | A 2 × 2 grid: the parent and three subplots linked to its X axis, each with its own Y range |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-probability.png" width="160" alt="Probability paper"> | `gallery-probability.png` | [probability_paper.rs](probability_paper.rs), `probability_paper` | Normal and Weibull probability axes, the Weibull panel over a log time axis |

### Charts new in 0.19

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-ternary.png" width="160" alt="Soil texture triangle with three cores"> | `gallery-ternary.png` | [ternary.rs](ternary.rs), `render` | The USDA soil texture triangle: twelve texture classes as filled region polygons named by text annotations, and three soil cores sampled every 10 cm as lines whose markers grow with depth (`point_size_map`) |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-surface.png" width="160" alt="3D terrain surface with a trail"> | `gallery-surface.png` | [surface_3d.rs](surface_3d.rs), `terrain` | Two summits and a saddle as a lit, wireframed 3D surface on a 61 × 61 grid (`view_3d`), floor contours in the ramp's colours, and a trail with waypoints as 3D line and marker series with a `z_column` |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-quiver3d.png" width="160" alt="3D vector field of a vortex"> | `gallery-quiver3d.png` | [surface_3d.rs](surface_3d.rs), `vortex` | A Burgers vortex sampled on a 9 × 9 × 3 lattice: one arrow per sample along (u, v, w), coloured by a host speed column through the colourbar |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-trimesh.png" width="160" alt="Stress around a hole on a triangle mesh"> | `gallery-trimesh.png` | [trimesh.rs](trimesh.rs), `plate` | Von Mises stress around a hole in a plate under tension (Kirsch's solution) on the host's graded mesh: interpolated fill, contour lines and the mesh edges as a faint line series |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-trisurf.png" width="160" alt="Drum vibration mode as a 3D triangle surface"> | `gallery-trisurf.png` | [trimesh.rs](trimesh.rs), `drum` | The (2, 2) mode of a circular drum on a disc mesh, a lit and wireframed 3D triangle surface on a diverging ramp, with floor contours |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-hexbin.png" width="160" alt="Hexbin of taxi pickups"> | `gallery-hexbin.png` | [heatmap_contours.rs](heatmap_contours.rs), `hexbin` | 100,000 taxi pickups the host bins into counts on a hexagonal lattice (`GridLayout::Hexagonal`), on a log colourbar; empty cells are NaN |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-imshow.png" width="160" alt="Planetary nebula as an RGB image"> | `gallery-imshow.png` | [heatmap_contours.rs](heatmap_contours.rs), `imshow` | A planetary nebula as one `RgbImage`: three host-composited channel grids on one lattice of square pixels, interpolated, with field stars |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-polar-heatmap.png" width="160" alt="Directional wave spectrum on polar axes"> | `gallery-polar-heatmap.png` | [polar_plot.rs](polar_plot.rs), `wave_spectrum` | A directional wave spectrum as annular cells: direction on a compass axis, frequency outward, energy in dB, with labelled polar contours |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-rose.png" width="160" alt="Wind rose"> | `gallery-rose.png` | [polar_plot.rs](polar_plot.rs), `wind_rose` | A wind rose: a year's hours from 16 directions as category bars stacked outward in four speed bands |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-sunburst.png" width="160" alt="Sunburst of electricity generation"> | `gallery-sunburst.png` | [radial_gallery.rs](radial_gallery.rs), `sunburst` | A sunburst of electricity generation: groups at the centre, sources on the middle ring and technologies outside, each share relative to its parent |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-parallel.png" width="160" alt="Parallel coordinates of a hyperparameter sweep"> | `gallery-parallel.png` | [parallel.rs](parallel.rs), `render` | A 120-run hyperparameter sweep across six axes (two of them logarithmic), each run coloured by its validation accuracy through the colourbar, the best drawn last |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-violin.png" width="160" alt="Split violins of reaction times"> | `gallery-violin.png` | [boxplot_gallery.rs](boxplot_gallery.rs), `violins` | Split violins: reaction times under four memory loads, rested on the left half and after 24 h awake on the right; each half is the host's density curve as a ragged column |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-raincloud.png" width="160" alt="Raincloud plot of commute times"> | `gallery-raincloud.png` | [boxplot_gallery.rs](boxplot_gallery.rs), `raincloud` | Horizontal rainclouds of commute time by mode: the host's half violin above a quartile box, the raw samples jittered below |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-waterfall.png" width="160" alt="Profit bridge waterfall"> | `gallery-waterfall.png` | [range_bars.rs](range_bars.rs), `waterfall` | A profit bridge: the host's running totals as start and end columns, increases and decreases coloured by direction, subtotal and totals flagged, connectors between the steps |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-candlestick.png" width="160" alt="Candlestick chart with a moving average"> | `gallery-candlestick.png` | [range_bars.rs](range_bars.rs), `candlestick` | 64 days of synthetic prices on a date axis: open–close bodies, low–high wicks and the host's 20-day moving average |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-gantt.png" width="160" alt="Gantt chart of a release plan"> | `gallery-gantt.png` | [range_bars.rs](range_bars.rs), `gantt` | A release plan on six rows over a date axis: a state column colours each task and a progress column shades its finished part |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-shapes.png" width="160" alt="Flow cytometry gates as shape annotations"> | `gallery-shapes.png` | [overlay_axes.rs](overlay_axes.rs), `shapes` | 4000 flow cytometry events with shape annotations as gates: an ellipse, a rectangle, a polygon and a dashed outline, labelled in text boxes |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-stem.png" width="160" alt="Stem plot of a filter impulse response"> | `gallery-stem.png` | [stem.rs](stem.rs), `impulse_response` | The 41 taps of a windowed-sinc low-pass filter as stems from zero, over the ideal sinc they approximate |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-lollipop.png" width="160" alt="Ranked lollipop chart"> | `gallery-lollipop.png` | [stem.rs](stem.rs), `lollipop` | A ranked lollipop chart of tree canopy change in twelve districts: horizontal stems from zero on a categorical axis |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-broken-axes.png" width="160" alt="Gamma spectrum on a broken axis"> | `gallery-broken-axes.png` | [broken_axes.rs](broken_axes.rs), `gamma_spectrum` | A gamma-ray spectrum of Cs-137 and Co-60 with the energy axis cut between 760 and 1080 keV, so both peak groups keep a readable scale |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-grotrian.png" width="160" alt="Grotrian diagram of He I"> | `gallery-grotrian.png` | [nist_grotrian_gallery.rs](nist_grotrian_gallery.rs), case `he-i` (`he-i-1x.png`; the diagram is `helium` in [support/grotrian_sets.rs](support/grotrian_sets.rs)) | A Grotrian diagram of He I from NIST data: singlet and triplet term columns, transitions labelled with their wavelengths to one decimal, the unit once in a corner note, the ground state below a broken energy axis |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-feynman.png" width="160" alt="Six Feynman diagrams"> | `gallery-feynman.png` | [feynman.rs](feynman.rs), `render` | Six processes on cards: annihilation, Compton scattering, gluon fusion through a top loop, beta decay through a W, the electron's one-loop vertex and a three-jet event |

### Radial chart

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-donut.png" width="160" alt="Ceramic donut"> | `gallery-donut.png` | [radial_gallery.rs](radial_gallery.rs), case `ceramic` | Named values, rounded edges, an oblique view |

### Category bars, box plots and radar series

These three examples open legacy documents through the deprecated readers
(`Config::from_categorical`, `Config::from_boxplot`, `Config::from_radar`),
upload the borrowed columns and export the Cartesian or polar chart they read
into. The readers are removed in the next milestone.

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-categorical.png" width="160" alt="Grouped bars"> | `gallery-categorical.png` | [categorical_gallery.rs](categorical_gallery.rs), case `grouped-matte` | Two series across four categories, a value label past every bar |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-boxplot.png" width="160" alt="Box plot"> | `gallery-boxplot.png` | [boxplot_gallery.rs](boxplot_gallery.rs), case `01-classic-matte` | Supplied five-number summaries and outliers, the median above every box |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-radar.png" width="160" alt="Radar chart"> | `gallery-radar.png` | [radar_gallery.rs](radar_gallery.rs), case `01-spider-web` | Two series over six categories on a polygon grid |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-radar-circle.png" width="160" alt="Radar chart with value labels"> | `gallery-radar-circle.png` | [radar_gallery.rs](radar_gallery.rs), case `02-circle-values` | A circular grid with a value label at every vertex |

### Draw styles

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-precise.png" width="160" alt="Precise style"> | `style-growth-response-precise.png` | [precise_sketch_preview.rs](precise_sketch_preview.rs), `main` | `DrawStyle::Precise` |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-sketch.png" width="160" alt="Sketch style"> | `style-growth-response-sketch.png` | [precise_sketch_preview.rs](precise_sketch_preview.rs), `main` | `DrawStyle::Sketch` on the same data |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-milkyway.png" width="160" alt="Milkyway style"> | `style-growth-response-milkyway.png` | [milkyway_line_preview.rs](milkyway_line_preview.rs), `main` | Milkyway on `Line` series: stars and nebula only |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-constellation.png" width="160" alt="Constellation style"> | `style-growth-response-constellation.png` | [constellation_preview.rs](constellation_preview.rs), `main` | Constellation on `ScatterLine` series: stars at the source samples |

### Visual concepts

Each function in [gallery_concepts.rs](gallery_concepts.rs) holds its data and
applies a built-in theme of its chart's kind (`Theme::named(kind, name)`, or
`RadialTheme::named(name)` for the radial charts): `midnight` and `editorial`
of the line kind, `revenue` of the category-bar kind, `topographic` of the
field kind, and the radial `pop-art` and `portfolio`.

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-observability.png" width="160" alt="Dark dashboard"> | `concept-observability.png` | `observability` | A dark service-monitoring card |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-revenue.png" width="160" alt="Revenue stacks"> | `concept-revenue.png` | `revenue` | Rounded horizontal stacked bars with inside labels |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-editorial.png" width="160" alt="Editorial sketch"> | `concept-editorial.png` | `editorial` | A warm paper-coloured Sketch figure |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-pop-art.png" width="160" alt="Toon pie"> | `concept-pop-art.png` | `radial_concepts` | A toon-shaded pie with outlines |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-portfolio.png" width="160" alt="Pearl donut"> | `concept-portfolio.png` | `radial_concepts` | A pearl donut from the same slice model |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-terrain.png" width="160" alt="Terrain"> | `concept-terrain.png` | `terrain` | A custom colour ramp under fine isolines |

## All headless examples

| Example | Writes | Command |
|---|---|---|
| [line.rs](line.rs) | One PNG at the given path | `cargo run --locked -p figgy-renderer --example line -- target/line.png` |
| [error_bars.rs](error_bars.rs) | `gallery-errorbars.png` | `cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars` |
| [histogram.rs](histogram.rs) | `gallery-histogram.png`, `gallery-histogram-horizontal.png` | `cargo run --locked -p figgy-renderer --example histogram -- target/histogram` |
| [heatmap_contours.rs](heatmap_contours.rs) | `gallery-contours.png`, `gallery-hexbin.png`, `gallery-imshow.png` | `cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours` |
| [animated_lines.rs](animated_lines.rs) | `wave/NNN.png`, `orbit/NNN.png`; the second argument is the frame count | `cargo run --locked -p figgy-renderer --example animated_lines -- target/animation 48` |
| [readme_gallery.rs](readme_gallery.rs) | The outputs of `error_bars`, `histogram`, `heatmap_contours`, `animated_lines`, `ternary`, `surface_3d`, `trimesh`, `parallel`, `range_bars`, `stem`, `broken_axes` and `feynman` from one shared device | `cargo run --locked -p figgy-renderer --example readme_gallery -- target/readme-gallery` |
| [polar_plot.rs](polar_plot.rs) | `01-antenna-pattern.png`, `02-compass-rose.png`, `03-spiral-arcs.png` (a dashed spiral with arc segments), `04-sector-radians.png` (a half-circle sector in radians), `05-gain-heatmap.png` (an antenna's gain as a heatmap with labelled contours), `gallery-polar-heatmap.png`, `gallery-rose.png` | `cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot` |
| [vector_field.rs](vector_field.rs) | `gallery-wind.png`, `gallery-currents.png`, and `gradient.png` from `gradient`: grid arrows in data units from their tails, coloured by a third matrix (`VectorColorMap::Grid`) | `cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field` |
| [value_maps.rs](value_maps.rs) | `gallery-colormap.png`, `gallery-bubbles.png` | `cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps` |
| [area_step.rs](area_step.rs) | `gallery-forecast.png`, `gallery-thermostat.png`, and `steps.png` from `step_modes`: `StepPre`, `StepPost` and `StepMid` on the same samples | `cargo run --locked -p figgy-renderer --example area_step -- target/area-step` |
| [overlay_axes.rs](overlay_axes.rs) | `gallery-twin-axes.png`, `gallery-shapes.png`; also checks the overlay's hit-test ids | `cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes` |
| [subplots.rs](subplots.rs) | `gallery-subplots.png`, and `mixed-axes.png` from `mixed_axes`: one linked axis, a different X column, and a twin Y axis inside a subplot | `cargo run --locked -p figgy-renderer --example subplots -- target/subplots` |
| [probability_paper.rs](probability_paper.rs) | `gallery-probability.png`: a normal and a Weibull probability axis, plotting positions and fitted lines computed by the host | `cargo run --locked -p figgy-renderer --example probability_paper -- target/probability` |
| [ternary.rs](ternary.rs) | `gallery-ternary.png` | `cargo run --locked -p figgy-renderer --example ternary -- target/ternary` |
| [surface_3d.rs](surface_3d.rs) | `gallery-surface.png`, `gallery-quiver3d.png` | `cargo run --locked -p figgy-renderer --example surface_3d -- target/surface-3d` |
| [trimesh.rs](trimesh.rs) | `gallery-trimesh.png`, `gallery-trisurf.png` | `cargo run --locked -p figgy-renderer --example trimesh -- target/trimesh` |
| [parallel.rs](parallel.rs) | `gallery-parallel.png` | `cargo run --locked -p figgy-renderer --example parallel -- target/parallel` |
| [range_bars.rs](range_bars.rs) | `gallery-waterfall.png`, `gallery-candlestick.png`, `gallery-gantt.png` | `cargo run --locked -p figgy-renderer --example range_bars -- target/range-bars` |
| [stem.rs](stem.rs) | `gallery-stem.png`, `gallery-lollipop.png` | `cargo run --locked -p figgy-renderer --example stem -- target/stem` |
| [broken_axes.rs](broken_axes.rs) | `gallery-broken-axes.png` | `cargo run --locked -p figgy-renderer --example broken_axes -- target/broken-axes` |
| [feynman.rs](feynman.rs) | `gallery-feynman.png` | `cargo run --locked -p figgy-renderer --example feynman -- target/feynman` |
| [nist_grotrian.rs](nist_grotrian.rs) | `nist-grotrian-he-i.png`: He I from the bundled NIST levels and lines exports, parsed by a std-only CSV reader; pass your own exports as two more arguments | `cargo run --locked -p figgy-renderer --example nist_grotrian -- target/nist-grotrian` |
| [nist_grotrian_gallery.rs](nist_grotrian_gallery.rs) | `he-i`, `ar-i`, `ar-i-all`, `cu-i`, `u-i` at 1x and 2x (`<name>-<scale>x.png`); `he-i-1x.png` is `gallery-grotrian.png` | `cargo run --locked -p figgy-renderer --example nist_grotrian_gallery -- target/nist-grotrian-gallery` |
| [gallery_concepts.rs](gallery_concepts.rs) | The six `concept-*.png` images | `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts` |
| [radial_gallery.rs](radial_gallery.rs) | One PNG per material (`matte`, `ceramic`, `satin-metal`, `toon`, `enamel`, `brushed-metal`, `paper`, `plywood`, `hatch`, `pearl`), plus `outlined-raised-pie`, `corners-sharp`, `corners-rounded`, `individual-edit`, `pie-of-pie`, `gallery-sunburst.png` and the hover frames `hover-00` to `hover-19` | `cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery` |
| [categorical_gallery.rs](categorical_gallery.rs) | `grouped-matte`, `horizontal-satin`, `stacked-enamel`, `percent-outline`, `signed-stacked`: categorical documents opened as Cartesian category bars by `Config::from_categorical` | `cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery` |
| [boxplot_gallery.rs](boxplot_gallery.rs) | `01-classic-matte`, `02-satin`, `03-horizontal`, `04-grouped-notch`, `05-mean-selection`: box plot documents opened as Cartesian box plot series by `Config::from_boxplot`; `gallery-violin.png` and `gallery-raincloud.png`, built as box plot series directly | `cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery` |
| [radar_gallery.rs](radar_gallery.rs) | `01-spider-web`, `02-circle-values`, `03-fixed-axis`, `04-four-series`, `05-selection`: radar documents opened as polar radar series by `Config::from_radar` | `cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery` |
| [math_gallery.rs](math_gallery.rs) | `math-gallery.png`, `math-gallery@2x.png`, `math-gallery-sketch.png`, `math-gallery-sketch@2x.png`: every math construction written with `RichText::from_tex`, each above its TeX source, in the precise and sketch draw styles | `cargo run --locked -p figgy-renderer --example math_gallery -- target/math-gallery` |
| [radial_titles.rs](radial_titles.rs) | `pie.png`: a rich title on the radial chart | `cargo run --locked -p figgy-renderer --example radial_titles -- target/radial-titles` |
| [precise_sketch_preview.rs](precise_sketch_preview.rs) | `style-growth-response-precise.png`, `style-growth-response-sketch.png` | `cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch` |
| [milkyway_line_preview.rs](milkyway_line_preview.rs) | One PNG at the given path | `cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway-preview.png` |
| [constellation_preview.rs](constellation_preview.rs) | One PNG at the given path | `cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation-preview.png` |

[support/gallery.rs](support/gallery.rs) holds device setup, the theme
helpers (`theme_named`, `themed`), a plain chart frame and PNG checks.
[support/styled.rs](support/styled.rs) holds the scene chart, colourbar and
export of the 1200 × 800 designed scenes. Each chart's data, series and
settings are in its own example file, and its look is a built-in theme of
its kind ([Themes](../USAGE.md#themes)); the
[gallery pages](https://github.com/shim9610/figgy/blob/master/crates/renderer/gallery/lines.md) name the theme of every image.

## Interactive examples

These open native windows and show host event handling as well as drawing.

```bash
cargo run --locked -p figgy-renderer --example winit_simple
cargo run --locked -p figgy-renderer --features egui_demo --example egui_embed
cargo run --locked -p figgy-renderer --features egui_demo --example radial_editor
cargo run --locked --release -p figgy-renderer --features egui_demo --example ssot_lab
cargo run --locked --release -p figgy-renderer --features egui_demo --example streaming_capacity_demo
cargo run --locked -p figgy-renderer --features egui_demo --example constellation_lab
```

- **winit_simple** and **egui_embed** draw three panels in a host window and
  handle pointer input. `winit_simple` also selects, drags and resizes chart
  elements.
- **radial_editor** selects, edits and drags the parts of a radial chart.
  `Edit plot area` shows eight resize handles.
- **ssot_lab** edits four charts live, one per draw style. All four series
  share one X column, and vertically adjacent panels share their X range.
- **streaming_capacity_demo** generates its data lazily through `ColumnSource`
  and draws every point. Add `-- --benchmark` to print timings for the default
  source.
- **constellation_lab** binds every `ConstellationOptions` field to a slider.

[Interaction contracts](../INTERACTION.md) explain logical coordinates, hit
priority, incremental deltas, rejected edits and export state.
The [browser product](https://github.com/shim9610/figgy/blob/master/crates/web/README.md)
(`crates/web`) wraps the renderer for WebAssembly. Its JavaScript API and
`<figgy-chart>` element are its own and do not mirror every native Rust API.

## Review and measurement tools

These are for checking renderer output and cost while changing it. Their
images are not in the README.

| Example | Purpose | Command |
|---|---|---|
| [constellation_demo.rs](constellation_demo.rs) | Despite its name, renders the Milkyway style: parameter sweeps of the line stars and ribbon, spectrum and sampling checks, scatter planets and errorbar jets, into `target/constellation_demo/` | `cargo run --locked -p figgy-renderer --example constellation_demo` |
| [sketch_demo.rs](sketch_demo.rs) | Precise and Sketch exports of one chart, and a Sketch tour with line, scatter-line and errorbar series, into `target/sketch_demo/` | `cargo run --locked -p figgy-renderer --example sketch_demo` |
| [raster_bench.rs](raster_bench.rs) | Prints the CPU raster time of the grid and decoration layers for Precise and Milkyway at two sizes; needs no GPU | `cargo run --locked --release -p figgy-renderer --example raster_bench` |
| [monotonic_bench.rs](monotonic_bench.rs) | Times a monotonic-x chart on the native GPU: one full call of N samples, sliding appends to a window, and a 1x export check that the line covers each pixel column's data range; f64 data or i16 counts on a volts axis | `cargo run --locked --release -p figgy-renderer --example monotonic_bench -- once --samples 100000000` |

## 한국어 안내

- **갤러리 그림**: README의 모든 그림은 `assets/`에 있다. 위 표의 "Drawn by" 열이 그 그림을
  그리는 예제와 함수를 가리킨다. `main`에서 사례 목록을 만드는 예제는 사례 이름으로 적었고,
  사례 이름이 곧 출력 파일 이름이다.
- **실행**: 명령은 저장소 루트 기준이다. 첫 인자가 출력 경로이며, 창 없이 GPU 어댑터만 있으면 된다.
  GPU가 없는 장비에서는 Mesa lavapipe 같은 소프트웨어 Vulkan 드라이버를 설치해 쓴다.
- **README에 없는 출력**: `steps.png`(계단 보간 세 가지 비교), `gradient.png`(데이터 단위
  격자 화살표와 격자 색 지도), `mixed-axes.png`(서브플롯 안의 보조 Y축)처럼 기능 확인용
  출력도 같은 예제가 함께 만든다.
- **데이터 경로**: 선·오차 막대·히스토그램·행렬·극좌표·벡터장 예제는 컬럼을 등록한다.
  범주형 막대 예제는 `Config::from_categorical`로, 박스플롯 예제는 `Config::from_boxplot`으로
  문서를 읽어 컬럼·레코드를 등록하고, 레이더 예제는 `Config::from_radar`로 문서를 읽어 컬럼을
  등록한다. 세 문서 리더는 이번 릴리스에서 사용 중단(deprecated)되며 다음 마일스톤에서
  제거된다. 원형 예제는 차트의 소량 데이터 모델을 직접 만든다.
  박스플롯의 사분위수 등 통계값은 호출자가 제공한다.
- **0.19 차트**: 삼각 좌표, 3D 곡면·화살표, 삼각 메시, 육각 빈, 컬러 이미지, 극좌표 히트맵, 장미, 선버스트,
  평행 좌표, 바이올린·레인클라우드, 폭포·캔들스틱·간트, 도형 주석, 줄기·롤리팝, 축 끊기, 에너지 준위,
  파인만 도표 그림은 위 [0.19 표](#charts-new-in-019)의 예제가 그린다. 그리는 값은 모두 예제가 계산한다.
- **창 예제와 도구**: 편집 예제는 실제 창을 열어 선택·이동·영역 조정을 보여준다.
  `constellation_demo`, `sketch_demo`, `raster_bench`, `monotonic_bench`는 렌더러를 바꿀 때 출력과 비용을
  확인하는 도구다.
