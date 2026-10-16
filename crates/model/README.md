# figgy-model

Chart settings and data definitions for figgy: Cartesian axes and series,
interaction policies, and the bounded radial, categorical and boxplot models.
This crate has no GPU or windowing dependency. The optional `serde` feature adds
serialization and deserialization.

Applications using the renderer normally only need `figgy-renderer`, which
re-exports these model types. Use this package directly when working with chart
definitions without a renderer. Rust 1.99 is the declared support floor; lower
versions have not been validated.

## Install

The Cargo package is **`figgy-model`**; its Rust import is **`model`**.
This source targets **0.11.0**. Use the dependency below once this version is on
[crates.io](https://crates.io/crates/figgy-model), or the local path below for a
checkout before publication:

```toml
[dependencies]
model = { package = "figgy-model", version = "0.11.0" }
```

For a local checkout, replace `version` with `path = "../figgy/crates/model"`.
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
| Selection, dragging and area handles | `Selectable`, `Draggable`, `Resizable`, `DecorationLayout` |

Bounded charts have explicit validation and small-data limits. They do not require
stream sources, and boxplots do not calculate statistics from raw samples. The
renderer owns GPU resources, frame caches and its `ColumnSource` upload adapter;
those are not part of this model crate. See the
[renderer interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)
for applying model edits to displayed frames.

## Serialization and documentation

Serialization represents model state, not a versioned application file format.
Validate externally supplied state before rendering. File storage, migrations and
the meaning of supplied statistical summaries belong to the consuming application.

docs.rs includes the optional `serde` implementations. Build the same feature set
locally with `cargo doc --locked -p figgy-model --features serde --no-deps --open`.
Documentation and model tests need no GPU or windowing libraries.

Code is licensed under MIT OR Apache-2.0.

Run `cargo test --features serde` to check the model contracts and optional
serialization. These tests require neither a GPU nor other source checkouts.
