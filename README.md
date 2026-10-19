# figgy

[![Rust packages](https://github.com/shim9610/figgy/actions/workflows/ci.yml/badge.svg)](https://github.com/shim9610/figgy/actions/workflows/ci.yml)

**Charts in Rust, rendered with wgpu.** Draw scientific plots, style small-data charts,
export PNGs, or embed interactive charts in your own application.

**[Try it in your browser → figgyplot.com](https://figgyplot.com/)**

[Quick start](#quick-start) · [Examples](#run-the-examples) · [Documentation](#documentation) · [한국어](README.ko.md)

## Gallery

**Each image links to the example file that draws it.** The
[example index](crates/renderer/examples/README.md#gallery-images) names the
function and the command for every image. All data is synthetic; the images are
actual native renderer output. GIFs show exported animation frames.

**Dashboards · Sketch · Pop art**

<p>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-observability.png" alt="Dark service dashboard with mint and lavender traffic curves" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-revenue.png" alt="Light revenue dashboard with horizontal rounded stacks" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-editorial.png" alt="Warm paper-colored sketch chart with hand-drawn lines and lettering" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-pop-art.png" alt="Bright toon-shaded pie with dark outlines and shallow extrusion" width="23%" align="top"></a>
</p>

**Pearl · Terrain · Contours · Box plot**

<p>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-portfolio.png" alt="Oblique pearl-material donut on a dark background" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-terrain.png" alt="Synthetic terrain with a forest-to-sand color ramp and fine isolines" width="23%" align="top"></a>
  <a href="crates/renderer/examples/heatmap_contours.rs"><img src="crates/renderer/assets/gallery-contours.png" alt="Heatmap with labeled contours" width="23%" align="top"></a>
  <a href="crates/renderer/examples/boxplot_gallery.rs"><img src="crates/renderer/assets/gallery-boxplot.png" alt="Box plot with outliers" width="23%" align="top"></a>
</p>

**Lines · Error bars · Histograms**

<p>
  <a href="crates/renderer/examples/line.rs"><img src="crates/renderer/assets/gallery-line.png" alt="A sine wave" width="23%" align="top"></a>
  <a href="crates/renderer/examples/error_bars.rs"><img src="crates/renderer/assets/gallery-errorbars.png" alt="Response curves with error bars" width="23%" align="top"></a>
  <a href="crates/renderer/examples/histogram.rs"><img src="crates/renderer/assets/gallery-histogram.png" alt="Vertical histogram" width="23%" align="top"></a>
  <a href="crates/renderer/examples/histogram.rs"><img src="crates/renderer/assets/gallery-histogram-horizontal.png" alt="Horizontal histogram" width="23%" align="top"></a>
</p>

**Bars · Donut · Animated curves**

<p>
  <a href="crates/renderer/examples/categorical_gallery.rs"><img src="crates/renderer/assets/gallery-categorical.png" alt="Grouped categorical bars" width="23%" align="top"></a>
  <a href="crates/renderer/examples/radial_gallery.rs"><img src="crates/renderer/assets/gallery-donut.png" alt="Ceramic 3D donut" width="23%" align="top"></a>
  <a href="crates/renderer/examples/animated_lines.rs"><img src="crates/renderer/assets/gallery-wave.gif" alt="Travelling waves" width="23%" align="top"></a>
  <a href="crates/renderer/examples/animated_lines.rs"><img src="crates/renderer/assets/gallery-orbit.gif" alt="Animated phase portrait" width="23%" align="top"></a>
</p>

**Radar · Radar with value labels · Polar · Compass rose**

<p>
  <a href="crates/renderer/examples/radar_gallery.rs"><img src="crates/renderer/assets/gallery-radar.png" alt="Radar chart comparing two releases over six categories on a spider-web grid" width="23%" align="top"></a>
  <a href="crates/renderer/examples/radar_gallery.rs"><img src="crates/renderer/assets/gallery-radar-circle.png" alt="Radar chart on a circular grid with a value label at every vertex" width="23%" align="top"></a>
  <a href="crates/renderer/examples/polar_plot.rs"><img src="crates/renderer/assets/gallery-polar.png" alt="Antenna pattern filled to the centre in polar coordinates" width="23%" align="top"></a>
  <a href="crates/renderer/examples/polar_plot.rs"><img src="crates/renderer/assets/gallery-polar-compass.png" alt="Compass rose: a polar scatter-line, clockwise from the top, with arc segments" width="23%" align="top"></a>
</p>

**Wind field · Surface currents · Colour-mapped points · Bubble chart**

<p>
  <a href="crates/renderer/examples/vector_field.rs"><img src="crates/renderer/assets/gallery-wind.png" alt="A sea breeze: wind speed as a smooth field with direction arrows on a grid" width="23%" align="top"></a>
  <a href="crates/renderer/examples/vector_field.rs"><img src="crates/renderer/assets/gallery-currents.png" alt="Surface currents at scattered drifters, arrow length and colour by speed" width="23%" align="top"></a>
  <a href="crates/renderer/examples/value_maps.rs"><img src="crates/renderer/assets/gallery-colormap.png" alt="Particles along two spiral arms coloured by age through the colourbar" width="23%" align="top"></a>
  <a href="crates/renderer/examples/value_maps.rs"><img src="crates/renderer/assets/gallery-bubbles.png" alt="Bubble chart of income and life expectancy for three regions, area by population" width="23%" align="top"></a>
</p>

**Forecast bands · Step schedule · Twin axes · Small multiples**

<p>
  <a href="crates/renderer/examples/area_step.rs"><img src="crates/renderer/assets/gallery-forecast.png" alt="A demand forecast: the history solid, the forecast dashed inside 80 and 95 percent bands" width="23%" align="top"></a>
  <a href="crates/renderer/examples/area_step.rs"><img src="crates/renderer/assets/gallery-thermostat.png" alt="A thermostat schedule as a filled step line under the measured room temperature" width="23%" align="top"></a>
  <a href="crates/renderer/examples/overlay_axes.rs"><img src="crates/renderer/assets/gallery-twin-axes.png" alt="A battery discharge: voltage on the left axis, current on a right overlay axis" width="23%" align="top"></a>
  <a href="crates/renderer/examples/subplots.rs"><img src="crates/renderer/assets/gallery-subplots.png" alt="Four sensors as a 2 by 2 grid of subplots sharing the hour axis" width="23%" align="top"></a>
</p>

**Draw styles: Precise · Sketch (hand-drawn) · Milkyway · Constellation**

<p>
  <a href="crates/renderer/examples/precise_sketch_preview.rs"><img src="crates/renderer/assets/style-growth-response-precise.png" alt="Precise growth curves with clean lines and markers" width="23%" align="top"></a>
  <a href="crates/renderer/examples/precise_sketch_preview.rs"><img src="crates/renderer/assets/style-growth-response-sketch.png" alt="Sketch growth curves with hand-drawn lines, axes and lettering" width="23%" align="top"></a>
  <a href="crates/renderer/examples/milkyway_line_preview.rs"><img src="crates/renderer/assets/style-growth-response-milkyway.png" alt="Milkyway line-only growth curves with stars and nebula" width="23%" align="top"></a>
  <a href="crates/renderer/examples/constellation_preview.rs"><img src="crates/renderer/assets/style-growth-response-constellation.png" alt="Constellation growth curves with stars at the source samples" width="23%" align="top"></a>
</p>

Milkyway uses line-only series; Constellation places stars at the source samples.

[Six editable visual recipes](crates/renderer/GALLERY.md#visual-concepts) show how
palettes, axes, materials and labels restyle charts built with the same APIs.

[More styles and reproduction commands](crates/renderer/GALLERY.md).

## Chart types

| Chart | Input | Guide |
|---|---|---|
| Lines and scatter | `f64` columns through `ColumnSource` | [Usage](crates/renderer/USAGE.md) · [Series schema](crates/renderer/CONFIG.md#data_config--declarative-series-schema-the-active-api) |
| Area fills and step lines | A line's `fill` (to a baseline or another series) and `interpolation` | [Area fill and step lines](crates/renderer/CONFIG.md#area-fill-and-step-lines) · [area_step.rs](crates/renderer/examples/area_step.rs) |
| Error bars | X, Y and error columns | [error_bars.rs](crates/renderer/examples/error_bars.rs) |
| Histograms | Bin edges and counts | [Histograms](crates/renderer/HISTOGRAM.md) |
| Heatmaps and contours | X and Y coordinate columns and one z column per grid row or column (`MatrixRef`) | [heatmap_contours.rs](crates/renderer/examples/heatmap_contours.rs) |
| Vector fields (quiver) | Anchors and (u, v) columns, or two matrices on a grid | [Vector fields](crates/renderer/CONFIG.md#vector-fields) · [vector_field.rs](crates/renderer/examples/vector_field.rs) |
| Colour and size maps (bubbles) | A value column per point, mapped by `PointColorMap` through the colorbar (`Config.colorbar`) or by `PointSizeMap` through its size domain | [Point colour and size maps](crates/renderer/CONFIG.md#point-colour-and-size-maps) · [value_maps.rs](crates/renderer/examples/value_maps.rs) |
| Overlay axes, subplots, annotations | Extra right-hand Y axes, a grid of panels, reference lines, spans, text and arrows | [Overlays](crates/renderer/INTERACTION.md#overlay-layers) · [Subplots](crates/renderer/INTERACTION.md#subplots) · [Annotations](crates/renderer/INTERACTION.md#annotation-layer) |
| Polar plots | (angle, radius) columns and `Config.polar` | [Polar](crates/renderer/POLAR.md) |
| Pies and donuts | `RadialChart` values | [Radial (in Korean)](crates/renderer/RADIAL.md) |
| Categorical bars | `CategoricalChart` categories and series | [Bars (in Korean)](crates/renderer/CATEGORICAL.md) |
| Box plots | `BoxPlotChart` summaries | [Boxes](crates/renderer/BOXPLOT.md) |
| Radar charts | `RadarChart` categories and one value per category per series | [Radar](crates/renderer/RADAR.md) |

Every row above pies and donuts is a column-backed chart: its data enters
through `ColumnSource` into one GPU column pool, and these charts share one
renderer, picking and export path. Overlay axes and subplots work with these
charts, except that a polar chart takes no overlay axes. Streamed, non-resident
sources draw lines, scatter, error bars, histograms and heatmaps in a
single-panel document. Contours, fills and step lines, vector fields, colour and
size maps, polar plots, and documents with overlays or subplots reject streamed
sources with an explicit error. Pies and donuts, categorical bars, box plots and
radar charts keep their categories and values in their own small model, with no
`ColumnSource`, and each has its own renderer. The annotation layer works on
both kinds.

## Quick start

Use **Rust 1.99 or later** and a working GPU driver. The repository pins Rust 1.99.0.
Add these dependencies to your application's `Cargo.toml`:

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.0" }
wgpu = "30"
pollster = "1"
```

The `version` line installs [figgy-renderer](https://crates.io/crates/figgy-renderer)
0.18.0, with figgy-model 0.12.0, from crates.io. With a local checkout, use
`path = "../figgy/crates/renderer"` (relative to your application) instead of
`version`. Code written for 0.17.0 needs the changes in the
[migration notes](CHANGELOG.md#migrating-from-0170).

Copy this into `src/main.rs`, then run `cargo run`. It writes `line.png` without
opening a window. `renderer` is the Rust import name of the `figgy-renderer` package.

```rust
use renderer::{
    Chart, Color, Renderer, RendererDevice, SeriesConfig,
    data::Column,
    data_config::{DataLineStyleConfig, DataRenderType},
    data_render::{create_instance, request_adapter, request_device},
    default, encode_png,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let mut renderer = Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024, // initial GPU column pool size in bytes
    )?;

    let x: Vec<f64> = (0..=100).map(|i| i as f64 / 10.0).collect();
    let y: Vec<f64> = x.iter().map(|x| x.sin()).collect();
    renderer.add_column(
        "x",
        &Column {
            data: x,
            min: 0.0,
            max: 10.0,
        },
    )?;
    renderer.add_column(
        "y",
        &Column {
            data: y,
            min: -1.0,
            max: 1.0,
        },
    )?;

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 600,
    });
    config.legend.visible = false;
    config.left_y.out_margin = 80.0;
    config.right_y.out_margin = 40.0;
    config.bottom_x.out_margin = 70.0;
    config.top_x.out_margin = 24.0;
    let mut chart = Chart::new(config)
        .with_title("A sine wave")
        .with_x_title("Time (s)")
        .with_y_title("Amplitude");
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    let series = [SeriesConfig {
        series_id: "sine".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::from_rgb8(34, 104, 198),
                line_width: 2.5,
                fill: None,
                fill_color: None,
                interpolation: renderer::data_config::LineInterpolation::Linear,
            },
        },
    }];
    let image = pollster::block_on(renderer.export_panel_rgba_with_clear_async(
        &chart,
        &series,
        1.0,
        Color::WHITE,
    ))?;
    let output = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "line.png".into());
    std::fs::write(&output, encode_png(&image)?)?;
    renderer.wait_idle();
    println!(
        "{}: {}x{}",
        std::path::Path::new(&output).display(),
        image.width,
        image.height
    );
    Ok(())
}
```

The renderer returns pixels or PNG bytes; the application chooses where to save them.
For your own data, replace the `x`/`y` vectors or implement
[`ColumnSource`](crates/renderer/USAGE.md#columnsource--the-data-adapter-trait).

### Polar plots and radar charts in a few lines

A polar plot is a Cartesian chart with `Config.polar` set. `x_column` holds the
angle (in degrees by default) and `y_column` the radius; the angle axis becomes
the outer ring and the radius axis a labelled ray. `set_y_range` sets the radius
axis. `set_x_range` only chooses which angle ticks, spokes and labels are drawn:
one turn is always 360° (or 2π) from `zero_angle_degrees`, so a range such as
0..180 does not stretch onto the full circle; the `sector` field of
`PolarOptions` shows part of it. To turn the quick start into a polar plot,
upload angle and radius columns in place of the sine and set up the chart like
this:

```rust,ignore
use renderer::{format::LabelFormat, polar::PolarOptions};

config.polar = Some(PolarOptions::default()); // 0° right, counter-clockwise
let mut chart = Chart::new(config);
chart.set_x_range(0.0, 360.0); // which angle ticks and spokes are drawn
chart.set_y_range(0.0, 1.0); // radius axis
// The range helpers choose tick spacing and label format, so set these after them.
chart.config_mut().bottom_x.major_spacing = 30.0; // spokes every 30°
chart.config_mut().bottom_x.label_style.format = LabelFormat::Degrees; // "30°"
```

A radar chart keeps its categories and values in a `RadarChart` model, with no
columns to upload, and has its own renderer:

```rust,ignore
use renderer::radar::{RadarChart, RadarRenderer, RadarSeries};
use renderer::{Category, Color, encode_png};

let mut chart = RadarChart::default();
chart.categories = ["Speed", "Safety", "Size", "Docs", "Tests"]
    .iter()
    .map(|c| Category::new(c.to_lowercase(), *c))
    .collect();
chart.series = vec![
    RadarSeries::new("v1", "Release 1", vec![6.0, 8.0, 4.0, 5.0, 7.0], Color::from_rgb8(60, 110, 200)),
    RadarSeries::new("v2", "Release 2", vec![8.0, 7.0, 6.0, 8.0, 9.0], Color::from_rgb8(220, 90, 70)),
];
// `gpu` is a `RendererDevice`. In the quick start, bind it with
// `let gpu = RendererDevice::new(...)` and pass `gpu.clone()` to `Renderer::try_new`.
let mut radar = RadarRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm)?;
let image = radar.export_rgba(&chart, (720, 520), 1.0)?;
std::fs::write("radar.png", encode_png(&image)?)?;
// In a render pass instead: radar.prepare(&chart, (720, 520), 1.0)?.draw(&mut pass);
```

The complete programs are [polar_plot.rs](crates/renderer/examples/polar_plot.rs)
and [radar_gallery.rs](crates/renderer/examples/radar_gallery.rs); the guides are
[Polar](crates/renderer/POLAR.md) and [Radar](crates/renderer/RADAR.md).

## Run the examples

From a checkout of this repository:

```bash
cargo run --locked -p figgy-renderer --example line
cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts
cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
cargo run --locked -p figgy-renderer --example area_step -- target/area-step
cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
cargo run --locked -p figgy-renderer --example subplots -- target/subplots
cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars
cargo run --locked -p figgy-renderer --example histogram -- target/histogram
cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
```

See the [example index](crates/renderer/examples/README.md) for the function and
command behind every gallery image, the outputs of every headless example,
animations and interactive examples.

## Embed and edit

| Goal | Example or guide |
|---|---|
| Draw in a native window | [winit_simple.rs](crates/renderer/examples/winit_simple.rs) |
| Embed in egui | [egui_embed.rs](crates/renderer/examples/egui_embed.rs), run with `--features egui_demo` |
| Edit pies, donuts, bars, box plots or radar charts | [Native editors](crates/renderer/examples/README.md#interactive-examples) |
| Connect picking, dragging and plot-area resizing | [Interaction guide](crates/renderer/INTERACTION.md) |
| Render data that does not fit in GPU memory | [Streaming and residency](crates/renderer/ARCHITECTURE.md#exact-streaming-and-residency) |

The Rust renderer can share a device with a **wgpu 30** host. No GUI framework is
required by default.

## WebAssembly example

The Rust renderer also compiles to `wasm32-unknown-unknown` and renders through
WebGPU. [`crates/web`](crates/web/README.md) demonstrates how an application can
wrap it with wasm-bindgen and connect a canvas, events and async calls. This is a
**browser integration example**, separate from the Rust library's API. Its
`FiggyChart` export and `<figgy-chart>` element cover Cartesian and polar charts;
`FiggyBoundedChart` covers pies, donuts, categorical bars, box plots and radar
charts, and the host handles their canvas events.

[Build and run the browser example](crates/web/README.md).
[figgyplot.com Studio](https://figgyplot.com/) is a separate web application, not
part of the `figgy-renderer` crate or the browser example.

## Documentation

| Topic | Guide |
|---|---|
| Native setup, data adapters, embedding and export | [Usage](crates/renderer/USAGE.md) |
| Axes, labels, legends, series and color maps | [Configuration reference](crates/renderer/CONFIG.md) |
| Pies/donuts · categorical bars · box plots · radar · polar · histograms | [Radial (in Korean)](crates/renderer/RADIAL.md) · [Bars (in Korean)](crates/renderer/CATEGORICAL.md) · [Boxes](crates/renderer/BOXPLOT.md) · [Radar](crates/renderer/RADAR.md) · [Polar](crates/renderer/POLAR.md) · [Histograms](crates/renderer/HISTOGRAM.md) |
| Ownership, prepared frames and API compatibility | [API contracts](crates/renderer/API.md) |
| Memory flow, caching, streaming and shader details | [Architecture](crates/renderer/ARCHITECTURE.md) |
| Source versions and migration notes | [Changelog](CHANGELOG.md) |
| Platform checks and known limits | [CI results](ci/STATUS.md) |

Linux/Vulkan and Windows/DX12 are covered by native rendering and pixel tests.
**macOS/Metal support remains experimental.** CI uses software or virtual GPU
adapters; its results do not establish coverage of every physical GPU driver.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for bug reports, development checks and pull
requests. Report suspected vulnerabilities privately using [SECURITY.md](SECURITY.md).

## License

Code: [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
Bundled fonts retain their OFL notices: [Liberation Sans](crates/renderer/fonts/LICENSE-LiberationSans.txt)
and [Comic Neue](crates/renderer/fonts/LICENSE-ComicNeue.txt).
