# Radial charts / 원형 차트

`RadialChart` is a pie or donut chart for a small number of categories and
their shares of a whole. `RadialRenderer` can use the same wgpu device and
queue as the Cartesian `Renderer`. A radial chart uses no columns: it has no
`ColumnSource` and no streaming, and a slice's value is a property of the
slice written in the model, like its name, color and explode distance. It
uses no axes and no auto-fit either. Pass a model that holds all category
values and styles, and the renderer builds an immutable frame. In the
browser, `FiggyBoundedChart` in `crates/web`, the browser product, draws
radial charts with this renderer and exposes hit maps, selection, dragging
and PNG output
([WASM guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host)).

The radial chart API is in `figgy-renderer` on crates.io from 0.16.1. The
async constructor was available as `RadialRenderer::new_async` from 0.17.0
and was renamed `RadialRenderer::try_new_async` in 0.19.0. The
[annotation layer](#annotation-layer) and the chart's `chart_title` and
`active_layer` fields are available from 0.18.0.

## API overview

The model types are defined in `model::radial` and can also be imported from
`renderer::radial` and from the renderer crate root. After editing the model,
call `prepare` again. No separate color-change command or cache invalidation
call is needed.

| Type / method | Role |
|---|---|
| `RadialChart` | The model holding values, shape, shared style, labels and interaction state |
| `RadialSlice`, `RadialSplit` | Slice data, and the detail circle that expands one slice |
| `RadialStyle`, `RadialOutline` | Material, 3D shape, texture and outline settings |
| `RadialLabels`, `RadialLabelFormat` | Label position and the content shown |
| `RadialChart::format_labels()` | Writes each slice's label string to draw (`RadialSlice::text`) at edit time |
| `RadialChart::regenerate_labels()` | Rewrites only the generated labels (`text_generated`) and the missing ones, and keeps labels the host wrote (#313) |
| `RadialTarget`, `RadialInteraction` | Slice identity, and hover and selection state |
| `RadialChart::validate()` | Checks values and options. `prepare` checks the actual label layout in addition |
| `RadialRenderer::prepare()` | Reads the model and returns an immutable `Arc<RadialFrame>` |
| `RadialFrame::draw()` | Draws into a render pass the host opened |
| `RadialFrame::pick()` | A request to pick, on the GPU, the slice visible in that frame. `RadialPickTicket::resolve()` returns the result |
| `export_rgba()` / `export_png()` / `export_svg()` / `export_pdf()` | Draws the current model and returns it. `ExportOptions` sets the scale and background |
| `export_rgba_with_background()` / `export_png_with_background()` | Exports over transparency or another color instead of the document background (`RadialExportBackground`, #313) |
| `clear_cache()` / `end_frame()` | Releases references to internal caches / schedules reclaiming resources whose submissions have finished |

`RadialChart::default()` is an empty pie chart, so add slices with positive
values before drawing.

## Getting started

```rust,no_run
use renderer::{
    Color, Material, RadialChart, RadialKind, RadialLabels,
    RadialRenderer, RadialSlice, RendererDevice, encode_png,
};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let instance = renderer::gpu::create_instance();
# let adapter = renderer::gpu::request_adapter_blocking(&instance)?;
# let (device, queue) = renderer::gpu::request_device_blocking(&adapter)?;
let mut renderer = RadialRenderer::try_new(
    RendererDevice::new(device, queue)?,
    wgpu::TextureFormat::Rgba8Unorm,
)?;
let mut chart = RadialChart {
    slices: vec![
        RadialSlice::new("A", 30.0, Color::from_rgb8(28, 179, 155)),
        RadialSlice::new("B", 70.0, Color::from_rgb8(251, 128, 74)),
    ],
    kind: RadialKind::Donut { inner_radius: 0.48 },
    labels: RadialLabels::Outside,
    ..Default::default()
};
// A plain-text title uses the current chart font, 1.5 times `font_size`, and `label_color`.
chart.set_title_text("Regional share");
chart.style = renderer::RadialStyle::three_dimensional();
// The renderer only draws the label strings. Write them again after changing values or the format.
chart.format_labels();

// The logical size is 1000×600; the actual output is 2000×1200.
// Text and shapes are drawn again at this resolution.
let image = renderer.export_rgba_blocking(&chart, (1000, 600), renderer::ExportOptions::document(2.0))?;
std::fs::write("donut.png", encode_png(&image)?)?;
renderer.end_frame();
# Ok(())
# }
```

To draw on screen, get an `Arc<RadialFrame>` from
`prepare(&chart, logical_size, scale)` and call `frame.draw(&mut pass)` inside
the host's render pass. The target must have the RGBA8/BGRA8 format given to
the constructor, the size `frame.size()` and sample count 1. The renderer
does its own edge antialiasing: each physical pixel takes a 2×2 grid of
samples, and each finds the slice it hits, or the background, and that
slice's outline at its own position. The lit material of each surface the
samples hit (one slice, one normal) is evaluated once per pixel and counted
for each sample it covers, and the soft shadow is taken once per pixel
(#349). A pixel whose samples would all take one colour (inside one flat top
face, or on the background), and every edge pixel of a chart whose slices
are flat tops (no depth, or level without a bevel), takes one ray instead:
the colour its samples share, or analytic coverage of the slice edges and
outline (#41, #454). Bevels, creases and tilted side walls keep the samples.
`scale` is at least 0.5, and the target's
actual size follows it. There is no upper limit; if the actual size exceeds
the device's texture size limit, an error is returned. The host decides the
clear/load policy of its render pass. Transparent backgrounds are supported.
The logical size is at least 160 pixels in each of width and height. The
actual size is `round(logical size × scale)` and cannot exceed the device's
texture size limit. `draw` sets the viewport and scissor to the whole target.
To place the chart in part of a UI, draw it into a separate texture of this
size and let the host lay it out.

On WASM, create the renderer with `RadialRenderer::try_new_async(gpu, format).await`,
which waits for the browser's pipeline compilation. Then use the same
`prepare`/`draw`, and for output use `export_rgba(...).await` or
`export_png(...).await`. On native targets you can await the async functions
directly (with a device bundle made by `RendererDevice::new` they complete
without the host polling the device), or wait on the calling thread with a
`_blocking` function such as `export_rgba_blocking`.

## Values and split circles

- Each circle takes 1 to 64 items. Values must be finite and 0 or greater,
  and the total must be positive.
- Items of 0 stay in the model, but no shape or label is made for them. NaN,
  infinity, negative values and an overflowing total are rejected.
- Slice colors are opaque RGBA. The background and labels may use alpha.
- Angles are 0 degrees to the right and increase clockwise. The default start
  angle is -90 degrees.
- `explode` is how far a slice moves, relative to the outer radius. 0 to
  0.35 is supported.
- `RadialSplit` expands one item into a small circle on the right. The child
  values are in **the same unit** as the parent, and their sum must equal the
  parent value (a relative error up to 1e-9 is allowed). The child circle's
  labels show shares of the parent subtotal.

For example, if "Other" is 13 out of a total of 100 and its breakdown is 16%,
14% and 70%, the child values are `2.08`, `1.82` and `9.1`. Entering `16`,
`14` and `70` as they are gives a sum that does not match, and is an error.
`slice_index` is the index into the original array, counting items of 0. The
connector lines join the two angular edges of the chosen parent slice to the
detail circle. Placing the parent slice on the right, as in the example,
shows the relation between the two circles clearly.

### Sunbursts (nested rings)

When slices have `children`, the chart is a sunburst. For a tree of depth
`d`, the space between the hole (`kind.inner_radius()`; a pie has 0, so the
center ring is a disc) and the outer radius is divided into `d` rings of
equal thickness, and a node at depth `k` takes, on ring `k`, its own share of
its parent's angular span (its value relative to the parent's value, in child
order). The children's values must sum to at most the parent's value
(relative error 1e-9), and the remaining span is left empty on the child
ring. `children` is omitted from JSON when it is empty.

- In `RadialTarget { group, index }`, `group` is the ring (0 is the center)
  and `index` is the node's position on that ring counted in document order
  (the children in the order of their parents), including nodes of value 0.
  `slice()`, `slice_mut()`, `step_selection` (from the center ring outward,
  only nodes with a value), and labels' `DecorationId::RadialLabel` and
  `radial_label_{group}_{index}` names all follow this rule.
  `group_count()`, `group_len(group)`, `node_count()` and
  `node_span(target)` report the rings and spans.
- A label's percentage is the node's share of its parent's value (on the
  center ring, of the total). `format_labels()` writes the `text` of every
  ring.
- `Outside` labels are placed only on the outermost ring (stacked left and
  right as on a pie, with the connector starting at 1.015 times the outer
  radius). `Outside` on an inner ring is drawn as an inside label within its
  own band. Inside labels sit at the band's middle radius. They shrink to
  fit the sector in its own frame
  ([#357](https://github.com/shim9610/figgy/issues/357)):
  - the label's extent along the bisector is held to the band's thickness;
  - its extent across the bisector is held to the arc through its centre;
  - its outer corners stay inside the rim.

  So a horizontal label at the top or bottom of a thin ring keeps its size,
  and one at the ring's side is held to the thickness. A label is placed
  where it fits largest (#357):
  - along its arc: it slides from the bisector towards the top or bottom
    of a wide segment when it fits at least 5% larger there;
  - upright: on a flat chart, a label that cannot keep its size
    horizontally reads bottom to top (as a y axis title) when that fits
    larger, so a wide segment at the side of a thin ring keeps its label;
  - never below 12 logical pixels (or the chart's `font_size` when that is
    smaller). A label that would be smaller moves outside with a leader
    when its slice is on the outermost ring (or on a pie or donut); the
    circle makes room for it as for any outside label, and it is hit,
    selected and dragged as one. On an inner sunburst ring, where no
    leader reaches past the outer rings, it is hidden: not drawn and not
    hit. With `inner_label_fallback: InnerLabelFallback::Outside` it is
    pulled outside instead: its leader starts just past its own ring's rim
    and crosses the outer rings to the elbow beside the circle, and it is
    stacked with the outermost ring's outside labels in angle order, so no
    two meet. It is then an outside label in every respect: hover, click
    selection, the part-by-part outline, drag (the leader follows), PNG,
    SVG and PDF, serde (the key is omitted while `Hide`) and WASM.
- Sunbursts are drawn flat only. A nonzero `tilt_degrees` or `depth` in the
  chart style or a node style is rejected
  (`"a sunburst draws flat: no tilt or depth"`). A sunburst cannot be used
  with `split` (`"a sunburst has no detail circle"`).
- Limits: at most 8 rings (`MAX_SUNBURST_DEPTH`) and at most 64 nodes on one
  ring (`MAX_RADIAL_SLICES`). Other rejection reasons:
  `"a node's children sum to at most its value"`,
  `"a sunburst has at most 8 rings"`, `"a sunburst ring has at most 64 nodes"`.
- Picking, emphasis, hover and materials (with a different grain per ring)
  all work as for slices.
- `RadialStyle::sunburst()` is the flat look of web sunburst charts: no
  depth, gap or rounded corners, and a 1 px white rim and separators. The
  white line separates every node from its siblings and every ring from the
  next, even when a whole branch shares one colour. On another background,
  set `outline.color` to it. It is the host default
  ([#357](https://github.com/shim9610/figgy/issues/357)):
  `RadialStyle::default()` and `RadialChart::default()` draw it, so a chart
  without a theme shows its separators. The serde default a document reads
  when it omits the style keeps no outline, so stored documents draw as
  before; `RadialOutline::default()` keeps the documented (off) outline. A theme with `child_tint` (the Gallery and
  cool slate members) colours each branch from its top-level slice, each
  ring paler.

```rust
use renderer::radial::*;
use renderer::Color;
let mut chart = RadialChart::default();
let mut ops = RadialSlice::new("Ops", 60.0, Color::from_rgb8(220, 60, 40));
ops.children = vec![
    RadialSlice::new("Fleet", 40.0, Color::from_rgb8(60, 120, 220)),
    RadialSlice::new("Crew", 10.0, Color::from_rgb8(40, 170, 70)),
]; // the remaining 10 is an empty span on the child ring
chart.slices = vec![ops, RadialSlice::new("R&D", 40.0, Color::from_rgb8(80, 80, 80))];
chart.format_labels();
assert_eq!(chart.group_count(), 2);
assert_eq!(chart.slice(RadialTarget { group: 1, index: 1 }).map(|s| s.label.as_str()), Some("Crew"));
```

## Materials and 3D shading

The model default is a flat pie seen from straight above. Changing the
material alone does not change the tilt or depth. Set the default 3D view
with `RadialStyle::three_dimensional()`. It sets the Ceramic material, a 55°
tilt, depth 0.16, bevel 0.025, a 1.4° gap, inner and outer corners of 0.045
and a shadow; each field can be edited afterwards. The donut shape is set
separately in `kind`.

| Setting | Look |
|---|---|
| `Flat` | A flat look that uses the given color as is, with no lighting |
| `Matte` | Diffuse light and a faint surface grain |
| `Ceramic` | A smooth surface, bright highlights and a rounded-edge feel |
| `SatinMetal` | Satin metal with a broad highlight and a faint grain |
| `BrushedMetal` | Fine circumferential grain and directional reflection |
| `Toon` | Three shading steps. The outline is turned on or off separately |
| `Enamel` | A sharp coated gloss |
| `Hatch` | Diagonal hatching |
| `Pearl` | A faint pearl color shift at curved edges |
| `Paper` | A fine matte paper grain continuing over the top and side faces |
| `Wood` | Rough matte plywood with fine, irregular grain and no coarse pattern |

### Materials

`Material` is one enum for sectors (`RadialStyle::material`), bars
(`CategoryBarStyle::material`) and boxes (`BoxPlotStyle::material`), and a
name has the same look in every family. A bar or a box is shaded like a
flat (zero depth, zero tilt) pie top under the default light. Its normal is constant, so its lighting
and highlights are uniform over the body; textures (grain, brush, paper,
wood, hatching) still vary across it.

| Material | Sectors | Bars and boxes |
|---|---|---|
| `Flat` | yes | yes; the bar default |
| `Matte` | yes | yes; the box default |
| `Ceramic` | yes | yes; `gloss` lifts the whole body by at most 2.5/255 |
| `SatinMetal`, `BrushedMetal` | yes | yes; a uniform sheen and a brush along the bar; at 1x a `texture_scale` above 0.13 has no visible effect |
| `Enamel` | yes | yes; `gloss` has no visible effect |
| `Paper`, `Wood`, `Hatch` | yes | yes |
| `Toon`, `Pearl` | yes | rejected by `validate`: "material not supported by bars" / "material not supported by boxes" |

`Material::supported_on_flat_body` tells the two apart. Bar and box styles
have no `roughness`, `light` or `texture_angle_degrees`; they use the
`RadialStyle` defaults (0.35, `[-0.5, -0.6, 1.0]` and 0°). One bar texture
unit is 150 logical pixels: a bar's texture matches that of a sector of
radius 150 px, and Matte grain cells are 1.67 px.

The default and allowed range of each field are in the table under
[Style option defaults and ranges](#style-option-defaults-and-ranges). The
visible gap of a very small slice is limited to 20% of that slice's angle.

A 3D slice draws its top, outer wall, inner wall and cut faces in orthographic
projection. `inner_corner` and `outer_corner` set how rounded the slice ends
are seen from above (0 to 0.25 of the outer radius). 0 is a sharp corner, and
both apply in 2D and 3D. On small slices and thin rings the radius is limited
automatically. `bevel` actually rounds the top and bottom edges of a 3D slice
and is limited to less than half the depth. At depth 0 no 3D bevel is
applied. The shadow is a soft shadow approximated per slice. Materials need
no external images. With the 3D projection the original angles and
percentage labels are kept, but the area of a slice projected on screen
changes with the tilt.

`gloss` (0 to 1) sets the highlight strength, `roughness` its spread, and
`texture_angle_degrees` the direction of the surface pattern. Under the same
light, each material applies them differently. This is not a full
physically based renderer, but stylized shading that keeps the chart's
category colors. `Paper` and `Wood` use texture coordinates that continue up
the height, so no vertical stripes appear on the side faces. Fine patterns
that the output size and the face slope cannot resolve are weakened to
suppress flicker. `Flat` uses no gloss or texture, and `Toon` uses fixed
shading steps instead of roughness. `Wood` draws no growth rings or regular
stripes; it adds fine grain while keeping the slice's own color. For a rough
plywood look, start from `texture_strength = 0.7`, `roughness = 0.9` and
`gloss = 0.1`.

### Style option defaults and ranges

Lengths given as ratios are relative to the **outer radius**. Out-of-range
input and non-finite numbers are rejected. Even within range, the corners
and gaps of small slices can shrink so they do not overlap.

| Field | Default | Allowed range / meaning |
|---|---|---|
| `material` | `Flat` | The material list above |
| `tilt_degrees` | `0` | 0 to 65°. 0 looks straight down from above |
| `depth` | `0` | 0 to 0.4 of the radius |
| `bevel` | `0.015` | 0 to 0.1 of the radius. Applied at less than half the depth |
| `inner_corner`, `outer_corner` | `0` each | 0 to 0.25 of the radius |
| `gap_degrees` | `0` | 0 to 8°. At most 20% of the slice angle |
| `roughness` | `0.35` | 0.05 to 1 |
| `gloss` | `0.55` | 0 to 1 |
| `texture_strength` | `0.2` | 0 to 1. 0 turns the pattern off |
| `texture_scale` | `1` | 0.1 to 8. Higher values make the pattern finer |
| `texture_angle_degrees` | `0` | A finite angle, repeating every 360° |
| `light` | `[-0.5, -0.6, 1.0]` | A direction vector. Each component -100 to 100, vector length at least 0.01 |
| `shadow` | `false` | Turns on the soft shadow |
| `hover_lift` | `0.08` | 0 to 0.25 of the radius. Vertical lift on hover and selection |
| `hover_brightness` | `0.04` | 0 to 0.5. Brightness increase on hover and selection |
| `outline.rim`, `.separators`, `.emphasis` | all `false` | Turn on the rim, slice boundaries and emphasis line |
| `outline.width` | `1` | 0 to 8 logical pixels |
| `outline.color` | RGBA `(0.08, 0.12, 0.2, 1)` | Each component 0 to 1, alpha allowed |

A donut's `inner_radius` is 0.1 to 0.85 of the outer radius. For a shape
without a hole, use `RadialKind::Pie`. `font_size` is 8 to 64 logical pixels
(default 16), the title follows the shared rich-text limits (at most 512
characters, 8 lines), and `value_suffix` is one line of at most 24
characters. Write any space before the suffix yourself (for example `" kg"`).

## Plot area

The radial layout area is selected, moved and resized as
`DecorationId::DataArea` in the `frame.decorations()` hit map
(`HitMap<RadialChart>`). The elements of this hit map compute their
positions from the chart passed in, so pass the chart the frame was prepared
from. `decorations.data_area = None` lays the chart out automatically, and a
logical area that is set also applies to PNG output. The picture is not
stretched: circles, connector lines and labels are computed again in the new
area. See
[the 8 handles, constraints and host call order](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#그리기-영역-선택과-크기-조정).

## Labels and updates

`None`, `Inside` and `Outside` are supported. Inside labels are placed on
their slice; outside labels are split left and right and placed with
connector lines. A connector line is part of the outside label. It runs from
the slice to the label text, and when the label moves its start stays on the
slice while the rest follows the text. Inside labels use white when the slice
color is dark and the chart's `label_color` otherwise. A slice's
`label_color`, when set, takes precedence over the automatic contrast color.
An item name is one line of at most 80 characters.

With the same model, logical size, scale and font registration state, frames
and GPU resources are reused. Changing values or styles makes a new
immutable frame. Changes that do not move labels, such as hover, selection,
material and outline, share the label texture. Changing a slice's color
under an outside label does not require regenerating labels either, but a
change that can alter the automatic contrast color of an inside label
updates the labels. A frame already handed out does not change, so the
state of a screen being prepared and of a new screen are never mixed.
Invalid input or an exceeded budget does not overwrite the existing frame
either.

Labels follow the same font registration and fallback order as the other
charts. A character missing from the given font is looked up, per character,
in the registered fonts, the bundled Liberation Sans and the installed system
fonts (native only), in that order. The browser does not look up system
fonts, so to show Korean text register a Korean font with
`register_font_bytes`.

## Editing the model and selection

Change colors directly in `RadialSlice.color`. `slice.style:
Option<RadialStyle>` replaces that slice's whole style; `None` inherits the
chart style. Once a per-slice style exists, changes to the chart style no
longer reach that slice. A per-slice style also overrides tilt, depth and
light, so to change the camera angle together, update `tilt_degrees` in both
the shared style and the per-slice styles. Set it back to `None` to inherit
again. `labels`, `label_format` and `label_color` can also be overridden per
slice. `slice.labels = None` inherits, and `Some(RadialLabels::None)` hides
that slice's label.

```rust,no_run
use renderer::radial::*;
use renderer::{Color, Material};
let mut chart = RadialChart {
    slices: vec![RadialSlice::new("Search", 40.0, Color::from_rgb8(51,132,245))],
    ..Default::default()
};
chart.label_format = RadialLabelFormat::NamePercent;
chart.label_decimals = 1; // 0..6; trailing zero decimals are removed
chart.value_suffix = " units".into();
let target = RadialTarget::main(0);
let override_style = RadialStyle {
    material: Material::SatinMetal,
    inner_corner: 0.05,
    outer_corner: 0.05,
    ..chart.style.clone()
};
let slice = chart.slice_mut(target).unwrap();
slice.color = Color::from_rgb8(195,80,210);
slice.style = Some(override_style);
slice.label_format = Some(RadialLabelFormat::NameValuePercent);
chart.interaction.selected = Some(target);
```

`RadialLabelFormat` offers `Name`, `Value`, `Percent`, `NamePercent`,
`NameValue`, `ValuePercent` and `NameValuePercent`. The content and the
`None/Inside/Outside` position are set independently. Percentages use the
total of each circle, and values use the original unit. Percentages in the
detail circle are relative to the parent subtotal. Changing the format does
not change the original values or slice angles.

The renderer draws the string written in a slice's `text` (at most two
lines, joined with `'\n'`) as it is. `format_labels()` formats the values and
computes the percentages: it rewrites the `text` of every slice and detail
circle slice from the format, decimals, suffix and per-circle totals. Call it
after changing values, names, the format, the decimals or the suffix. The
host may set `text` directly, but calling `format_labels()` again overwrites
that string too. For older documents without `text`, `normalize_labels()`
fills only the empty `text` fields, and the same strings as before are drawn.
If a slice with a value and a visible label has no `text`, `prepare` rejects
it. One line is at most 400 characters and cannot contain control
characters.

A slice's `text_generated` (#313) tells whether its `text` is a generated
label. Strings written by `format_labels()`, `normalize_labels()` and
`regenerate_labels()` are `true`; strings written by the host are `false`.
`regenerate_labels()` rewrites only labels whose `text_generated` is `true`
and missing labels, and leaves the host's labels alone. Changing one value
changes its circle's total, so the percentages of generated labels change
together on the other slices of the same ring, the children of a sunburst
node and the detail slices of a split circle. `label_lines()` decides the
percentages and rounding. When the host writes a string directly, it writes
it with `RadialSlice::set_text(Some(..))` or leaves `text_generated` at
`false`. `set_text(None)` clears the string so that the next call writes a
generated label again. To rewrite the host's strings as well, call
`format_labels()`. The browser wrapper runs `regenerate_labels()` when it
reads a document (`create`, `set_document`), and
`regenerate_labels(rewriteUserText)` requests the same work directly.
Documents saved before `text_generated` existed read every stored string as
the host's.

`frame.pick([x, y])` takes **logical pixels** inside the chart, submits a
request that picks on the GPU the slice actually visible at that position,
and returns a `RadialPickTicket`. `ticket.resolve().await` returns that
slice's `RadialTarget`; native hosts can use `resolve_blocking()`. It picks
the slice visible on screen, accounting for tilt, rounded corners, gaps, lift
and occlusion. `group=0` is the main circle and `group=1` the detail circle,
and `index` is the original index, counting items of 0. Items of 0, the hole
and the background are not picked. A host that reorders the array or deletes
items must update the selection index too.

Hover is `interaction.hovered` with `hover_progress` (0 to 1), and click
selection is the separate `interaction.selected`.
`animate_hover(target, dt_seconds, duration_seconds)` takes elapsed time and
advances entering and leaving. When the pointer moves to another slice, the
previous slice comes down before the new one rises. A duration of 0 switches
immediately. Setting the style's `hover_lift` and `hover_brightness` to 0
turns those effects off. Click selection keeps its emphasis after the pointer
leaves, and the slice returns to normal when the selection is cleared. `pick_at_rest` looks up
the position before the lift. A host can use this result as a secondary test
only when it equals the previous hover, which keeps hover from toggling on
and off at an edge.

While hovering, outside labels stay in place, and inside labels rise with
their slice: the same label texture is drawn shifted by the lift. Text is not
enlarged, and earlier frames are not transformed again. Hover does not change
the original proportions or the chart size. PNG output includes the
selection and hover state of the model passed in. For a PNG without
transient state, reset `interaction` to `Default::default()` on a copy of the
model.

The label selection `decorations.selected` is separate from the slice
selection `interaction.selected`. Updating only the UI's selection list does
not change the renderer's slice selection state.

```rust,no_run
# async fn export_selected(renderer: &mut renderer::RadialRenderer, chart: &renderer::RadialChart)
# -> Result<Vec<u8>, Box<dyn std::error::Error>> {
// Take a snapshot that includes the current selection and hover progress.
let snapshot = chart.clone();
let image = renderer.export_rgba(&snapshot, (1000, 600), renderer::ExportOptions::document(2.0)).await?;
let png = renderer::encode_png(&image)?;
# Ok(png)
# }
```

With hover alone, the lift can be released when the pointer moves to an
export button. To keep a clicked slice raised, keep
`interaction.selected = Some(target)`. To keep a shape from the middle of an
animation, copy the model at the moment you want, and do not reset
`interaction` or set `hover_progress` to 0 before exporting.

A raster export reuses the frame `prepare` cached when it was prepared for
the same chart, size and scale. At another size or scale it builds a frame
for that export alone and drops it afterwards: the frame drawn on screen,
its label texture and its annotation layer stay cached, so the next
`prepare` at the screen's size and scale returns the same frame (#469).

`export_rgba_with_background(&chart, size, options, background)` and
`export_png_with_background` (native synchronous version
`export_rgba_with_background_blocking`) export with only the background
changed (#313). `RadialExportBackground::Document` is the same as
`export_rgba`; `Transparent` draws only the slices with their shadows, the
labels and the annotations over transparent pixels; and `Color(c)` paints
`c` (each component 0 to 1) where the document background would be. The
choice applies to that one export only. It does not modify the model and
leaves the frame used on screen and the renderer caches as they are. The
frame's slice table and label texture are used as they are, and a separate
112-byte uniform with only the background changed is made for the duration
of the export. A color with a component outside 0 to 1 is rejected with
`RadialError::Invalid`.

`export_svg(&chart, size, options)` and `export_pdf` (native synchronous
versions `export_svg_blocking` / `export_pdf_blocking`) export the same
snapshot as SVG and PDF documents. Size and scale are checked by the same
rules as `prepare`. The slices and background go in as one raster image. The
title, the labels and their leader lines, the lines joining the split circle,
and the annotations go in as vectors, with text kept as glyph outlines. Two
things differ from PNG. All inside labels are drawn at their lifted position
above every slice (in a PNG, a lifted neighboring slice can cover a label).
The selection box of a selected decoration is not drawn. The page size is the
logical size converted at 1 px = 0.75 pt.

`rim` (the outer and hole outline), `separators` (slice boundaries) and
`emphasis` (the hover and selection emphasis) in `style.outline` can each be
turned on or off. On a 3D chart, `rim` also includes the boundary between the
visible top and the outer and inner walls. `separators` also apply to the
slice boundaries between the cut faces and the top. Color, alpha and
thickness in logical pixels are adjustable, and lines on hidden back faces
are not drawn through. With a bevel, the lines follow the middle of the
rounded edge. A shared boundary between adjacent slices is drawn half its
width inside each slice. Labels and connector lines do not receive material
shading.

### Host update order

Subtract the chart's on-screen origin from the pointer coordinates and
convert by the DPR to get **logical coordinates inside the chart**. If the
UI already provides logical coordinates, do not divide by the DPR again. Pick
with the frame currently displayed, change the model's interaction state, and
then prepare a new frame.

```rust,no_run
use renderer::{RadialChart, RadialError, RadialFrame, RadialRenderer};
use std::sync::Arc;

async fn update_hover(
    renderer: &mut RadialRenderer,
    chart: &mut RadialChart,
    displayed: &Arc<RadialFrame>,
    pointer: Option<[f32; 2]>, // logical coordinates inside the chart; None outside the area
    elapsed_seconds: f32,
    logical_size: (u32, u32),
    scale: f32,
) -> Result<Arc<RadialFrame>, RadialError> {
    let target = match pointer {
        Some(p) => match displayed.pick(p)?.resolve().await? {
            Some(hit) => Some(hit),
            None => displayed
                .pick_at_rest(p)?
                .resolve()
                .await?
                .filter(|t| Some(*t) == chart.interaction.hovered),
        },
        None => None,
    };
    chart.interaction.animate_hover(target, elapsed_seconds, 0.16);
    renderer.prepare(chart, logical_size, scale)
}
```

The host must request frames during an animation. Entering and leaving
continue until they finish even when the pointer stops; the renderer runs no
timer of its own. Set click selection with
`chart.interaction.selected = displayed.pick(position)?.resolve().await?` and
clear it with `None`. Submit the command buffer that drew the new frame,
release the previous frame, and then call `end_frame()`. Size, DPR and
content changes use the same `prepare` path. Calling `clear_cache()` on
simple UI edits is not needed, because it also discards reusable labels.

### Saving and compatibility

With the `serde` feature of `figgy-renderer` (or of `figgy-model` when only
the model is used), the model can be serialized. A slice's `style`,
`labels`, `label_format` and `label_color`, and the chart's `decorations`,
`label_format`, `label_decimals`, `value_suffix` and `interaction` are read
as defaults when an older snapshot lacks them. The chart's `chart_title`,
`active_layer` and `annotations`, and a slice's `text`, are also read as
defaults when missing, and are not saved when they hold the default.
`chart_title` is the shared chart title `ChartTitleOptions`, but it is saved
as the same `{ "visible", "text" }` object as before. Its position lives in
`decorations`, so `offset_x`, `offset_y` and `top_margin` must be 0. Missing
fields of `RadialStyle` use their defaults too. Documents from before 0.19
also have a plain string `title`. When `chart_title` is missing, this string
is read as a `chart_title` with the document's `font_family`, 1.5 times
`font_size`, and `label_color`; when `chart_title` is present, `title` is
ignored. Saving writes only `chart_title`. Check both the saved model and the
restored model with `validate`. To avoid saving transient hover and
selection, reset `interaction` on the copy you save.

A slice's `text_generated` (#313) is also read as `false` when missing, and
is not saved when it is `false`.

JSON compatibility for missing fields differs from Rust source
compatibility. Older `RadialSlice` struct literals need the new fields, so
prefer `RadialSlice::new`. For charts and styles, use `..Default::default()`
or a copy of an existing value. `Material` is not `#[non_exhaustive]`, so
when materials are added an exhaustive `match` no longer compiles. Add the
new materials to the match when upgrading.

## Native editor example

Run it from the repository root; it is in the [egui example crate](https://github.com/shim9610/figgy/blob/master/examples/egui).

```bash
cargo run --locked --manifest-path examples/egui/Cargo.toml --example radial_editor
```

Click a slice or select it in the list on the left, then change its color,
per-slice style and label content. `Whole chart` edits the shared settings
and `Individual style` edits the selected slice's settings. The editor starts
from `RadialStyle::three_dimensional()`, so the tilt is 55°, and the
gallery's 3D scenes also use 55°. 0° looks straight down from above; higher
values show more of the side faces. The still image and hover use the same
view. `Inherit chart labels` clears the per-slice label settings. The editor
draws frames directly into a texture sized to the window and DPR.

## Resource lifetime and tests

GPU memory accounting and the budget belong to the device bundle
(`RendererDevice`) the renderer was created from, and every renderer made
from the same bundle shares them. By default there is no budget; set one with
`set_memory_budget(Some(bytes))`. Each frame uses a 112-byte uniform and a
176-byte table entry per visible slice. Each slice pick (`pick`) creates a
16-byte query and result buffer and a 16-byte readback buffer, and releases
them after the result is read. When the label layout changes, an RGBA label
texture of the actual output size is created; with the same layout it is
shared. Export adds the output target and an aligned readback buffer. Total
usage and the budget are checked before each GPU allocation.
`gpu_memory_usage()` counts both live resources and resources waiting for
GPU completion. It shares the accounting with any `Renderer` made from the
same `RendererDevice` bundle, so reading it from any renderer gives the total
for the whole bundle. To draw radial charts on the device of an existing
Cartesian `Renderer`, get its bundle with `renderer.renderer_device()` (#313)
and pass it to `RadialRenderer::try_new`. The two renderers share the device,
the queue, the device-loss record, the accounting and the budget. Releasing
one renderer or the bundle does not destroy the device; it lives as long as
an owner remains. If the device is lost, the next wait or export of both
renderers returns a loss error. In the browser,
`FiggyBoundedChart.create_on(parent, ...)` creates a radial chart on this
bundle.

Each immutable frame can create a new small uniform and slice table, so
release frames you are done with, submit or discard all related command
buffers, and then call `end_frame()`. Hovering does not keep growing
resources the size of the label resolution. Native hosts handle completion
callbacks by polling the device; on the web the browser handles them.
Release unneeded caches with `clear_cache()`. While an
`Arc<RadialFrame>` is kept elsewhere, its resources stay alive.

```bash
cargo test --locked -p figgy-model --features serde
cargo test --locked -p figgy-renderer --test radial_render -- --test-threads=1
cargo test --locked -p figgy-renderer --doc
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
```

If a native GPU cannot be requested, the pixel tests fail instead of
passing.

## Selecting and moving the title and labels

Separately from slice picking (`pick`), `frame.decorations()` provides a
`HitMap<RadialChart>` for the title, the plot area and the slice labels. Hit
areas, selection borders, drag limits and on-screen emphasis all match the
layout drawn on screen. Update `chart.decorations` with the same
`Selectable` and `Draggable` contract as Cartesian charts, then `prepare`.
Frame methods such as `frame.decoration_at(&chart, [x, y])` and
`drag_decoration(&mut chart, hit, dx, dy)` wrap these calls. The title sits
in the middle of the title band at the top of the chart; when it is wider
than the canvas, it is clipped to the canvas, and its hit area, selection
border and drag limits are then the part that overlaps the canvas. To return
a moved decoration to its original place, call
`chart.decorations.reset(&id)`. It clears the stored offset (for `DataArea`,
it also clears the set plot area) and, unlike dragging it back, leaves no
value behind. In the browser, `reset_decoration_position(id)` (#313) wraps
this call.

`frame.drag_decoration_snapped(&mut chart, hit, dx, dy, &mut snap)` is
`drag_decoration` with snapping (#465). The title or a slice label snaps
its drawn box to the centres of the canvas and the plot area and to the
edges and centres of the other decorations, within
`layout::SNAP_THRESHOLD_PX` (6 px on screen). The plot area does not snap.
`snap` is a `layout::DragSnap` the host keeps from press to release
(`DragSnap::new(display_scale)`; `set_enabled(false)` while a modifier is
held, `release()` at the end). Pass `snap.guides()` to
`renderer.prepare_with_guides(chart, size, scale, guides)` (and
`prepare_annotations_with_guides` for the annotation layer) to draw the
guides with the selection box. `prepare` and `prepare_annotations` draw no
guides, and exports never draw them. In the browser, `FiggyBoundedChart`
snaps `drag` and `drag_annotation` between `begin_drag()` and
`end_drag()`; `set_snap_modifier(held)` turns snapping off while the host
holds a modifier key. For coordinate units, per-element move rules, caching, save and
restore, and the web bindings, see the
[shared interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md).

### Annotation layer

The text boxes and arrows in `chart.annotations` are drawn over the frame in
a separate pass. A radial chart has no value axis, so only `Anchor::Ratio`
(a fraction of the plot area) can be placed; `Anchor::Data`, reference lines
and bands are skipped. Each skipped item is recorded in
`BoundedAnnotationFrame::render_warnings()` as
`RenderWarning::AnnotationSkipped { index, reason: AnnotationSkip::NoValueAxis }`.
The `BoundedAnnotationFrame` returned by
`renderer.prepare_annotations(chart, size, scale)` is drawn with
`draw_overlay` after `frame.draw`, and its hit map comes from
`annotations()`. Bands are skipped, so there is no layer drawn below the
slices. Editing annotations and selecting with
`decorations.selected = Annotation(i)` do not rebuild the frame or the label
raster. In the browser, `FiggyBoundedChart.annotation_preview(json)` reports
where an item that is not added yet would be drawn, or why it would be
skipped, without changing the document or the selection
([#334](https://github.com/shim9610/figgy/issues/334)). For the full contract, see the
[shared interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer).

## 한국어 사용법

`RadialChart`는 소수의 범주와 구성 비율을 보여 주는 파이·도넛 차트다. `RadialRenderer`는
직교 좌표 `Renderer`와 같은 wgpu 장치·큐를 사용할 수 있다. 원형 차트는 컬럼을 쓰지 않는다.
`ColumnSource`와 스트리밍이 없고, 조각의 값은 이름·색·분리 거리처럼 모델에 적는 조각의 속성이다.
축과 자동 맞춤도 사용하지 않는다. 범주 값과 스타일을 모두 담은 모델을 넘기면 불변 프레임을 만든다.
브라우저에서는 브라우저용 제품인 `crates/web`의 `FiggyBoundedChart`가 이 렌더러로 원형 차트를 그리고
히트맵·선택·드래그·PNG 출력을 노출한다([WASM 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host)).

원형 차트 API는 crates.io의 `figgy-renderer` 0.16.1부터 들어 있다. 비동기 생성자는 0.17.0부터
`RadialRenderer::new_async`로 쓸 수 있었고, 0.19.0에서 이름이 `RadialRenderer::try_new_async`로 바뀌었다.
[주석 층](#주석-층)과 차트의 `chart_title`·`active_layer` 필드는 0.18.0부터 쓸 수 있다.

### API 구성

모델 타입은 `model::radial`에 정의하며 `renderer::radial`과 renderer crate 루트에서도
가져올 수 있다. 모델을 편집한 뒤 `prepare`를 다시 호출하면 된다. 별도의 색 변경 명령이나
캐시 무효화 호출은 필요하지 않다.

| 타입 / 메서드 | 역할 |
|---|---|
| `RadialChart` | 값·모양·공통 스타일·라벨·상호작용 상태를 모두 담은 모델 |
| `RadialSlice`, `RadialSplit` | 조각 데이터와 한 조각을 펼친 상세 원 |
| `RadialStyle`, `RadialOutline` | 재질·입체 형상·질감·외곽선 설정 |
| `RadialLabels`, `RadialLabelFormat` | 라벨 위치와 표시할 내용 |
| `RadialChart::format_labels()` | 각 조각의 그릴 라벨 문자열(`RadialSlice::text`)을 편집 시점에 기록 |
| `RadialChart::regenerate_labels()` | 자동으로 만든 라벨(`text_generated`)과 빠진 라벨만 다시 기록하고 호스트가 쓴 라벨은 둔다(#313) |
| `RadialTarget`, `RadialInteraction` | 조각 식별과 호버·선택 상태 |
| `RadialChart::validate()` | 값과 옵션 검사. 실제 라벨 배치는 `prepare`에서 추가 검사 |
| `RadialRenderer::prepare()` | 모델을 읽고 불변 `Arc<RadialFrame>` 반환 |
| `RadialFrame::draw()` | 호스트가 연 render pass에 그리기 |
| `RadialFrame::pick()` | 해당 프레임에서 보이는 조각을 GPU에서 고르는 요청. `RadialPickTicket::resolve()`가 결과를 돌려준다 |
| `export_rgba()` / `export_png()` / `export_svg()` / `export_pdf()` | 현재 모델을 그려 반환. 배율과 배경은 `ExportOptions`로 정한다 |
| `export_rgba_with_background()` / `export_png_with_background()` | 문서 배경 대신 투명이나 다른 색 위에 내보낸다(`RadialExportBackground`, #313) |
| `clear_cache()` / `end_frame()` | 내부 캐시 참조 해제 / 제출이 끝난 자원의 회수 예약 |

`RadialChart::default()`는 빈 파이 차트이므로, 그리기 전에 양수 값을 가진 조각을 추가해야 한다.

### 시작하기

```rust,no_run
use renderer::{
    Color, Material, RadialChart, RadialKind, RadialLabels,
    RadialRenderer, RadialSlice, RendererDevice, encode_png,
};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let instance = renderer::gpu::create_instance();
# let adapter = renderer::gpu::request_adapter_blocking(&instance)?;
# let (device, queue) = renderer::gpu::request_device_blocking(&adapter)?;
let mut renderer = RadialRenderer::try_new(
    RendererDevice::new(device, queue)?,
    wgpu::TextureFormat::Rgba8Unorm,
)?;
let mut chart = RadialChart {
    slices: vec![
        RadialSlice::new("A", 30.0, Color::from_rgb8(28, 179, 155)),
        RadialSlice::new("B", 70.0, Color::from_rgb8(251, 128, 74)),
    ],
    kind: RadialKind::Donut { inner_radius: 0.48 },
    labels: RadialLabels::Outside,
    ..Default::default()
};
// 일반 텍스트 제목은 지금의 차트 글꼴, `font_size`의 1.5배, `label_color`로 만든다.
chart.set_title_text("Regional share");
chart.style = renderer::RadialStyle::three_dimensional();
// 렌더러는 라벨 문자열을 그리기만 한다. 값이나 표시 형식을 바꾼 뒤에는 다시 기록한다.
chart.format_labels();

// 논리 크기는 1000×600, 실제 출력은 2000×1200이다.
// 글자와 도형을 이 해상도로 다시 그린다.
let image = renderer.export_rgba_blocking(&chart, (1000, 600), renderer::ExportOptions::document(2.0))?;
std::fs::write("donut.png", encode_png(&image)?)?;
renderer.end_frame();
# Ok(())
# }
```

화면에 그릴 때는 `prepare(&chart, logical_size, scale)`로 `Arc<RadialFrame>`을 받은 뒤,
호스트의 render pass 안에서 `frame.draw(&mut pass)`를 호출한다. 타깃은 생성자에 지정한
RGBA8/BGRA8 포맷, `frame.size()` 크기, sample count 1이어야 한다. 가장자리 안티에일리어싱은
렌더러가 직접 처리한다. 실제 픽셀마다 2×2 표본을 두고, 각 표본은 자기 위치에서 맞는 조각(없으면 배경)과
그 조각의 외곽선을 찾는다. 표본이 맞은 면(같은 조각, 같은 법선)마다 재질 조명은 픽셀당 한 번 계산하고
그 면이 덮은 표본 수만큼 반영하며, 부드러운 그림자도 픽셀당 한 번 계산한다(#349). 모든 표본이 같은 색을 낼 픽셀(평평한 윗면 한 곳의 안쪽이나 배경)과, 조각이 평평한 윗면뿐인 차트(깊이가 없거나, 기울지 않고 베벨이 없는 차트)의 가장자리 픽셀은 광선 하나로 그린다. 표본들이 낼 색을 그대로 쓰거나, 조각 경계와 외곽선의 해석적 피복을 쓴다(#41, #454). 베벨, 꺾인 모서리, 기울어진 옆면은 표본을 그대로 쓴다. `scale`은 0.5 이상이며 타깃의 실제 크기도 함께 맞춘다. 상한은 없고, 실제 크기가 장치의 텍스처 크기 한도를 넘으면 오류를 돌려준다.
호스트 render pass의 clear/load 정책은 호스트가 정한다. 투명 배경도 지원한다.
논리 크기는 가로·세로 각각 최소 160픽셀이다. 실제 크기는 `round(논리 크기 × scale)`이며
장치의 텍스처 크기 한도를 넘을 수 없다. `draw`는 viewport와 scissor를 타깃 전체로 설정한다.
UI의 일부 영역에 넣으려면 이 크기의 별도 텍스처에 그린 뒤 호스트가 배치한다.

WASM 생성에는 `RadialRenderer::try_new_async(gpu, format).await`를 사용해 브라우저
파이프라인 컴파일을 기다린다. 이후 같은 `prepare`/`draw`를 쓰고, 출력에는
`export_rgba(...).await`나 `export_png(...).await`를 사용한다.
네이티브에서는 비동기 함수를 그대로 기다려도 되고(`RendererDevice::new`로 만든 장치 묶음이면 호스트가 장치를 poll하지 않아도 완료된다), `export_rgba_blocking` 같은 `_blocking` 함수로 호출한 스레드에서 기다려도 된다.

### 값과 분할 원

- 원마다 1~64개 항목을 받는다. 값은 유한한 0 이상의 수여야 하고, 전체 합은 양수여야 한다.
- 0인 항목은 모델에 남지만 도형과 라벨은 만들지 않는다. NaN·무한대·음수·합계 오버플로는 거절한다.
- 조각 색은 불투명 RGBA다. 배경과 라벨에는 알파를 사용할 수 있다.
- 각도는 오른쪽을 0도로 보고 시계 방향으로 증가한다. 기본 시작 각도는 -90도다.
- `explode`는 바깥 반지름 대비 조각 이동량이다. 0~0.35를 지원한다.
- `RadialSplit`은 한 항목을 오른쪽의 작은 원으로 펼친다. 자식 값은 부모와 **같은 단위**이며
  합계가 부모 값과 같아야 한다(상대 오차 1e-9까지 허용). 자식 원의 라벨은 부모 소계 대비 비율이다.

예를 들어 전체 100 중 ‘기타’가 13이고, 세부 비율이 16%·14%·70%라면 자식 값은
`2.08`, `1.82`, `9.1`이다. `16`, `14`, `70`을 그대로 넣으면 합계가 맞지 않아 오류가 난다.
`slice_index`는 0인 항목까지 포함한 원본 배열 인덱스다. 연결선은 선택한 부모 조각의 두
각도 경계와 상세 원을 잇는다. 예제처럼 부모 조각을 오른쪽에 두면 두 원의 관계가 잘 보인다.

#### 선버스트(중첩 고리)

조각에 `children`을 두면 차트는
선버스트가 된다. 트리의 깊이 `d`만큼 구멍(`kind.inner_radius()`, 파이는 0이므로 가운데 고리는
원판)과 바깥 반지름 사이를 같은 두께의 고리 `d`개로 나누고, 깊이 `k`의 노드는 고리 `k` 위에서
부모의 각도 구간 가운데 자기 몫(부모 값 대비 자기 값, 자식 순서대로)만큼을 차지한다. 자식 값의
합은 부모 값 이하여야 하며(상대 오차 1e-9), 남는 구간은 자식 고리에서 비워 둔다. `children`은
비어 있으면 JSON에서 생략한다.

- `RadialTarget { group, index }`에서 `group`은 고리(0이 가운데), `index`는 그 고리의 노드를
  문서 순서(부모 순서대로 자식들)로 센 위치이며 값이 0인 노드도 포함한다. `slice()`,
  `slice_mut()`, `step_selection`(가운데 고리부터 바깥쪽으로, 값이 있는 노드만), 라벨의
  `DecorationId::RadialLabel`과 `radial_label_{group}_{index}` 이름이 모두 이 규칙을 따른다.
  `group_count()`, `group_len(group)`, `node_count()`, `node_span(target)`가 고리와 구간을
  알려 준다.
- 라벨의 백분율은 부모 값 대비 자기 몫이다(가운데 고리는 전체 합 대비). `format_labels()`가
  모든 고리의 `text`를 쓴다.
- `Outside` 라벨은 가장 바깥 고리에만 놓인다(파이와 같은 좌우 쌓기, 바깥 반지름의 1.015 지점에서
  연결선 시작). 안쪽 고리의 `Outside`는 자기 띠 안에 내부 라벨로 그린다. 내부 라벨은 띠의 가운데
  반지름에 놓인다. 크기는 조각 자신의 좌표계에서 맞춘다(#357). 이등분선 방향 길이는 띠 두께 안에,
  그와 수직인 방향 길이는 라벨 중심을 지나는 호 안에 들어가야 하고, 바깥 모서리는 테두리 안에 있어야
  한다. 그래서 얇은 고리의 위나 아래에 놓인 가로 라벨은 원래 크기를 지키고, 옆에 놓인 라벨은 두께에
  맞춰 줄어든다. 라벨은 가장 크게 들어가는 곳에 놓인다(#357). 넓은 조각에서는 그 크기가 5% 이상
  커지면 이등분선에서 호를 따라 위나 아래로 옮긴다. 평면 차트에서 가로로 원래 크기를 지킬 수 없는 라벨은
  더 크게 들어가면 y축 제목처럼 아래에서 위로 세워 쓴다. 그래서 얇은 고리 옆의 넓은 조각도 라벨을 지킨다.
  라벨은 12 논리 픽셀(차트의 `font_size`가 더 작으면 그 값)보다 작아지지 않는다. 그보다 작아져야 하는
  라벨은 조각이 가장 바깥 고리(또는 파이·도넛)에 있으면 연결선과 함께 바깥으로 옮긴다. 원은 다른 바깥
  라벨처럼 자리를 내주고, 그 라벨은 바깥 라벨과 똑같이 맞히고 선택하고 끌 수 있다. 연결선이 바깥 고리를
  넘어갈 수 없는 안쪽 선버스트 고리에서는 숨긴다. 그리지도 맞히지도 않는다. `inner_label_fallback`을
  `InnerLabelFallback::Outside`로 두면 숨기지 않고 바깥으로 꺼낸다. 연결선은 자기 고리 테두리 바로 바깥에서
  시작해 바깥 고리들을 가로질러 원 옆의 꺾임점까지 가고, 라벨은 가장 바깥 고리의 바깥 라벨과 각도 순으로
  함께 쌓여 서로 겹치지 않는다. 그다음부터는 호버, 클릭 선택, 부분별 윤곽선, 드래그(연결선이 따라감),
  PNG·SVG·PDF, serde(`Hide`이면 키 생략), WASM 모두 바깥 라벨과 같다.
- 평면에서만 그린다. 차트 스타일이나 노드 스타일의 `tilt_degrees`·`depth`가 0이 아니면
  거부한다(`"a sunburst draws flat: no tilt or depth"`). `split`과 함께 쓸 수 없다
  (`"a sunburst has no detail circle"`).
- 한계: 고리는 최대 8개(`MAX_SUNBURST_DEPTH`), 고리 하나의 노드는 최대 64개
  (`MAX_RADIAL_SLICES`)다. 그 밖의 거부 사유: `"a node's children sum to at most its value"`,
  `"a sunburst has at most 8 rings"`, `"a sunburst ring has at most 64 nodes"`.
- 피킹·강조·호버·재질(고리마다 다른 결)은 모두 조각과 같다.
- `RadialStyle::sunburst()`는 웹 선버스트 차트의 평면 모양이다. 깊이·간격·둥근 모서리가 없고,
  1px 흰색 테두리와 구분선을 그린다. 그래서 한 가지가 같은 색이어도 노드와 고리가 서로 구분된다.
  배경이 흰색이 아니면 `outline.color`를 배경색으로 바꾼다. 이 모양이 호스트 기본값이다(#357).
  `RadialStyle::default()`와 `RadialChart::default()`가 이 모양을 그리므로 테마 없는 차트에도
  구분선이 보인다. 스타일을 생략한 문서가 읽는 serde 기본값은 윤곽선이 없으므로 저장된 문서는
  예전 그대로 그려진다. `RadialOutline::default()`는 문서화된(꺼진) 윤곽선을 유지한다. `child_tint`가 있는 테마(Gallery,
  cool slate)는 가지마다 맨 위 조각의 색을 쓰고, 바깥 고리로 갈수록 옅게 칠한다.

```rust
use renderer::radial::*;
use renderer::Color;
let mut chart = RadialChart::default();
let mut ops = RadialSlice::new("Ops", 60.0, Color::from_rgb8(220, 60, 40));
ops.children = vec![
    RadialSlice::new("Fleet", 40.0, Color::from_rgb8(60, 120, 220)),
    RadialSlice::new("Crew", 10.0, Color::from_rgb8(40, 170, 70)),
]; // 남은 10은 자식 고리에서 빈 구간
chart.slices = vec![ops, RadialSlice::new("R&D", 40.0, Color::from_rgb8(80, 80, 80))];
chart.format_labels();
assert_eq!(chart.group_count(), 2);
assert_eq!(chart.slice(RadialTarget { group: 1, index: 1 }).map(|s| s.label.as_str()), Some("Crew"));
```

### 재질과 입체 표현

모델 기본값은 정면에서 내려다본 평면 파이다. 재질을 바꾸는 것만으로 기울기나 깊이가
바뀌지는 않는다. 입체 기본 뷰는 `RadialStyle::three_dimensional()`로 지정한다. 세라믹 재질, 기울기 55°, 깊이 0.16, 베벨 0.025, 간격 1.4°, 안쪽·바깥 모서리 0.045, 그림자를 함께 설정하며, 각 필드는 이후 개별 편집할 수 있다. 도넛 모양은 별도로 `kind`에서 지정한다.

| 설정 | 표현 |
|---|---|
| `Flat` | 조명 없이 지정한 색을 그대로 사용하는 평면 표현 |
| `Matte` | 확산광과 약한 표면 입자 |
| `Ceramic` | 매끈한 표면, 밝은 반사광과 둥근 모서리 느낌 |
| `SatinMetal` | 넓게 퍼지는 반사광과 약한 결을 가진 새틴 금속 |
| `BrushedMetal` | 원주 방향의 미세한 결과 방향성 반사 |
| `Toon` | 세 단계 명암. 외곽선은 별도로 켜거나 끈다 |
| `Enamel` | 선명한 코팅 광택 |
| `Hatch` | 대각선 해칭 |
| `Pearl` | 곡면 가장자리의 약한 펄 색 변화 |
| `Paper` | 윗면·옆면에 이어지는 미세한 무광 종이 결 |
| `Wood` | 굵은 무늬 없이 미세하고 불규칙한 잔결을 가진 거친 무광 합판 |

#### Materials shared with bars and boxes

`Material` is one enum for sectors (`RadialStyle::material`), bars
(`CategoryBarStyle::material`) and boxes (`BoxPlotStyle::material`), and a
name has the same look in every family. A bar or a box is shaded like a
flat (zero depth, zero tilt) pie top under the default light. Its normal is constant, so its lighting
and highlights are uniform over the body; textures (grain, brush, paper,
wood, hatching) still vary across it.

| Material | Sectors | Bars and boxes |
|---|---|---|
| `Flat` | yes | yes; the bar default |
| `Matte` | yes | yes; the box default |
| `Ceramic` | yes | yes; `gloss` lifts the whole body by at most 2.5/255 |
| `SatinMetal`, `BrushedMetal` | yes | yes; a uniform sheen and a brush along the bar; at 1x a `texture_scale` above 0.13 has no visible effect |
| `Enamel` | yes | yes; `gloss` has no visible effect |
| `Paper`, `Wood`, `Hatch` | yes | yes |
| `Toon`, `Pearl` | yes | rejected by `validate`: "material not supported by bars" / "material not supported by boxes" |

`Material::supported_on_flat_body` tells the two apart. Bar and box styles
have no `roughness`, `light` or `texture_angle_degrees`; they use the
`RadialStyle` defaults (0.35, `[-0.5, -0.6, 1.0]` and 0°). One bar texture
unit is 150 logical pixels: a bar's texture matches that of a sector of
radius 150 px, and Matte grain cells are 1.67 px.

각 필드의 기본값과 허용 범위는 아래 '스타일 옵션 기본값과 범위' 표에 있다. 아주 작은 조각의 시각적 간격은 해당 조각 각도의 20% 이내로 제한한다.

입체 조각은 윗면·바깥 벽·안쪽 벽·절단면을 직교 투영으로 그린다.
`inner_corner`와 `outer_corner`는 평면에서 본 조각 끝의 둥근 정도다(바깥 반지름의 0~0.25).
0이면 각진 형태이며 2D·3D에 모두 적용된다. 작은 조각과 얇은 링에서는 반경을 자동으로
제한한다. `bevel`은 입체 조각의 위아래 모서리를 실제로 둥글게 만들며, 깊이의 절반 미만으로
제한한다. 깊이가 0이면 입체 bevel은 적용하지 않는다. 그림자는 조각마다 근사한 부드러운 그림자다. 재질에 외부 이미지는 필요 없다. 입체 투영을 사용해도
원래 각도와 비율 라벨은 유지하지만, 화면에 투영된 조각 면적은 기울기에 따라 달라진다.

`gloss`(0~1)는 반사광 강도, `roughness`는 퍼짐, `texture_angle_degrees`는 표면 무늬 방향을
조절한다. 같은 조명에서도 재질에 따라 적용 방식이 다르다. 물리 기반 렌더러 전체를 구현한
것은 아니며, 차트의 범주 색을 유지하는 스타일화된 셰이딩이다.
`Paper`와 `Wood`는 높이까지 이어지는 질감 좌표를 써서 옆면에 세로줄이 생기지 않는다.
출력 크기와 면의 기울기로 구분할 수 없는 미세 무늬는 약하게 줄여 깜빡임을 억제한다. `Flat`은 광택·질감을 사용하지
않고, `Toon`은 거칠기 대신 고정 명암 단계를 사용한다. `Wood`는 나이테나 규칙적인 줄무늬를
그리지 않는다. 조각의 원래 색을 유지하면서 잔결을 더한다. 거친 합판 표현은
`texture_strength = 0.7`, `roughness = 0.9`, `gloss = 0.1`부터 조정할 수 있다.

#### 스타일 옵션 기본값과 범위

비율로 표기한 길이는 **바깥 반지름**을 기준으로 한다. 범위를 벗어난 입력과 유한하지 않은
수는 거절한다. 범위 안에서도 작은 조각의 모서리·간격은 서로 겹치지 않도록 줄어들 수 있다.

| 필드 | 기본값 | 허용 범위 / 의미 |
|---|---|---|
| `material` | `Flat` | 위 재질 목록 |
| `tilt_degrees` | `0` | 0~65°. 0은 위에서 수직으로 내려다보는 시점 |
| `depth` | `0` | 반지름의 0~0.4 |
| `bevel` | `0.015` | 반지름의 0~0.1. 실제 적용은 깊이의 절반 미만 |
| `inner_corner`, `outer_corner` | 각각 `0` | 반지름의 0~0.25 |
| `gap_degrees` | `0` | 0~8°. 조각 각도의 20% 이내 |
| `roughness` | `0.35` | 0.05~1 |
| `gloss` | `0.55` | 0~1 |
| `texture_strength` | `0.2` | 0~1. 0이면 무늬 효과 끄기 |
| `texture_scale` | `1` | 0.1~8. 높일수록 무늬가 촘촘해짐 |
| `texture_angle_degrees` | `0` | 유한한 각도. 360° 주기로 반복 |
| `light` | `[-0.5, -0.6, 1.0]` | 방향 벡터. 성분별 -100~100, 벡터 길이 최소 0.01 |
| `shadow` | `false` | 부드러운 그림자 켜기 |
| `hover_lift` | `0.08` | 반지름의 0~0.25. 호버·선택 시 수직 이동 |
| `hover_brightness` | `0.04` | 0~0.5. 호버·선택 시 밝기 증가 |
| `outline.rim`, `.separators`, `.emphasis` | 모두 `false` | 윤곽·조각 경계·강조선 켜기 |
| `outline.width` | `1` | 논리 픽셀 0~8 |
| `outline.color` | RGBA `(0.08, 0.12, 0.2, 1)` | 성분별 0~1, 알파 허용 |

도넛의 `inner_radius`는 바깥 반지름의 0.1~0.85다. 구멍 없는 모양은 `RadialKind::Pie`로
지정한다. `font_size`는 논리 픽셀 8~64(기본 16)이고, 제목은 공통 리치 텍스트 한도(최대 512자,
8줄)를 따르며, `value_suffix`는 한 줄 24자 이내다. 접미사 앞의 공백도 직접 넣는다(예: `" kg"`).

### 영역 조정

원형 배치 영역은 `frame.decorations()` 히트맵(`HitMap<RadialChart>`)의 `DecorationId::DataArea`로
선택·이동·리사이즈한다. 이 히트맵의 요소는 넘긴 차트에서 위치를 계산하므로 프레임을 준비한 차트를 함께 넘긴다.
`decorations.data_area = None`이면 자동 배치하며, 지정한 논리 영역은 PNG에도 적용한다.
그림을 늘이지 않고 새 영역에서 원·연결선·라벨을 다시 계산한다.
[8개 핸들, 제약과 호스트 호출 순서](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#그리기-영역-선택과-크기-조정)를 참고한다.

### 라벨과 갱신

`None`, `Inside`, `Outside`를 지원한다. 내부 라벨은 조각의 위치에, 외부 라벨은 좌우로
나누어 연결선과 함께 배치한다. 연결선은 외부 라벨의 일부다. 조각에서 시작해 라벨 글자까지
이어지며, 라벨을 옮기면 시작점은 조각에 남고 나머지는 글자를 따라간다.
내부 라벨은 조각 색이 어두우면 흰색을, 그 밖에는 차트의 `label_color`를 사용한다.
조각의 `label_color`를 지정하면 자동 대비 색보다 우선한다.
항목 이름은 한 줄, 80자 이내다.

같은 모델·논리 크기·배율·글꼴 등록 상태에서는 프레임과 GPU 자원을 재사용한다. 값이나
스타일을 바꾸면 새 불변 프레임을 만든다. 호버·선택·재질·외곽선처럼 라벨 배치를 바꾸지 않는
변경에서는 라벨 텍스처를 공유한다. 외부 라벨의 조각 색 변경도 라벨 재생성을 요구하지 않지만,
내부 라벨의 자동 대비 색이 달라질 수 있는 변경은 라벨을 갱신한다. 이미 받은 프레임은 바뀌지 않으므로 준비 중인 화면과
새 화면의 상태가 섞이지 않는다. 유효하지 않은 입력이나 예산 초과도 기존 프레임을 덮어쓰지 않는다.

라벨은 다른 차트와 같은 글꼴 등록과 대체 순서를 따른다. 지정한 글꼴에 없는 글자는 글자마다 등록 글꼴,
내장 Liberation Sans, 설치된 시스템 글꼴(네이티브 전용) 순으로 찾아 그린다. 브라우저에서는
시스템 글꼴을 찾지 않으므로 한글을 표시하려면 `register_font_bytes`로 한글 글꼴을 등록해야 한다.


### 모델 편집과 선택

색은 `RadialSlice.color`에서 직접 바꾼다. `slice.style: Option<RadialStyle>`은 해당 조각의
스타일 전체를 덮어쓰며, `None`이면 차트 스타일을 상속한다. 개별 스타일을 만든 뒤에는 차트의
스타일 변경이 그 조각에 전파되지 않는다. 개별 스타일은 기울기·깊이·조명까지 덮어쓰므로,
카메라 각도를 함께 바꾸려면 공통 스타일과 개별 스타일의 `tilt_degrees`를 모두 갱신해야 한다.
다시 상속하려면 `None`으로 돌린다.
`labels`, `label_format`, `label_color`도 조각별로 덮어쓸 수 있다.
`slice.labels = None`은 상속이고, `Some(RadialLabels::None)`은 그 조각의 라벨 숨기기다.

```rust,no_run
use renderer::radial::*;
use renderer::{Color, Material};
let mut chart = RadialChart {
    slices: vec![RadialSlice::new("Search", 40.0, Color::from_rgb8(51,132,245))],
    ..Default::default()
};
chart.label_format = RadialLabelFormat::NamePercent;
chart.label_decimals = 1; // 0..6, 불필요한 소수점 0은 제거
chart.value_suffix = " units".into();
let target = RadialTarget::main(0);
let override_style = RadialStyle {
    material: Material::SatinMetal,
    inner_corner: 0.05,
    outer_corner: 0.05,
    ..chart.style.clone()
};
let slice = chart.slice_mut(target).unwrap();
slice.color = Color::from_rgb8(195,80,210);
slice.style = Some(override_style);
slice.label_format = Some(RadialLabelFormat::NameValuePercent);
chart.interaction.selected = Some(target);
```

`RadialLabelFormat`은 `Name`, `Value`, `Percent`, `NamePercent`, `NameValue`, `ValuePercent`,
`NameValuePercent`를 제공한다. 표기 내용과 `None/Inside/Outside` 위치를 독립적으로 설정한다.
백분율은 원별 합계, 값은 원래 단위를 사용한다. 세부 원의 백분율은 부모 소계를 기준으로 한다.
표기 변경은 원본 값과 조각 각도를 바꾸지 않는다.

렌더러는 조각의 `text`(최대 두 줄, `'\n'`으로 연결)에 적힌 문자열을 그대로 그린다. 값의 형식화와
백분율 계산은 `format_labels()`가 맡는다. `format_labels()`가 표시 형식·소수 자릿수·접미사·원별 합계로 모든 조각과 세부 원 조각의 `text`를
다시 기록한다. 값·이름·표시 형식·자릿수·접미사를 바꾼 뒤에 호출한다. 호스트가 `text`를 직접
지정해도 되며, `format_labels()`를 다시 호출하면 그 문자열도 덮어쓴다. `text`가 없는 이전 문서는
`normalize_labels()`로 비어 있는 `text`만 채우면 이전과 같은 문자열을 그린다. 값이 있고 라벨이
보이는 조각에 `text`가 없으면 `prepare`가 거부한다. 한 줄은 400자 이내이며 제어 문자를 쓸 수 없다.

조각의 `text_generated`(#313)는 그 `text`가 자동 라벨인지 알려 준다. `format_labels()`,
`normalize_labels()`, `regenerate_labels()`가 쓴 문자열은 `true`, 호스트가 쓴 문자열은 `false`다.
`regenerate_labels()`는 `text_generated`가 `true`인 라벨과 빠진 라벨만 다시 쓰고 호스트의 라벨은
그대로 둔다. 값 하나를 바꾸면 그 원의 합계가 바뀌므로 같은 고리의 다른 조각, 선버스트 노드의 자식,
분할 원의 세부 조각까지 자동 라벨의 백분율이 함께 바뀐다. 백분율과 반올림은 `label_lines()`가 정한다.
호스트가 직접 문자열을 쓸 때는 `RadialSlice::set_text(Some(..))`로 쓰거나 `text_generated`를 `false`로
둔다. `set_text(None)`은 문자열을 지워 다음 호출에서 자동 라벨을 다시 받게 한다. 호스트의 문자열까지
다시 쓰려면 `format_labels()`를 호출한다. 브라우저 래퍼는 문서를 읽을 때(`create`, `set_document`)
`regenerate_labels()`를 실행하고, `regenerate_labels(rewriteUserText)`로 같은 작업을 직접 요청할 수 있다.
`text_generated`가 생기기 전에 저장한 문서는 저장된 문자열을 모두 호스트의 것으로 읽는다.

`frame.pick([x, y])`는 차트 내부의 **논리 픽셀**을 받아 그 위치에 실제로 보이는 조각을
GPU에서 고르는 요청을 제출하고 `RadialPickTicket`을 돌려준다. `ticket.resolve().await`가
그 조각의 `RadialTarget`을 돌려주며, 네이티브 호스트는 `resolve_blocking()`을 쓸 수 있다.
기울기·둥근 모서리·간격·들림·가림 관계까지 화면에 보이는 조각을 고른다.
`group=0`은 큰 원, `group=1`은 세부 원이며 `index`는 값이 0인 항목도 포함한 원본 인덱스다.
값이 0인 항목, 구멍, 배경은 선택되지 않는다. 배열을 재정렬하거나 항목을 삭제한 호스트는
선택 인덱스도 함께 갱신해야 한다.

호버는 `interaction.hovered`와 `hover_progress`(0~1), 클릭 선택은 `interaction.selected`로
분리된다. `animate_hover(target, dt_seconds, duration_seconds)`는 시간을 입력받아 진입·이탈을
진행한다. 다른 조각으로 이동할 때는 이전 조각이 내려온 뒤 새 조각이 올라간다. duration이
0이면 즉시 전환한다. 스타일의 `hover_lift`와 `hover_brightness`를 0으로 두면 해당 효과를 끈다.
클릭 선택은 포인터가 나가도 강조를 유지하며, 다시 선택을 해제하면 복원된다.
`pick_at_rest`는 들리기 전 위치를 조회한다. 호스트는 이 결과가 직전 호버와 같을 때만
보조 판정에 사용해 가장자리에서 호버가 반복해서 켜지고 꺼지는 것을 막을 수 있다.

호버 중 외부 라벨은 제자리에 있고, 내부 라벨은 같은 라벨 텍스처를 들림만큼 옮겨 그려
조각과 함께 올라간다. 글자를 확대하거나 이전 프레임을 다시 변환하지 않는다. 원래 비율과
차트 크기는 호버로 바뀌지 않는다. PNG는 넘긴 모델의 선택·호버 상태를 그대로 포함한다.
일시 상태 없는 PNG가 필요하면 모델 복사본의 `interaction`을 `Default::default()`로 초기화한다.

라벨 선택인 `decorations.selected`는 조각 선택인 `interaction.selected`와 별개다. UI의 선택 목록만 갱신해도 렌더러의 조각 선택 상태가 자동으로 바뀌지는 않는다.

```rust,no_run
# async fn export_selected(renderer: &mut renderer::RadialRenderer, chart: &renderer::RadialChart)
# -> Result<Vec<u8>, Box<dyn std::error::Error>> {
// 현재 선택과 호버 진행도를 포함한 스냅샷을 잡는다.
let snapshot = chart.clone();
let image = renderer.export_rgba(&snapshot, (1000, 600), renderer::ExportOptions::document(2.0)).await?;
let png = renderer::encode_png(&image)?;
# Ok(png)
# }
```

호버만 사용하면 내보내기 버튼으로 포인터가 이동할 때 들림이 해제될 수 있다. 클릭해서
고른 조각을 계속 띄우려면 `interaction.selected = Some(target)`를 유지한다. 애니메이션 중간
모양까지 남기려면 원하는 시점에 모델을 복사하고, 내보내기 전에 `interaction`을 초기화하거나
`hover_progress`를 0으로 바꾸지 않는다.

래스터 내보내기는 같은 차트, 크기, 배율로 준비해 둔 `prepare`의 캐시 프레임을 그대로 쓴다. 크기나
배율이 다르면 그 내보내기만을 위한 프레임을 만들고 끝나면 버린다. 화면에 그리는 프레임과 그 라벨
텍스처, 주석 레이어는 캐시에 그대로 남으므로, 화면 크기와 배율로 다시 부르는 `prepare`는 같은
프레임을 돌려준다(#469).

`export_rgba_with_background(&chart, size, options, background)`와
`export_png_with_background`(네이티브 동기 버전 `export_rgba_with_background_blocking`)는
배경만 바꿔 내보낸다(#313). `RadialExportBackground::Document`는 `export_rgba`와 같고,
`Transparent`는 배경 없이 조각과 조각의 그림자, 라벨, 주석만 투명한 픽셀 위에 그리며, `Color(c)`는
문서 배경 자리에 `c`(각 성분 0~1)를 칠한다. 이 선택은 내보내기 하나에만 적용된다. 모델을 고치지 않고,
화면에 쓰는 프레임과 렌더러 캐시도 그대로 둔다. 프레임의 조각 표와 라벨 텍스처는 그대로 쓰고,
배경만 바꾼 112바이트 uniform을 내보내기 동안 따로 만든다. 성분이 0~1을 벗어난 색은
`RadialError::Invalid`로 거부한다.

`export_svg(&chart, size, options)`와 `export_pdf`(네이티브 동기 버전 `export_svg_blocking` /
`export_pdf_blocking`)는 같은 스냅샷을 SVG와 PDF 문서로 내보낸다.
크기와 배율은 `prepare`와 같은 규칙으로 검사한다. 조각과 배경은 래스터 이미지 한 장으로 들어간다.
제목, 라벨과 그 지시선, 분할 원을 잇는 선, 주석은 벡터로 들어가며 글자는 글리프
윤곽선으로 남는다. PNG와 다른 점은 두 가지다.
내부 라벨은 모두 들린 위치에서 모든 조각 위에 그린다(PNG에서는 들린 이웃 조각이 라벨을 가릴 수 있다).
선택한 장식의 선택 상자는 그리지 않는다. 페이지 크기는 논리 크기를 1 px = 0.75 pt로 환산한 값이다.


`style.outline`의 `rim`(바깥·구멍 윤곽), `separators`(조각 경계), `emphasis`(호버·선택 강조)는
각각 켜고 끌 수 있다. 입체 차트의 `rim`은 보이는 윗면과 바깥·안쪽 벽 사이의 경계도 포함한다.
`separators`는 절단면과 윗면 사이의 조각 경계에도 적용된다. 색·알파·논리 픽셀 두께를 조절하며
숨은 뒷면의 선을 투과해 그리지 않는다. 베벨이 있으면 둥근 모서리의 중간을 따른다.
조각이 맞붙은 공유 경계는 각 조각 안쪽에 반 폭씩 그린다. 라벨과 연결선은 재질 셰이딩을 받지 않는다.

#### 호스트의 갱신 순서

포인터 좌표에서 차트의 화면상 원점을 빼고 DPR로 변환해 **차트 내부 논리 좌표**를 구한다.
UI가 이미 논리 좌표를 제공하면 DPR로 다시 나누지 않는다. 현재 표시 중인 프레임으로 선택을
판정하고, 모델의 상호작용 상태를 바꾼 뒤 새 프레임을 준비한다.

```rust,no_run
use renderer::{RadialChart, RadialError, RadialFrame, RadialRenderer};
use std::sync::Arc;

async fn update_hover(
    renderer: &mut RadialRenderer,
    chart: &mut RadialChart,
    displayed: &Arc<RadialFrame>,
    pointer: Option<[f32; 2]>, // 차트 내부 논리 좌표. 영역 밖이면 None
    elapsed_seconds: f32,
    logical_size: (u32, u32),
    scale: f32,
) -> Result<Arc<RadialFrame>, RadialError> {
    let target = match pointer {
        Some(p) => match displayed.pick(p)?.resolve().await? {
            Some(hit) => Some(hit),
            None => displayed
                .pick_at_rest(p)?
                .resolve()
                .await?
                .filter(|t| Some(*t) == chart.interaction.hovered),
        },
        None => None,
    };
    chart.interaction.animate_hover(target, elapsed_seconds, 0.16);
    renderer.prepare(chart, logical_size, scale)
}
```

호스트가 애니메이션 중 프레임을 요청해야 한다. 포인터가 멈춰도 진입·이탈이 끝날 때까지
진행하며, 렌더러 자체는 타이머를 돌리지 않는다. 클릭 선택은
`chart.interaction.selected = displayed.pick(position)?.resolve().await?`로 설정하고 `None`으로
해제한다.
새 프레임을 그린 command buffer를 제출하고 이전 프레임을 놓은 뒤 `end_frame()`을 호출한다.
크기·DPR·내용 변경도 같은 `prepare` 경로를 사용하며, 단순 UI 편집 때 `clear_cache()`를
호출하면 재사용 가능한 라벨까지 버리므로 필요하지 않다.

#### 저장과 호환성

`figgy-renderer`의 `serde` 기능(모델만 쓸 때는 `figgy-model`의 `serde` 기능)을 켜면 모델을
직렬화할 수 있다. 조각의 `style`·`labels`·`label_format`·`label_color`와 차트의
`decorations`·`label_format`·`label_decimals`·`value_suffix`·`interaction`은 이전 스냅샷에 없으면
기본값으로 읽는다. 차트의 `chart_title`·`active_layer`·`annotations`와 조각의 `text`도 없으면
기본값으로 읽고, 기본값일 때는 저장하지 않는다. `chart_title`은 공통 차트 제목
`ChartTitleOptions`이지만 이전과 같은 `{ "visible", "text" }` 객체로 저장한다. 위치는
`decorations`에 있으므로 `offset_x`·`offset_y`·`top_margin`은 0이어야 한다. `RadialStyle`의 빠진 필드도 기본값을 사용한다.
0.19 이전 문서에는 일반 문자열 `title`도 있다. `chart_title`이 없으면 이 문자열을 문서의 `font_family`, `font_size`의 1.5배, `label_color`로 된 `chart_title`로 읽고, `chart_title`이 있으면 무시한다. 저장할 때는 `chart_title`만 쓴다.
저장된 모델과 복원한 모델은 모두 `validate`로 확인한다. 일시적인 호버·선택을 저장하지
않으려면 저장용 복사본의 `interaction`을 초기화한다.

조각의 `text_generated`(#313)도 없으면 `false`로 읽고, `false`일 때는 저장하지 않는다.

JSON의 누락 필드 호환과 Rust 소스 호환은 다르다. 이전 `RadialSlice` 구조체 리터럴은 새 필드가
필요하므로 `RadialSlice::new`를 사용하는 편이 좋다. 차트·스타일은 `..Default::default()`나
기존 값의 복사본을 사용한다. `Material`은 `#[non_exhaustive]`가 아니므로 재질이 늘어나면
빠짐없이 분기하는 `match`는 컴파일되지 않는다. 버전을 올릴 때 새 재질을 분기에 추가한다.

### 네이티브 편집 예제

저장소 루트에서 실행한다. 예제는 [egui 예제 크레이트](https://github.com/shim9610/figgy/blob/master/examples/egui)에 있다.

```bash
cargo run --locked --manifest-path examples/egui/Cargo.toml --example radial_editor
```

조각을 클릭하거나 왼쪽 목록에서 선택한 뒤 색, 개별 스타일, 표기 내용을 바꿀 수 있다.
`Whole chart`는 공통 설정을, `Individual style`은 선택한 조각의 설정을 편집한다.
편집기는 `RadialStyle::three_dimensional()`로 시작하므로 기울기가 55°이고, 갤러리의 입체 장면도
55°를 쓴다. 0°는 바로 위에서 내려다본 시점이고, 값을 높일수록 옆면이 더 많이 보인다. 정지 화면과
호버는 같은 시점을 사용한다.
`Inherit chart labels`는 개별 라벨 설정을 지운다. 편집기는 창 크기와 DPR에 맞춘 텍스처에 프레임을
직접 그린다.

### 자원 수명과 검증

GPU 메모리 집계와 예산은 렌더러를 만든 장치 묶음(`RendererDevice`)에 속하며, 같은 묶음에서 만든 모든 렌더러가 함께 쓴다. 기본값은 예산 없음이고 `set_memory_budget(Some(bytes))`로 정한다. 프레임마다 112바이트 uniform,
보이는 조각당 176바이트 표를 사용한다. 조각 선택(`pick`)마다 16바이트 질의·결과 버퍼와 16바이트
readback 버퍼를 만들고, 결과를 읽은 뒤 놓는다. 라벨 배치가 바뀌면 실제 출력 크기의 RGBA 라벨
텍스처를 만들고, 배치가 같으면 공유한다. 내보내기는 출력
타깃과 정렬된 readback 버퍼를 추가한다. GPU 할당 전에 전체 사용량과 예산을 검사한다.
`gpu_memory_usage()`는 살아 있는 자원과 GPU 완료를 기다리는 자원을 모두 집계한다.
같은 `RendererDevice` 묶음에서 만든 `Renderer`와 집계를 함께 쓰므로, 어느 렌더러에서 읽어도 묶음 전체의 합계가 나온다.
이미 있는 직교 좌표 `Renderer`의 장치에 원형 차트를 그리려면 `renderer.renderer_device()`(#313)로
그 묶음을 받아 `RadialRenderer::try_new`에 넘긴다. 두 렌더러는 장치, 큐, 장치 손실 기록, 집계와
예산을 함께 쓴다. 한쪽 렌더러나 묶음을 놓아도 장치는 파괴되지 않으며, 장치를 가진 쪽이 남아 있는 동안
유지된다. 장치를 잃으면 두 렌더러의 다음 대기·내보내기가 모두 손실 오류를 돌려준다. 브라우저에서는
`FiggyBoundedChart.create_on(parent, ...)`가 이 묶음으로 원형 차트를 만든다.

불변 프레임마다 작은 uniform·조각 표를 새로 만들 수 있으므로, 사용이 끝난 프레임을 놓고 관련
command buffer를 모두 제출하거나 버린 뒤 `end_frame()`을 호출한다. 호버할 때 라벨 해상도 크기의
자원이 계속 늘어나지는 않는다. 네이티브 호스트는 장치 poll로 완료 콜백을 처리하고, 웹에서는
브라우저가 콜백을 처리한다. 불필요한 캐시는 `clear_cache()`로 놓는다. 외부에 보관한 `Arc<RadialFrame>`이 있으면 해당 자원은 계속 유지된다.

```bash
cargo test --locked -p figgy-model --features serde
cargo test --locked -p figgy-renderer --test radial_render -- --test-threads=1
cargo test --locked -p figgy-renderer --doc
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
```

네이티브 GPU를 요청하지 못하면 픽셀 테스트는 통과로 처리하지 않고 실패한다.

### 제목·라벨 선택과 이동

조각 선택(`pick`)과 별도로 `frame.decorations()`가 제목·그리기 영역·조각 라벨의
`HitMap<RadialChart>`를 제공한다. 히트 영역·선택 테두리·드래그 제한·화면의 강조는 모두 화면에 그린
배치와 일치한다. 직교 좌표 차트와 같은 `Selectable`·`Draggable` 계약으로 `chart.decorations`를 갱신하고 `prepare`한다.
`frame.decoration_at(&chart, [x, y])`, `drag_decoration(&mut chart, hit, dx, dy)` 같은 프레임
메서드가 이 호출을 감싼다. 제목은 차트 위쪽의 제목 띠 가운데에 놓이며, 캔버스보다 넓으면
캔버스에서 잘린다. 이때 히트 영역·선택 테두리·드래그 제한도 캔버스와 겹친 부분이다.
옮긴 장식을 원래 자리로 돌리려면 `chart.decorations.reset(&id)`를 호출한다. 저장된 오프셋을 지우며
(`DataArea`는 지정한 그리기 영역도 지운다) 반대 방향으로 끄는 것과 달리 아무 값도 남기지 않는다.
브라우저에서는 `reset_decoration_position(id)`(#313)가 이 호출을 감싼다.

`frame.drag_decoration_snapped(&mut chart, hit, dx, dy, &mut snap)`는 맞춤 가이드에 붙는
`drag_decoration`이다(#465). 제목이나 조각 라벨은 그린 상자를 캔버스와 그리기 영역의 중심, 다른
장식의 가장자리와 중심에 `layout::SNAP_THRESHOLD_PX`(화면 6 px) 안에서 붙인다. 그리기 영역은 붙지
않는다. `snap`은 호스트가 누를 때부터 놓을 때까지 유지하는 `layout::DragSnap`이다
(`DragSnap::new(display_scale)`, 보조 키를 누르는 동안 `set_enabled(false)`, 끝에 `release()`).
`snap.guides()`를 `renderer.prepare_with_guides(chart, size, scale, guides)`(주석 층은
`prepare_annotations_with_guides`)에 넘기면 선택 상자와 함께 가이드를 그린다. `prepare`와
`prepare_annotations`는 가이드를 그리지 않으며, 내보내기도 가이드를 그리지 않는다. 브라우저에서는
`FiggyBoundedChart`가 `begin_drag()`와 `end_drag()` 사이의 `drag`, `drag_annotation`을 붙이고,
`set_snap_modifier(held)`가 호스트가 보조 키를 누르는 동안 붙임을 끈다.
좌표 단위, 요소별 이동 규칙, 캐시·저장 복원 및 웹 연결은 [공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)를 참고한다.


#### 주석 층

`chart.annotations`의 텍스트 상자와 화살표는 프레임 위에 별도 패스로 그린다. 원형 차트에는 값 축이
없으므로 `Anchor::Ratio`(플롯 영역 비율)만 배치할 수 있고, `Anchor::Data`와 기준선·구간은
건너뛴다. 건너뛴 항목은 `BoundedAnnotationFrame::render_warnings()`에
`RenderWarning::AnnotationSkipped { index, reason: AnnotationSkip::NoValueAxis }`로 남는다.
`renderer.prepare_annotations(chart, size, scale)`가 돌려주는 `BoundedAnnotationFrame`은
`frame.draw` 다음에 `draw_overlay`로 그리고, 히트 맵은 `annotations()`로 얻는다. 구간은
건너뛰므로 조각 아래에 그리는 층은 없다. 주석 편집과 `decorations.selected = Annotation(i)` 선택은 프레임과 라벨 래스터를 다시 만들지
않는다. 브라우저에서는 `FiggyBoundedChart.annotation_preview(json)`가 아직 넣지 않은 항목이 그려질 자리나
건너뛸 이유를 문서와 선택을 바꾸지 않고 알려 준다(#334). 자세한 계약은 [공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer)를 참고한다.
