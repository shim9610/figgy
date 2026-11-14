# Styles and visual concepts

[← Gallery](../../../README.md#gallery) · [Lines, areas and stems](lines.md) · [Bars and distributions](distributions.md) · [Ranges and timelines](ranges.md) · [Fields and maps](fields.md) · [3D](3d.md) · [Polar and radial](polar.md) · [Science diagrams](science.md) · [Layouts and annotations](layouts.md) · **Styles and visual concepts**

Click an image to open the example code that draws it. All data is synthetic;
the images are native renderer output.
Each entry names the theme its example applies ([Themes](../USAGE.md#themes)):
a theme family, or a custom theme the example builds in its support code
(`examples/support/looks.rs`); some examples edit a copy first, such as the
palette order or a fill opacity.

## Precise style

<a href="../examples/precise_sketch_preview.rs"><img src="../assets/style-growth-response-precise.png" alt="Precise style" width="760"></a>

`DrawStyle::Precise`.

**Code:** [precise_sketch_preview.rs](../examples/precise_sketch_preview.rs), `main`

**Theme:** the documented look (`Theme::default()`), with larger text

**Run:** `cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch`

## Sketch style

<a href="../examples/precise_sketch_preview.rs"><img src="../assets/style-growth-response-sketch.png" alt="Sketch style" width="760"></a>

`DrawStyle::Sketch` on the same data.

**Code:** [precise_sketch_preview.rs](../examples/precise_sketch_preview.rs), `main`

**Theme:** the documented look (`Theme::default()`), with larger text and `DrawStyle::Sketch`

**Run:** `cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch`

## Milkyway style

<a href="../examples/milkyway_line_preview.rs"><img src="../assets/style-growth-response-milkyway.png" alt="Milkyway style" width="760"></a>

Milkyway on `Line` series: stars and nebula only.

**Code:** [milkyway_line_preview.rs](../examples/milkyway_line_preview.rs), `main`

**Theme:** a custom theme, `looks::milkyway()` in the example support

**Run:** `cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway-preview.png`

## Constellation style

<a href="../examples/constellation_preview.rs"><img src="../assets/style-growth-response-constellation.png" alt="Constellation style" width="760"></a>

Constellation on `ScatterLine` series: stars at the source samples.

**Code:** [constellation_preview.rs](../examples/constellation_preview.rs), `main`

**Theme:** a custom theme, `looks::constellation()` in the example support

**Run:** `cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation-preview.png`

## Dark dashboard

<a href="../examples/gallery_concepts.rs"><img src="../assets/concept-observability.png" alt="Dark dashboard" width="760"></a>

A dark service-monitoring card.

**Code:** [gallery_concepts.rs](../examples/gallery_concepts.rs), `observability`

**Theme:** the `midnight` family

**Run:** `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts`

## Revenue stacks

<a href="../examples/gallery_concepts.rs"><img src="../assets/concept-revenue.png" alt="Revenue stacks" width="760"></a>

Rounded horizontal stacked bars with inside labels.

**Code:** [gallery_concepts.rs](../examples/gallery_concepts.rs), `revenue`

**Theme:** a custom theme, `looks::revenue()` in the example support

**Run:** `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts`

## Editorial sketch

<a href="../examples/gallery_concepts.rs"><img src="../assets/concept-editorial.png" alt="Editorial sketch" width="760"></a>

A warm paper-coloured Sketch figure.

**Code:** [gallery_concepts.rs](../examples/gallery_concepts.rs), `editorial`

**Theme:** a custom theme, `looks::editorial()` in the example support

**Run:** `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts`

## Toon pie

<a href="../examples/gallery_concepts.rs"><img src="../assets/concept-pop-art.png" alt="Toon pie" width="760"></a>

A toon-shaded pie with outlines.

**Code:** [gallery_concepts.rs](../examples/gallery_concepts.rs), `radial_concepts`

**Theme:** the `warm-paper` family

**Run:** `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts`

## Pearl donut

<a href="../examples/gallery_concepts.rs"><img src="../assets/concept-portfolio.png" alt="Pearl donut" width="760"></a>

A pearl donut from the same slice model.

**Code:** [gallery_concepts.rs](../examples/gallery_concepts.rs), `radial_concepts`

**Theme:** the `midnight` family

**Run:** `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts`

## Terrain

<a href="../examples/gallery_concepts.rs"><img src="../assets/concept-terrain.png" alt="Terrain" width="760"></a>

A custom colour ramp under fine isolines.

**Code:** [gallery_concepts.rs](../examples/gallery_concepts.rs), `terrain`

**Theme:** a custom theme, `looks::topographic()` in the example support

**Run:** `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts`
