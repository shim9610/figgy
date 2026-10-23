//! Compile-time assembly of WGSL modules from fragment files.
//!
//! WGSL has no `import`. A block of WGSL that several shaders need lives in
//! one fragment file under `src/data_render/common/` (or, for the layered
//! vector and pick shaders, next to them), and every module that needs it is
//! declared with [`wgsl_module!`]: the fragments' text spliced in order, one
//! newline between two fragments, as a `&'static str` built by `concat!` at
//! compile time. There is no runtime templating and no allocation; the
//! fragment list is the module's identity.
//!
//! [`MODULES`] names every assembled module, so a test can parse and
//! validate each one without a GPU and the integration tests can read the
//! source a pipeline compiles.

/// One WGSL module assembled from fragment files at compile time.
///
/// Each argument is a path relative to the renderer's `src/` directory. The
/// result is the files' contents joined with one newline, a `&'static str`.
macro_rules! wgsl_module {
    ($first:literal $(, $rest:literal)* $(,)?) => {
        concat!(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/", $first))
            $(, "\n", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/", $rest)))*
        )
    };
}
pub(crate) use wgsl_module;

/// Every assembled module, by name: the source each pipeline compiles.
pub const MODULES: &[(&str, &str)] = &[
    ("scatter", crate::data_render::SCATTER_SHADER),
    ("line", crate::data_render::LINE_SHADER),
    ("errorbar", crate::data_render::ERRORBAR_SHADER),
    ("bar", crate::data_render::BAR_SHADER),
    ("field", crate::data_render::FIELD_SHADER),
    ("line_arc", crate::data_render::line_arc::LINE_ARC_SHADER),
    ("contour_anchor", crate::gpu_contour::CONTOUR_ANCHOR_SHADER),
    ("contour_label", crate::gpu_contour::CONTOUR_LABEL_SHADER),
    ("errorbar_fit", crate::gpu_errorbar::ERRORBAR_FIT_SHADER),
    ("boxplot", crate::boxplot::BOXPLOT_SHADER),
    ("categorical", crate::categorical::CATEGORICAL_SHADER),
    ("radar", crate::radar::RADAR_SHADER),
    ("radial", crate::radial::RADIAL_SHADER),
    (
        "vector_points",
        crate::data_render::vector::VECTOR_POINTS_SHADER,
    ),
    (
        "vector_grid",
        crate::data_render::vector::VECTOR_GRID_SHADER,
    ),
    ("pick_gate", crate::gpu_pick::PickShader::Gate.source()),
    ("pick_marker", crate::gpu_pick::PickShader::Marker.source()),
    (
        "pick_marker_line",
        crate::gpu_pick::PickShader::MarkerLine.source(),
    ),
    ("pick_line", crate::gpu_pick::PickShader::Line.source()),
    ("pick_vector", crate::gpu_pick::PickShader::Vector.source()),
];

/// The assembled module called `name` in [`MODULES`].
///
/// # Panics
///
/// When no module has that name.
pub fn module(name: &str) -> &'static str {
    match MODULES.iter().find(|(n, _)| *n == name) {
        Some((_, source)) => source,
        None => panic!("no assembled WGSL module named `{name}`"),
    }
}
