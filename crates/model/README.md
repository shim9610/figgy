# figgy-model

Chart settings and data definitions for figgy: Cartesian axes and series,
interaction policies, and the bounded radial, categorical and boxplot models.
This crate has no GPU or windowing dependency. The optional `serde` feature adds
serialization and deserialization.

The Cargo package is **`figgy-model`**; its Rust library name remains **`model`**.
The package has not yet been published to crates.io. Existing Git consumers must
add `package = "figgy-model"` when selecting a revision with the new package name.

Applications using the renderer normally only need `figgy-renderer`, which
re-exports these model types. Use this package directly when working with chart
definitions without a renderer. Rust 1.99 is the declared support floor; lower
versions have not been validated.

Serialization represents model state, not a versioned application file format.
Validate externally supplied state before rendering. File storage, migrations and
the meaning of supplied statistical summaries belong to the consuming application.

Code is licensed under MIT OR Apache-2.0.

Run `cargo test --features serde` to check the model contracts and optional
serialization. These tests require neither a GPU nor other source checkouts.
