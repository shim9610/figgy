# figgy

[![Rust packages](https://github.com/shim9610/figgy/actions/workflows/ci.yml/badge.svg)](https://github.com/shim9610/figgy/actions/workflows/ci.yml)

**Charts in Rust, rendered with wgpu.** Draw scientific plots, style small-data charts,
export PNGs, or embed interactive charts in your own application.

**[Try it in your browser → figgyplot.com](https://figgyplot.com/)**

[Quick start](#quick-start) · [Examples](#run-the-examples) · [Documentation](#documentation) · [한국어](README.ko.md)

## Gallery

**Click an image to open the Rust code that produces it.** All data is synthetic;
the images are actual native renderer output. GIFs show exported animation frames.

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

**Draw styles: Precise · Sketch (hand-drawn) · Milkyway · Constellation**

<p>
  <a href="crates/renderer/examples/precise_sketch_preview.rs"><img src="crates/renderer/assets/style-growth-response-precise.png" alt="Precise growth curves with clean lines and markers" width="23%" align="top"></a>
  <a href="crates/renderer/examples/precise_sketch_preview.rs"><img src="crates/renderer/assets/style-growth-response-sketch.png" alt="Sketch growth curves with hand-drawn lines, axes and lettering" width="23%" align="top"></a>
  <a href="crates/renderer/examples/milkyway_line_preview.rs"><img src="crates/renderer/assets/style-growth-response-milkyway.png" alt="Milkyway line-only growth curves with stars and nebula" width="23%" align="top"></a>
  <a href="crates/renderer/examples/constellation_preview.rs"><img src="crates/renderer/assets/style-growth-response-constellation.png" alt="Constellation growth curves with stars at the source samples" width="23%" align="top"></a>
</p>

Milkyway uses line-only series; Constellation places stars at the source samples.

[Six editable visual recipes](crates/renderer/GALLERY.md#visual-concepts) show how
palettes, axes, materials and labels change the same chart APIs.

[More styles and reproduction commands](crates/renderer/GALLERY.md).

## Quick start

Use **Rust 1.99 or later** and a working GPU driver. The repository pins Rust 1.99.0.
Install renderer **0.17.0** from [crates.io](https://crates.io/crates/figgy-renderer).
It includes model **0.11.0**. Add these dependencies to your application's
`Cargo.toml`:

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.17.0" }
wgpu = "30"
pollster = "1"
```

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
        4 * 1024 * 1024,
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

## Run the examples

From a checkout of this repository:

```bash
cargo run --locked -p figgy-renderer --example line
cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts
cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars
cargo run --locked -p figgy-renderer --example histogram -- target/histogram
cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
```

See the [example index](crates/renderer/examples/README.md) for animations,
interactive editors and the data contract used by each chart.

## Embed and edit

| Goal | Example or guide |
|---|---|
| Draw in a native window | [winit_simple.rs](crates/renderer/examples/winit_simple.rs) |
| Embed in egui | [egui_embed.rs](crates/renderer/examples/egui_embed.rs) — enable `egui_demo` |
| Edit pies, donuts, bars or box plots | [Native editors](crates/renderer/examples/README.md#interactive-editors) |
| Connect picking, dragging and plot-area resizing | [Interaction guide](crates/renderer/INTERACTION.md) |
| Render data that does not fit in GPU memory | [Streaming and residency](crates/renderer/ARCHITECTURE.md#exact-streaming-and-residency) |

The Rust renderer can share a device with a **wgpu 30** host. No GUI framework is
required by default. Pie/donut, categorical bars and box plots use separate bounded
data models; they do not require the Cartesian column/streaming API.

## WebAssembly example

The Rust renderer also compiles to `wasm32-unknown-unknown` and renders through
WebGPU. [`crates/web`](crates/web/README.md) demonstrates how an application can
wrap it with wasm-bindgen and connect a canvas, events and async calls. This is a
**browser integration example**, separate from the Rust library's API; its
`FiggyChart` / `<figgy-chart>` wrapper does not expose every renderer chart type.

[Build and run the browser example](crates/web/README.md).
The independently developed [figgyplot.com Studio](https://figgyplot.com/) is a
separate application, not a component bundled with the renderer.

## Documentation

| Topic | Guide |
|---|---|
| Native setup, data adapters, embedding and export | [Usage](crates/renderer/USAGE.md) |
| Axes, labels, legends, series and color maps | [Configuration reference](crates/renderer/CONFIG.md) |
| Pies/donuts · categorical bars · box plots · histograms | [Radial](crates/renderer/RADIAL.md) · [Bars](crates/renderer/CATEGORICAL.md) · [Boxes](crates/renderer/BOXPLOT.md) · [Histograms](crates/renderer/HISTOGRAM.md) |
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
