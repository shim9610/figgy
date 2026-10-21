# figgy-model

Chart settings and data definitions for figgy: Cartesian axes and series
(including polar coordinates, vector fields, point colour and size maps, area
fills and step lines), overlay axes, subplots, annotations, interaction
policies, and the bounded radial, categorical, boxplot and radar models.
This crate has no GPU or windowing dependency. The optional `serde` feature adds
serialization and deserialization.

Applications using the renderer normally only need `figgy-renderer`, which
re-exports these model types. Use this package directly when working with chart
definitions without a renderer. Rust 1.99 is the declared support floor; lower
versions have not been validated.

## Install

The Cargo package is **`figgy-model`**; its Rust import is **`model`**.

```toml
[dependencies]
model = { package = "figgy-model", version = "0.12.1" }
```

The `version` line installs figgy-model 0.12.1 from
[crates.io](https://crates.io/crates/figgy-model). For a local checkout, replace
`version` with `path = "../figgy/crates/model"`. Code written for 0.11.0, the
release that shipped with figgy-renderer 0.17.0, needs the changes in the
[migration notes](https://github.com/shim9610/figgy/blob/master/CHANGELOG.md#migrating-from-0170).

Add `features = ["serde"]` only when serialization is needed. No feature is
required for creating, editing or validating models.

## Create and edit a chart model

This example runs without a GPU. It creates a donut model, changes one value and
validates it. A renderer consumes this same model to prepare pixels and hit maps.

```rust
use model::{Color, RadialChart, RadialKind, RadialSlice};

let mut chart = RadialChart {
    kind: RadialKind::Donut { inner_radius: 0.55 },
    slices: vec![
        RadialSlice::new("Control", 30.0, Color::from_rgb8(40, 140, 200)),
        RadialSlice::new("Treatment", 50.0, Color::from_rgb8(220, 160, 60)),
    ],
    ..Default::default()
};
chart.slices[1].value = 70.0;
assert!(chart.validate().is_ok());
assert_eq!(chart.slices.iter().map(|slice| slice.value).sum::<f64>(), 100.0);
```

## Find the relevant types

| Data | Types |
| --- | --- |
| Cartesian axes, ticks, legends and styles | `Config`, `default::default_config`, `SeriesConfig` |
| Numeric input columns | `Column`, `DataCell` |
| Pie/donut slice values and presentation | `RadialChart`, `RadialSlice` |
| Category/series bars | `CategoricalChart`, `BarSeries` |
| Supplied statistical summaries | `BoxPlotChart`, `BoxPlotSeries` |
| One value per category and series on shared spokes | `RadarChart`, `RadarSeries` |
| A Cartesian chart drawn in (angle, radius) | `polar::PolarOptions` on `Config.polar`, `polar::PolarGeometry` |
| Arrows, point colour and size maps, area fills and step lines | `data_config::DataRenderType::Vector`, `PointColorMap`, `PointSizeMap`, `LineFill`, `LineInterpolation` |
| Selection, dragging and area handles | `Selectable`, `Draggable`, `Resizable`, `DecorationLayout` |
| Secondary Y axes, subplots and the layer that receives pointer input | `OverlayLayer` on `Config.overlays`, `Subplot` on `Config.subplots`, `LayerRef` on `active_layer` |
| Reference lines, spans, text boxes and arrows | `AnnotationLayer` on `annotations`, `Annotation`, `AnnotationKind`, `Anchor` |
| Rich titles on pie, bar, boxplot and radar charts | `BoundedTitleOptions` on `chart_title` |

Bounded charts (pie/donut, bar, boxplot and radar) carry their values in the
model, not in columns, and take no stream sources. Their `validate` methods
enforce fixed size limits, such as 64 slices or categories and 16 series. The
renderer owns GPU resources, frame caches and its `ColumnSource` upload adapter;
those are not part of this model crate. See the
[renderer interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)
for applying model edits to displayed frames.

## Serialization and documentation

Serialization represents model state, not a versioned application file format.
Validate externally supplied state before rendering. File storage, migrations and
the meaning of supplied statistical summaries belong to the consuming application.

docs.rs documents the published release, including the optional `serde`
implementations. Build the same feature set for this source with
`cargo doc --locked -p figgy-model --features serde --no-deps --open`.

Code is licensed under MIT OR Apache-2.0.

Run `cargo test --features serde` to check the model contracts and optional
serialization. These tests require neither a GPU nor other source checkouts.
