//! Compile-time assembly of WGSL modules from fragment files.
//!
//! WGSL has no `import`. A block of WGSL that several shaders need lives in
//! one fragment file under `src/data_render/common/` (or, for the layered
//! vector and pick shaders, next to them), and every module that needs it is
//! declared with `assembled!`: the fragments' text spliced in order, one
//! newline between two fragments, as a `&'static str` built by `concat!` at
//! compile time, next to the list of fragment names it was built from. There
//! is no runtime templating and no allocation; the fragment list is the
//! module's identity.
//!
//! `MODULES` names every assembled module with its source and fragments, so
//! a test can parse and validate each one without a GPU, compare the
//! fragments two modules splice, and the integration tests can read the
//! source a pipeline compiles.
//!
//! A module that places data is assembled twice, by `assembled_by_cuts!`
//! (#299): with the cut remap of a broken axis and with its identity twin
//! (`AxisCutShaders`). `MODULES_BY_CUTS` names those.

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

/// One assembled module: the WGSL source a pipeline compiles and the fragment
/// files it is spliced from, in order, relative to the renderer's `src/`.
#[derive(Clone, Copy, Debug)]
pub struct Assembled {
    /// The fragments' contents joined with one newline.
    pub source: &'static str,
    /// The fragment files, in assembly order.
    pub fragments: &'static [&'static str],
}

/// An [`Assembled`] module from one literal list of fragment files: the
/// source is `wgsl_module!` over the list, so the source and the names it is
/// built from cannot disagree.
macro_rules! assembled {
    ($($fragment:literal),+ $(,)?) => {
        $crate::shader_assembly::Assembled {
            source: $crate::shader_assembly::wgsl_module!($($fragment),+),
            fragments: &[$($fragment),+],
        }
    };
}
pub(crate) use assembled;

/// Which axis-cut fragment a module is assembled with (#299).
///
/// The cut remap of a broken axis (#200) is the most expensive code a data
/// pipeline compiles, and a chart without a break never needs it. Every
/// module that places data is therefore assembled twice, from one fragment
/// list (`assembled_by_cuts!`): once with `axis_cuts.wgsl` and once with
/// its twin `axis_cuts_identity.wgsl`, which has the same signatures and no
/// cut. A renderer compiles the identity modules until it is first handed a
/// chart with an axis break and the remap modules from then on
/// (`Renderer::admit_axis_cuts`); a chart with a break is never drawn or
/// picked through an identity module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AxisCutShaders {
    /// `axis_cuts_identity.wgsl`: no chart with an axis break has been seen.
    #[default]
    Identity,
    /// `axis_cuts.wgsl`: the cut remap.
    Remap,
}

/// The fragment [`AxisCutShaders::Remap`] splices.
pub const AXIS_CUTS_FRAGMENT: &str = "data_render/common/axis_cuts.wgsl";
/// The fragment [`AxisCutShaders::Identity`] splices.
pub const AXIS_CUTS_IDENTITY_FRAGMENT: &str = "data_render/common/axis_cuts_identity.wgsl";

/// One module assembled twice from the same fragment list, apart from the
/// axis-cut fragment ([`AxisCutShaders`]).
#[derive(Clone, Copy, Debug)]
pub struct AssembledByCuts {
    /// With `axis_cuts_identity.wgsl`.
    pub identity: Assembled,
    /// With `axis_cuts.wgsl`.
    pub remap: Assembled,
}

impl AssembledByCuts {
    /// The assembly for `cuts`.
    pub const fn of(&self, cuts: AxisCutShaders) -> Assembled {
        match cuts {
            AxisCutShaders::Identity => self.identity,
            AxisCutShaders::Remap => self.remap,
        }
    }
}

/// An [`AssembledByCuts`] from one literal list of fragment files, with the
/// word `cuts` where the axis-cut fragment goes: [`assembled!`] over the list
/// with `axis_cuts_identity.wgsl` there and again with `axis_cuts.wgsl`, so
/// the two modules cannot differ in anything else.
macro_rules! assembled_by_cuts {
    ($($before:literal,)* cuts $(, $after:literal)* $(,)?) => {
        $crate::shader_assembly::AssembledByCuts {
            identity: $crate::shader_assembly::assembled!(
                $($before,)*
                "data_render/common/axis_cuts_identity.wgsl"
                $(, $after)*
            ),
            remap: $crate::shader_assembly::assembled!(
                $($before,)*
                "data_render/common/axis_cuts.wgsl"
                $(, $after)*
            ),
        }
    };
}
pub(crate) use assembled_by_cuts;

/// Every assembled module, by name: the source each pipeline compiles and
/// its fragments.
pub const MODULES: &[(&str, Assembled)] = &[
    ("scatter", crate::data_render::SCATTER_SHADER),
    ("line", crate::data_render::LINE_SHADER),
    ("errorbar", crate::data_render::ERRORBAR_SHADER),
    ("bar", crate::data_render::BAR_SHADER),
    ("field", crate::data_render::FIELD_SHADER),
    ("tri_mesh", crate::data_render::tri_mesh::TRI_MESH_SHADER),
    ("surface", crate::data_render::surface_mesh::SURFACE_SHADER),
    (
        "tri_surface",
        crate::data_render::surface_mesh::TRI_SURFACE_SHADER,
    ),
    ("line_arc", crate::data_render::line_arc::LINE_ARC_SHADER),
    ("contour_anchor", crate::gpu_contour::CONTOUR_ANCHOR_SHADER),
    ("contour_label", crate::gpu_contour::CONTOUR_LABEL_SHADER),
    ("errorbar_fit", crate::gpu_extent::ERRORBAR_FIT_SHADER),
    ("radial", crate::radial::RADIAL_SHADER),
    ("radial_pick", crate::radial::RADIAL_PICK_SHADER),
    (
        "vector_points",
        crate::data_render::vector::VECTOR_POINTS_SHADER,
    ),
    (
        "vector_grid",
        crate::data_render::vector::VECTOR_GRID_SHADER,
    ),
    (
        "vector_path",
        crate::data_render::vector::VECTOR_PATH_SHADER,
    ),
    ("pick_gate", crate::gpu_pick::PickShader::Gate.assembled()),
    (
        "pick_marker",
        crate::gpu_pick::PickShader::Marker.assembled(),
    ),
    (
        "pick_marker_line",
        crate::gpu_pick::PickShader::MarkerLine.assembled(),
    ),
    ("pick_line", crate::gpu_pick::PickShader::Line.assembled()),
    (
        "pick_vector",
        crate::gpu_pick::PickShader::Vector.assembled(),
    ),
];

/// Every module assembled by cuts ([`AssembledByCuts`], #299), by name: the
/// modules that splice `projection.wgsl`. [`MODULES`] lists the remap
/// assembly of each under the same name where it has one.
pub const MODULES_BY_CUTS: &[(&str, AssembledByCuts)] = &[
    ("scatter", crate::data_render::SCATTER_SHADER_BY_CUTS),
    ("line", crate::data_render::LINE_SHADER_BY_CUTS),
    ("errorbar", crate::data_render::ERRORBAR_SHADER_BY_CUTS),
    ("bar", crate::data_render::BAR_SHADER_BY_CUTS),
    ("field", crate::data_render::FIELD_SHADER_BY_CUTS),
    (
        "tri_mesh",
        crate::data_render::tri_mesh::TRI_MESH_SHADER_BY_CUTS,
    ),
    (
        "surface",
        crate::data_render::surface_mesh::SURFACE_SHADER_BY_CUTS,
    ),
    (
        "tri_surface",
        crate::data_render::surface_mesh::TRI_SURFACE_SHADER_BY_CUTS,
    ),
    (
        "ternary_region",
        crate::data_render::ternary_region::TERNARY_REGION_SHADER_BY_CUTS,
    ),
    (
        "line_arc",
        crate::data_render::line_arc::LINE_ARC_SHADER_BY_CUTS,
    ),
    (
        "contour_anchor",
        crate::gpu_contour::CONTOUR_ANCHOR_SHADER_BY_CUTS,
    ),
    (
        "contour_label",
        crate::gpu_contour::CONTOUR_LABEL_SHADER_BY_CUTS,
    ),
    (
        "vector_points",
        crate::data_render::vector::VECTOR_POINTS_SHADER_BY_CUTS,
    ),
    (
        "vector_grid",
        crate::data_render::vector::VECTOR_GRID_SHADER_BY_CUTS,
    ),
    (
        "vector_path",
        crate::data_render::vector::VECTOR_PATH_SHADER_BY_CUTS,
    ),
    (
        "vector_points_3d",
        crate::data_render::vector::VECTOR_POINTS_3D_SHADER_BY_CUTS,
    ),
    ("pick_gate", crate::gpu_pick::PickShader::Gate.by_cuts()),
    ("pick_marker", crate::gpu_pick::PickShader::Marker.by_cuts()),
    (
        "pick_marker_line",
        crate::gpu_pick::PickShader::MarkerLine.by_cuts(),
    ),
    ("pick_line", crate::gpu_pick::PickShader::Line.by_cuts()),
    ("pick_vector", crate::gpu_pick::PickShader::Vector.by_cuts()),
];

/// The source of the module called `name` in [`MODULES_BY_CUTS`] assembled
/// for `cuts`, or `None` when no module there has that name.
pub fn module_by_cuts(name: &str, cuts: AxisCutShaders) -> Option<&'static str> {
    MODULES_BY_CUTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, by_cuts)| by_cuts.of(cuts).source)
}

/// The identity assembly's source for a remap assembly's `source`, and
/// `source` itself for any other: the two compile to pipelines with the same
/// descriptors, so the browser prewarm contract
/// (`shader_pipeline_contract.json`) stores each pipeline once, under the
/// identity source.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn contract_source(source: &str) -> &str {
    MODULES_BY_CUTS
        .iter()
        .find(|(_, by_cuts)| by_cuts.remap.source == source)
        .map_or(source, |(_, by_cuts)| by_cuts.identity.source)
}

/// The source of the assembled module called `name` in [`MODULES`], or
/// `None` when no module has that name.
pub fn module(name: &str) -> Option<&'static str> {
    MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, assembled)| assembled.source)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    /// Every assembly a pipeline can compile: [`super::MODULES`] and both
    /// assemblies of [`super::MODULES_BY_CUTS`] (#299), each under a name
    /// that says which it is.
    fn every_assembly() -> Vec<(&'static str, super::Assembled)> {
        let mut all: Vec<(&'static str, super::Assembled)> = super::MODULES.to_vec();
        for (name, by_cuts) in super::MODULES_BY_CUTS {
            all.push((
                Box::leak(format!("{name} (identity cuts)").into_boxed_str()),
                by_cuts.identity,
            ));
            all.push((
                Box::leak(format!("{name} (cut remap)").into_boxed_str()),
                by_cuts.remap,
            ));
        }
        all
    }

    /// The two assemblies of a module differ in the axis-cut fragment alone
    /// (#299), every module that splices `projection.wgsl` (or calls a cut
    /// function) is assembled by cuts, and [`super::MODULES`] names the
    /// remap assembly.
    #[test]
    fn modules_by_cuts_differ_in_the_axis_cut_fragment_alone() {
        use super::{AXIS_CUTS_FRAGMENT, AXIS_CUTS_IDENTITY_FRAGMENT, AxisCutShaders};
        for (name, by_cuts) in super::MODULES_BY_CUTS {
            let (identity, remap) = (by_cuts.identity.fragments, by_cuts.remap.fragments);
            assert_eq!(identity.len(), remap.len(), "{name}");
            let mut cut_fragments = 0;
            for (a, b) in identity.iter().zip(remap) {
                if a == b {
                    assert!(
                        *a != AXIS_CUTS_FRAGMENT && *a != AXIS_CUTS_IDENTITY_FRAGMENT,
                        "{name}"
                    );
                    continue;
                }
                assert_eq!(
                    (*a, *b),
                    (AXIS_CUTS_IDENTITY_FRAGMENT, AXIS_CUTS_FRAGMENT),
                    "{name}"
                );
                cut_fragments += 1;
            }
            assert_eq!(cut_fragments, 1, "{name}");
            assert!(
                identity.contains(&"data_render/common/projection.wgsl"),
                "{name}"
            );
            assert_eq!(
                by_cuts.of(AxisCutShaders::Identity).source,
                by_cuts.identity.source
            );
            assert_eq!(
                by_cuts.of(AxisCutShaders::Remap).source,
                by_cuts.remap.source
            );
            assert_ne!(by_cuts.identity.source, by_cuts.remap.source, "{name}");
            assert_eq!(
                super::contract_source(by_cuts.remap.source),
                by_cuts.identity.source,
                "{name}"
            );
            assert_eq!(
                super::contract_source(by_cuts.identity.source),
                by_cuts.identity.source,
                "{name}"
            );
            assert_eq!(
                super::module_by_cuts(name, AxisCutShaders::Identity),
                Some(by_cuts.identity.source)
            );
            // The plain name is the remap assembly: the full contract.
            if let Some(plain) = super::module(name) {
                assert_eq!(plain, by_cuts.remap.source, "{name}");
            }
        }
        // No module outside the table splices either cut fragment or the
        // projection that calls them.
        for (name, assembled) in super::MODULES {
            let by_cuts = super::MODULES_BY_CUTS.iter().any(|(n, _)| n == name);
            for fragment in [
                AXIS_CUTS_FRAGMENT,
                AXIS_CUTS_IDENTITY_FRAGMENT,
                "data_render/common/projection.wgsl",
            ] {
                assert_eq!(
                    assembled.fragments.contains(&fragment),
                    by_cuts && fragment != AXIS_CUTS_IDENTITY_FRAGMENT,
                    "{name}: {fragment}"
                );
            }
        }
    }

    /// The identity fragment declares what the cut fragment's callers use,
    /// with the same signatures (#299): `AxisCuts`, `no_cuts`, `cuts_of`,
    /// `cut_remap`, `cut_unmap` and `cut_scale_at`. It maps every `t` to
    /// itself at scale 1, which a module built from it and evaluated by naga
    /// shows for a table that holds cuts.
    #[test]
    fn the_identity_cut_fragment_has_the_cut_fragments_signatures() {
        use wgpu::naga::{Module, TypeInner};
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let read = |fragment: &str| std::fs::read_to_string(src.join(fragment)).unwrap();
        let identity = parse("identity", &read(super::AXIS_CUTS_IDENTITY_FRAGMENT));
        let remap = parse("remap", &read(super::AXIS_CUTS_FRAGMENT));
        // A type as text: a struct by its name, anything else by its shape.
        fn type_text(module: &Module, handle: wgpu::naga::Handle<wgpu::naga::Type>) -> String {
            let ty = &module.types[handle];
            match &ty.inner {
                TypeInner::Struct { .. } => ty.name.clone().unwrap(),
                TypeInner::Array { base, size, .. } => {
                    format!("array<{}, {size:?}>", type_text(module, *base))
                }
                other => format!("{other:?}"),
            }
        }
        let signature = |module: &Module, name: &str| -> Option<(Vec<String>, Option<String>)> {
            let (_, function) = module
                .functions
                .iter()
                .find(|(_, f)| f.name.as_deref() == Some(name))?;
            Some((
                function
                    .arguments
                    .iter()
                    .map(|argument| type_text(module, argument.ty))
                    .collect(),
                function
                    .result
                    .as_ref()
                    .map(|result| type_text(module, result.ty)),
            ))
        };
        let shared = [
            "no_cuts",
            "cuts_of",
            "cut_remap",
            "cut_unmap",
            "cut_scale_at",
        ];
        for name in shared {
            let found = signature(&identity, name);
            assert!(found.is_some(), "identity lacks {name}");
            assert_eq!(found, signature(&remap, name), "{name}");
        }
        // The identity fragment declares nothing else, so no caller can come
        // to depend on a function only one variant has.
        let mut names: Vec<&str> = identity
            .functions
            .iter()
            .filter_map(|(_, f)| f.name.as_deref())
            .collect();
        names.sort_unstable();
        let mut expected = shared.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
        for module in [&identity, &remap] {
            assert!(
                module
                    .types
                    .iter()
                    .any(|(_, ty)| ty.name.as_deref() == Some("AxisCuts")),
            );
        }
        // No other fragment reads a member of `AxisCuts` or calls a helper
        // only the cut fragment has.
        fn wgsl_files(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    wgsl_files(&path, files);
                } else if path.extension().is_some_and(|e| e == "wgsl") {
                    files.push(path);
                }
            }
        }
        let mut files = Vec::new();
        wgsl_files(&src, &mut files);
        for file in files {
            if file.ends_with(super::AXIS_CUTS_FRAGMENT)
                || file.ends_with(super::AXIS_CUTS_IDENTITY_FRAGMENT)
            {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for private in [
                "piece_slope",
                "count_scale",
                ".k0",
                ".k1",
                ".k2",
                "AxisCuts(",
            ] {
                assert!(
                    !text.contains(private),
                    "{} uses `{private}` of axis_cuts.wgsl",
                    file.display()
                );
            }
        }
    }

    /// Every assembled module parses and validates as WGSL, with no GPU: a
    /// fragment that breaks a module fails here, not on the first draw.
    #[test]
    fn every_assembled_module_parses_and_validates() {
        for (name, assembled) in &every_assembly() {
            let source = assembled.source;
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
        }
    }

    /// The constants of `source` that a browser's WGSL compiler refuses
    /// although naga accepts them: a constant expression that evaluates to
    /// NaN or an infinity (`bitcast<f32>(0x7fc00000u)`) and a float literal
    /// past `f32::MAX` (`3.4028235e38`). WGSL makes both a shader-creation
    /// error, so Chrome fails every pipeline of the module.
    fn browser_rejected_constants(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        for (number, line) in source.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let mut rest = code;
            while let Some(at) = rest.find("bitcast<f32>(") {
                rest = &rest[at + "bitcast<f32>(".len()..];
                let argument = rest.split(')').next().unwrap_or("").trim();
                let bits = argument
                    .strip_prefix("0x")
                    .and_then(|hex| hex.strip_suffix('u'))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok());
                if bits.is_some_and(|bits| bits & 0x7f80_0000 == 0x7f80_0000) {
                    found.push(format!("line {}: {}", number + 1, line.trim()));
                }
            }
            let bytes = code.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let starts = bytes[i].is_ascii_digit()
                    && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'));
                if !starts {
                    i += 1;
                    continue;
                }
                let begin = i;
                if code[i..].starts_with("0x") || code[i..].starts_with("0X") {
                    i += 2;
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.')
                    {
                        i += 1;
                    }
                    continue;
                }
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let literal = &code[begin..i];
                if literal
                    .parse::<f64>()
                    .is_ok_and(|v| v > f64::from(f32::MAX))
                {
                    found.push(format!("line {}: {}", number + 1, line.trim()));
                }
            }
        }
        found
    }

    #[test]
    fn no_assembled_module_holds_a_constant_a_browser_refuses() {
        let mut found = Vec::new();
        for (name, assembled) in &every_assembly() {
            for hit in browser_rejected_constants(assembled.source) {
                found.push(format!("{name} {hit}"));
            }
        }
        assert!(found.is_empty(), "{found:#?}");
        // The scan finds both forms.
        assert_eq!(
            browser_rejected_constants(
                "let a = bitcast<f32>(0x7fc00000u);\nlet b = abs(v) <= 3.4028235e38;\n\
                 let c = bitcast<f32>(0x3f800000u); // bitcast<f32>(0x7f800000u)\nlet d = 3.4e38;"
            )
            .len(),
            2
        );
    }

    fn parse(name: &str, source: &str) -> wgpu::naga::Module {
        wgpu::naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)))
    }

    /// Every `u32` and `f32` constant a WGSL module declares that a Rust
    /// constant also defines has the Rust value, in every module that
    /// declares it; an `f32` matches bit for bit. The pairs are (WGSL name,
    /// Rust value); a WGSL name that no module declares fails too, so a
    /// renamed constant cannot drop out of the check.
    #[test]
    fn wgsl_constants_equal_their_rust_constants() {
        use crate::data_config::LineInterpolation as Step;
        use crate::data_render::vector;
        use crate::gpu_pick::types as pick;
        use crate::line_step::{self, AREA_FILL_TO_SERIES, LINE_ARC_SHIFT, LINE_STEP_SHIFT};
        use crate::{data_render as dr, gpu_data_pick as data_pick, gpu_extent as fit};
        let rust: &[(&str, u32)] = &[
            ("FIELD_COLUMNS_ARE_Y", dr::FIELD_FLAG_COLUMNS_ARE_Y),
            ("FIELD_CENTERS", dr::FIELD_FLAG_CENTERS),
            ("FIELD_INTERPOLATED", dr::FIELD_FLAG_INTERPOLATED),
            ("FIELD_BANDS", dr::FIELD_FLAG_BANDS),
            ("FIELD_LOG_Z", dr::FIELD_FLAG_LOG_Z),
            ("FIELD_HEXAGONAL", dr::FIELD_FLAG_HEXAGONAL),
            ("FIELD_RGB", dr::FIELD_FLAG_RGB),
            ("FIELD_MESH", dr::FIELD_FLAG_MESH),
            ("FIELD_FACE_VALUES", dr::FIELD_FLAG_FACE_VALUES),
            ("MESH_VALUES", dr::tri_mesh::MESH_TABLE_VALUES as u32),
            ("MESH_INDICES", dr::tri_mesh::MESH_TABLE_INDICES as u32),
            ("VECTOR_FLAG_GRID", vector::VECTOR_FLAG_GRID),
            (
                "VECTOR_FLAG_COLUMNS_ARE_Y",
                vector::VECTOR_FLAG_COLUMNS_ARE_Y,
            ),
            ("VECTOR_FLAG_CENTERS", vector::VECTOR_FLAG_CENTERS),
            ("VECTOR_FLAG_SCALE_PIXELS", vector::VECTOR_FLAG_SCALE_PIXELS),
            ("VECTOR_FLAG_LOG_Z", vector::VECTOR_FLAG_LOG_Z),
            ("VECTOR_FLAG_COLOR_MAP", vector::VECTOR_FLAG_COLOR_MAP),
            ("ARROW_PIVOT_TAIL", vector::VECTOR_PIVOT_TAIL),
            ("ARROW_PIVOT_MIDDLE", vector::VECTOR_PIVOT_MIDDLE),
            ("ARROW_PIVOT_TIP", vector::VECTOR_PIVOT_TIP),
            ("ARROW_VERTICES", vector::ARROW_VERTICES),
            ("VALUE_MAP_COLOR", dr::VALUE_MAP_COLOR),
            ("VALUE_MAP_SIZE", dr::VALUE_MAP_SIZE),
            ("VALUE_MAP_LOG_Z", dr::VALUE_MAP_LOG_Z),
            ("VALUE_MAP_SIZE_AREA", dr::VALUE_MAP_SIZE_AREA),
            ("PICK_FLAG_SCATTER", pick::FLAG_SCATTER),
            ("PICK_FLAG_LINE", pick::FLAG_LINE),
            ("PICK_FLAG_STYLE_MAP", pick::FLAG_STYLE_MAP),
            ("PICK_FLAG_STYLE_INDEX", pick::FLAG_STYLE_INDEX),
            ("PICK_FLAG_SIZE_MAP", pick::FLAG_SIZE_MAP),
            ("PICK_FLAG_SIZE_AREA", pick::FLAG_SIZE_AREA),
            ("PICK_FLAG_VECTOR", pick::FLAG_VECTOR),
            ("PICK_FLAG_REGION", pick::FLAG_REGION),
            ("ARC_FLAG_3D", crate::data_render::line_arc::ARC_FLAG_3D),
            ("PICK_VECTOR_SCALE_PIXELS", pick::VECTOR_SCALE_PIXELS),
            ("PICK_MARKER_PRECISE", pick::PICK_MARKER_PRECISE),
            ("PICK_MARKER_SKETCH", pick::PICK_MARKER_SKETCH),
            ("PICK_MARKER_PLANET", pick::PICK_MARKER_PLANET),
            ("PICK_MARKER_STAR", pick::PICK_MARKER_STAR),
            // The ring measures the marker under the same draw-style modes.
            ("RING_MARKER_PRECISE", pick::PICK_MARKER_PRECISE),
            ("RING_MARKER_SKETCH", pick::PICK_MARKER_SKETCH),
            ("RING_MARKER_PLANET", pick::PICK_MARKER_PLANET),
            ("RING_MARKER_STAR", pick::PICK_MARKER_STAR),
            ("PICK_LINE_PRECISE", pick::PICK_LINE_PRECISE),
            ("PICK_LINE_SKETCH", pick::PICK_LINE_SKETCH),
            ("PICK_LINE_RIBBON", pick::PICK_LINE_RIBBON),
            (
                "PICK_VARIANT_STYLED_MARKER",
                crate::gpu_pick::exact_variant::STYLED_MARKER,
            ),
            (
                "PICK_VARIANT_ERRORBAR",
                crate::gpu_pick::exact_variant::ERRORBAR,
            ),
            ("PICK_VARIANT_AREA", crate::gpu_pick::exact_variant::AREA),
            ("PICK_VARIANT_DASH", crate::gpu_pick::exact_variant::DASH),
            (
                "PICK_VARIANT_STYLED_LINE",
                crate::gpu_pick::exact_variant::STYLED_LINE,
            ),
            ("PICK_VARIANT_POLAR", crate::gpu_pick::exact_variant::POLAR),
            (
                "PICK_VARIANT_PATH_ARROWS",
                crate::gpu_pick::exact_variant::PATH_ARROWS,
            ),
            (
                "PICK_VARIANT_CATEGORY_JITTER",
                crate::gpu_pick::exact_variant::CATEGORY_JITTER,
            ),
            (
                "PICK_VARIANT_TERNARY",
                crate::gpu_pick::exact_variant::TERNARY,
            ),
            ("STYLE_MASK_COLOR", dr::STYLE_MASK_COLOR),
            ("STYLE_MASK_RADIUS", dr::STYLE_MASK_RADIUS),
            ("STYLE_MASK_SHAPE", dr::STYLE_MASK_SHAPE),
            ("PICK_STYLE_MASK_RADIUS", dr::STYLE_MASK_RADIUS),
            ("PICK_STYLE_MASK_SHAPE", dr::STYLE_MASK_SHAPE),
            ("MATERIAL_FLAT", dr::MATERIAL_FLAT),
            ("MATERIAL_MATTE", dr::MATERIAL_MATTE),
            ("MATERIAL_CERAMIC", dr::MATERIAL_CERAMIC),
            ("MATERIAL_BRUSHED_METAL", dr::MATERIAL_BRUSHED_METAL),
            ("MATERIAL_PAPER", dr::MATERIAL_PAPER),
            ("MATERIAL_WOOD", dr::MATERIAL_WOOD),
            ("MATERIAL_SATIN_METAL", dr::MATERIAL_SATIN_METAL),
            ("MATERIAL_TOON", dr::MATERIAL_TOON),
            ("MATERIAL_ENAMEL", dr::MATERIAL_ENAMEL),
            ("MATERIAL_HATCH", dr::MATERIAL_HATCH),
            ("MATERIAL_PEARL", dr::MATERIAL_PEARL),
            ("BAR_STYLE_MASK_MATERIAL", dr::BAR_STYLE_MASK_MATERIAL),
            ("CATEGORY_BAR_MODE_GROUPED", dr::CATEGORY_BAR_MODE_GROUPED),
            ("CATEGORY_BAR_MODE_STACKED", dr::CATEGORY_BAR_MODE_STACKED),
            ("CATEGORY_BAR_MODE_PERCENT", dr::CATEGORY_BAR_MODE_PERCENT),
            ("CATEGORY_BAR_VERTICES", dr::CATEGORY_BAR_VERTICES),
            ("BAR_STYLE_MASK_FILL", dr::BAR_STYLE_MASK_FILL),
            (
                "BAR_STYLE_MASK_BORDER_COLOR",
                dr::BAR_STYLE_MASK_BORDER_COLOR,
            ),
            (
                "BAR_STYLE_MASK_BORDER_WIDTH",
                dr::BAR_STYLE_MASK_BORDER_WIDTH,
            ),
            ("BAR_STYLE_MASK_GAP", dr::BAR_STYLE_MASK_GAP),
            ("BAR_STYLE_MASK_WIDTH_RATIO", dr::BAR_STYLE_MASK_WIDTH_RATIO),
            ("BOX_PLOT_BASE_VERTICES", dr::BOX_PLOT_BASE_VERTICES),
            ("BOX_PLOT_QUAD_VERTICES", dr::BOX_PLOT_QUAD_VERTICES),
            ("BOX_PLOT_PARTS", dr::BOX_PLOT_PARTS),
            ("BOX_PART_BODY", dr::BOX_PART_BODY),
            ("BOX_LANE_OUTLIER_VALUES", dr::BOX_LANE_OUTLIER_VALUES),
            ("BOX_LANE_OUTLIER_OFFSETS", dr::BOX_LANE_OUTLIER_OFFSETS),
            ("BOX_LANE_OUTLIER_GROUPS", dr::BOX_LANE_OUTLIER_GROUPS),
            ("BOX_LANE_MAX_GROUP", dr::BOX_LANE_MAX_GROUP),
            ("BOX_LANE_ABSENT", dr::BOX_LANE_ABSENT),
            ("BOX_LANE_BOX_WIDTH", dr::BOX_LANE_BOX_WIDTH),
            ("BOX_LANE_VIOLIN_CURVES", dr::BOX_LANE_VIOLIN_CURVES),
            ("VIOLIN_VERTICES", dr::VIOLIN_VERTICES),
            ("VIOLIN_LANE_HALVES", dr::VIOLIN_LANE_HALVES as u32),
            (
                "VIOLIN_LANE_FIRST_SEGMENTS",
                dr::VIOLIN_LANE_FIRST_SEGMENTS as u32,
            ),
            ("VIOLIN_HALF_BASE", dr::VIOLIN_HALF_BASE as u32),
            ("VIOLIN_HALF_LANES", dr::VIOLIN_HALF_LANES as u32),
            ("VIOLIN_SIDE_BOTH", dr::VIOLIN_SIDE_BOTH),
            ("VIOLIN_SIDE_LOW", dr::VIOLIN_SIDE_LOW),
            ("VIOLIN_SIDE_HIGH", dr::VIOLIN_SIDE_HIGH),
            ("RANGE_BAR_VERTICES", dr::RANGE_BAR_VERTICES),
            ("RANGE_LANE_END", dr::RANGE_LANE_END),
            ("RANGE_LANE_START", dr::RANGE_LANE_START),
            ("RANGE_LANE_LOW", dr::RANGE_LANE_LOW),
            ("RANGE_LANE_HIGH", dr::RANGE_LANE_HIGH),
            ("RANGE_LANE_COLOR", dr::RANGE_LANE_COLOR),
            ("RANGE_LANE_PROGRESS", dr::RANGE_LANE_PROGRESS),
            ("RANGE_LANE_COLOR_MODE", dr::RANGE_LANE_COLOR_MODE),
            ("RANGE_LANE_STATES", dr::RANGE_LANE_STATES),
            ("RANGE_COLOR_SINGLE", dr::RANGE_COLOR_SINGLE),
            ("RANGE_COLOR_DIRECTION", dr::RANGE_COLOR_DIRECTION),
            ("RANGE_COLOR_STATE", dr::RANGE_COLOR_STATE),
            ("RANGE_SLOT_WHISKER", dr::RANGE_SLOT_WHISKER),
            ("RANGE_SLOT_CONNECTOR", dr::RANGE_SLOT_CONNECTOR),
            ("RANGE_SLOT_UP", dr::RANGE_SLOT_UP),
            ("RANGE_SLOT_DOWN", dr::RANGE_SLOT_DOWN),
            ("RANGE_SLOT_TOTAL", dr::RANGE_SLOT_TOTAL),
            ("RANGE_SLOT_PROGRESS", dr::RANGE_SLOT_PROGRESS),
            ("RANGE_SLOT_STATES", dr::RANGE_SLOT_STATES),
            ("ERRORBAR_HAS_Y", dr::ERRORBAR_HAS_Y),
            ("ERRORBAR_HAS_X", dr::ERRORBAR_HAS_X),
            ("LINE_STEP_SHIFT", LINE_STEP_SHIFT),
            ("LINE_ARC_SHIFT", LINE_ARC_SHIFT),
            ("AREA_FILL_TO_SERIES", AREA_FILL_TO_SERIES),
            ("LINE_STEP_LINEAR", line_step::step_mode(Step::Linear)),
            ("LINE_STEP_PRE", line_step::step_mode(Step::StepPre)),
            ("LINE_STEP_POST", line_step::step_mode(Step::StepPost)),
            ("LINE_STEP_MID", line_step::step_mode(Step::StepMid)),
            ("DATA_PICK_FLAG_FILL", data_pick::DATA_PICK_FLAG_FILL),
            ("DATA_PICK_FLAG_CONTOUR", data_pick::DATA_PICK_FLAG_CONTOUR),
            ("DATA_PICK_FLAG_PAINTED", data_pick::DATA_PICK_FLAG_PAINTED),
            (
                "DATA_PICK_KIND_HISTOGRAM_BIN",
                data_pick::DATA_PICK_KIND_HISTOGRAM_BIN,
            ),
            (
                "DATA_PICK_KIND_MATRIX_CELL",
                data_pick::DATA_PICK_KIND_MATRIX_CELL,
            ),
            (
                "DATA_PICK_KIND_CONTOUR_LEVEL",
                data_pick::DATA_PICK_KIND_CONTOUR_LEVEL,
            ),
            (
                "DATA_PICK_KIND_VALUE_LABEL",
                data_pick::DATA_PICK_KIND_VALUE_LABEL,
            ),
            (
                "DATA_PICK_KIND_MESH_TRIANGLE",
                data_pick::DATA_PICK_KIND_MESH_TRIANGLE,
            ),
            ("VALUE_LABEL_KIND_BARS", dr::VALUE_LABEL_KIND_BARS),
            ("VALUE_LABEL_KIND_BOXES", dr::VALUE_LABEL_KIND_BOXES),
            ("VALUE_LABEL_KIND_RADAR", dr::VALUE_LABEL_KIND_RADAR),
            ("VALUE_LABEL_KIND_RANGE", dr::VALUE_LABEL_KIND_RANGE),
            ("VALUE_LABEL_OUTSIDE", dr::VALUE_LABEL_OUTSIDE),
            ("VALUE_LABEL_INSIDE", dr::VALUE_LABEL_INSIDE),
            ("VALUE_LABEL_AUTO", dr::VALUE_LABEL_AUTO),
            ("VALUE_LABEL_VERTICES", dr::VALUE_LABEL_VERTICES),
            ("MODE_FIELD_EDGES_CELLS", fit::FIELD_EDGES_CELLS_MODE),
            ("MODE_FIELD_EDGES_SAMPLES", fit::FIELD_EDGES_SAMPLES_MODE),
            ("MODE_FIELD_CENTERS_CELLS", fit::FIELD_CENTERS_CELLS_MODE),
            (
                "MODE_FIELD_CENTERS_SAMPLES",
                fit::FIELD_CENTERS_SAMPLES_MODE,
            ),
        ];
        let rust_f32: &[(&str, f32)] = &[
            ("QUAD_MARGIN_PX", dr::MARKER_QUAD_MARGIN_PX),
            ("STROKE_HALF_PX", dr::MARKER_STROKE_HALF_PX),
            ("PLANET_QUAD_EXTENT", crate::gpu_pick::PLANET_QUAD_EXTENT),
            ("CATEGORY_BAR_ROUGHNESS", dr::FLAT_BODY_ROUGHNESS),
        ];
        let mut declared: Vec<(&str, String, u32)> = Vec::new();
        let mut declared_f32: Vec<(&str, String, f32)> = Vec::new();
        for (module_name, assembled) in &every_assembly() {
            let module = parse(module_name, assembled.source);
            for (_, constant) in module.constants.iter() {
                let Some(name) = &constant.name else {
                    continue;
                };
                match module.global_expressions[constant.init] {
                    wgpu::naga::Expression::Literal(wgpu::naga::Literal::U32(value)) => {
                        declared.push((module_name, name.clone(), value));
                    }
                    wgpu::naga::Expression::Literal(wgpu::naga::Literal::F32(value)) => {
                        declared_f32.push((module_name, name.clone(), value));
                    }
                    _ => {}
                }
            }
        }
        for (name, value) in rust {
            let found: Vec<_> = declared.iter().filter(|(_, n, _)| n == name).collect();
            assert!(!found.is_empty(), "no assembled module declares `{name}`");
            for (module, _, wgsl) in found {
                assert_eq!(
                    wgsl, value,
                    "{module}: `{name}` differs from its Rust constant"
                );
            }
        }
        for (name, value) in rust_f32 {
            let found: Vec<_> = declared_f32.iter().filter(|(_, n, _)| n == name).collect();
            assert!(!found.is_empty(), "no assembled module declares `{name}`");
            for (module, _, wgsl) in found {
                assert_eq!(
                    wgsl.to_bits(),
                    value.to_bits(),
                    "{module}: `{name}` is {wgsl}, its Rust constant {value}"
                );
            }
        }
    }

    /// One WGSL struct and the Rust struct that mirrors it: the members as
    /// (WGSL name, Rust byte offset) and the Rust size.
    struct Mirror {
        module: &'static str,
        wgsl: &'static str,
        size: usize,
        members: Vec<(&'static str, usize)>,
    }

    /// `mirror!("module", "WgslStruct", RustType { same_name, ... }; "wgsl_name" = rust_field + extra, ...)`.
    macro_rules! mirror {
        ($module:literal, $wgsl:literal, $ty:ty { $($field:ident),* $(,)? } $(; $($name:literal = $mapped:ident + $add:literal),* $(,)?)?) => {
            Mirror {
                module: $module,
                wgsl: $wgsl,
                size: std::mem::size_of::<$ty>(),
                members: vec![
                    $((stringify!($field), std::mem::offset_of!($ty, $field)),)*
                    $($(($name, std::mem::offset_of!($ty, $mapped) + $add),)*)?
                ],
            }
        };
    }

    /// Every uniform and storage struct the Rust side mirrors has, in the
    /// WGSL module that declares it, the same member names at the same byte
    /// offsets (as naga lays the struct out) and the same size as the
    /// `#[repr(C)]` Rust struct.
    #[test]
    fn mirrored_struct_layouts_match_their_rust_twins() {
        use crate::data_render::vector::VectorParamsGpu;
        use crate::data_render::{
            BarStyleOverrideGpu, BarStyleSlotGpu, CategoryBarParamsGpu, ValueLabelParamsGpu,
        };
        use crate::data_render::{
            ContourLookupMetadataGpu, DataSelectionGpu, FieldParamsGpu, GridColumnGpu, MaterialGpu,
            PointValueMapParamsGpu, PrimitiveStyle, ScatterStyleMapMeta, ScatterStyleOverrideGpu,
            ScatterStyleSlotGpu, ScatterTransform,
        };
        use crate::gpu_contour::{AnchorParamsGpu, LabelAnchorGpu, LabelParamsGpu};
        use crate::gpu_data_pick::{DataPickCandidateGpu, DataPickQueryGpu};
        use crate::gpu_pick::types::{PickCandidateGpu, PickQueryParamsGpu};
        use crate::radial::{RadialPickQueryGpu, RadialPickResultGpu, Sector};
        let transform = |module, wgsl| {
            mirror!(
                "",
                "",
                ScatterTransform {
                    data_min,
                    data_max,
                    data_min_lo,
                    data_max_lo,
                    scale_log,
                    pixel_to_ndc,
                    data_to_panel_scale,
                    data_to_panel_offset,
                    style_params,
                    polar_angle,
                    polar_extent,
                    ternary_lt,
                    ternary_r,
                    breaks,
                    break_info,
                    scale_prob,
                }
            )
            .with(module, wgsl)
        };
        let slot = |module, wgsl| {
            mirror!("", "", ScatterStyleSlotGpu { color_premul }; "params" = meta + 0)
                .with(module, wgsl)
        };
        let over = |module, wgsl| {
            mirror!("", "", ScatterStyleOverrideGpu { point_index, color_premul };
                "_pad0" = _pad + 0, "_pad1" = _pad + 4, "_pad2" = _pad + 8, "params" = meta + 0)
            .with(module, wgsl)
        };
        let material =
            |module| mirror!("", "", MaterialGpu { params, finish }).with(module, "Material");
        // The draw and the pick read the same sector layout.
        let sector = |module| {
            mirror!(
                "",
                "",
                Sector {
                    geometry,
                    arc,
                    color,
                    material,
                    detail,
                    rounding,
                    outline,
                    outline_color,
                    effects,
                    light,
                }
            )
            .with(module, "Sector")
        };
        let meta = |module, wgsl| {
            mirror!(
                "",
                "",
                ScatterStyleMapMeta {
                    style_count,
                    override_count,
                    has_index,
                    _pad
                }
            )
            .with(module, wgsl)
        };
        let mirrors = [
            transform("scatter", "Transform"),
            transform("pick_marker_line", "Transform"),
            mirror!(
                "scatter",
                "Style",
                PrimitiveStyle {
                    color_premul,
                    line_width_px,
                    point_radius_px,
                    cap_half_px,
                    cap_width_px,
                    shape_id,
                    dash_len,
                    series_salt,
                    primitive_flags,
                    dash,
                }
            ),
            mirror!("field", "GridColumn", GridColumnGpu { base, len }),
            mirror!("vector_grid", "GridColumn", GridColumnGpu { base, len }),
            mirror!(
                "field",
                "FieldParams",
                FieldParamsGpu {
                    x_base,
                    y_base,
                    x_len,
                    y_len,
                    cols,
                    rows,
                    level_count,
                    stop_count,
                    flags,
                    opacity,
                    line_width_px,
                    level_color_count,
                    z_min,
                    z_max,
                    image,
                }
            ),
            mirror!(
                "field",
                "ContourLookupMetadata",
                ContourLookupMetadataGpu {
                    finite_count,
                    negative_infinity_count,
                }
            ),
            mirror!(
                "scatter",
                "PointValueMapParams",
                PointValueMapParamsGpu {
                    z_min,
                    z_max,
                    size_domain_min,
                    size_domain_max,
                    size_px,
                    flags,
                    stop_count,
                    nan_color_premul,
                }
            ),
            mirror!(
                "line",
                "PointValueMapParams",
                PointValueMapParamsGpu {
                    z_min,
                    z_max,
                    size_domain_min,
                    size_domain_max,
                    size_px,
                    flags,
                    stop_count,
                    nan_color_premul,
                }
            ),
            mirror!(
                "vector_grid",
                "VectorParams",
                VectorParamsGpu {
                    x_base,
                    y_base,
                    x_len,
                    y_len,
                    u_base,
                    u_len,
                    v_base,
                    v_len,
                    c_base,
                    c_len,
                    cols,
                    rows,
                    flags,
                    count,
                    stop_count,
                    pivot,
                    scale,
                    width_px,
                    head_length_px,
                    head_width_px,
                    min_length_px,
                    aa_px,
                    spacing_px,
                    pad1,
                    height_base,
                    height_len,
                    w_base,
                    w_len,
                    z_min,
                    z_max,
                    nan_color_premul,
                }
            ),
            material("bar"),
            material("radial"),
            material("radial_pick"),
            sector("radial"),
            sector("radial_pick"),
            mirror!(
                "radial_pick",
                "RadialPickQuery",
                RadialPickQueryGpu { pixel, count, rest }
            ),
            mirror!(
                "radial_pick",
                "RadialPickResult",
                RadialPickResultGpu {
                    index,
                    hit,
                    t,
                    reserved
                }
            ),
            slot("scatter", "ScatterStyleSlot"),
            over("scatter", "ScatterStyleOverride"),
            meta("scatter", "ScatterStyleMapMeta"),
            slot("errorbar", "ErrorBarStyleSlot"),
            over("errorbar", "ErrorBarStyleOverride"),
            meta("errorbar", "ErrorBarStyleMapMeta"),
            slot("pick_marker", "PickScatterStyleSlot"),
            over("pick_marker", "PickScatterStyleOverride"),
            // The error-bar rows share the marker rows' buffers and bytes.
            slot("pick_marker", "ErrorBarStyleSlot"),
            slot("pick_marker_line", "ErrorBarStyleSlot"),
            mirror!(
                "bar",
                "BarStyleSlot",
                BarStyleSlotGpu {
                    fill_color_premul,
                    border_color_premul,
                    params,
                    material,
                    finish,
                }
            ),
            mirror!("bar", "BarStyleOverride", BarStyleOverrideGpu {
                bin_index, fill_color_premul, border_color_premul, params, material, finish,
            }; "_pad0" = _pad + 0, "_pad1" = _pad + 4, "_pad2" = _pad + 8),
            meta("bar", "BarStyleMapMeta"),
            mirror!(
                "bar",
                "CategoryBarParams",
                CategoryBarParamsGpu {
                    positions,
                    count,
                    member,
                    members,
                    mode,
                    horizontal,
                    gap_px,
                    group_width,
                    corner_radius_px,
                    emphasis,
                    scale,
                    seed,
                    slot_half,
                    wedge,
                    _reserved,
                }
            ),
            transform("bar", "Transform"),
            mirror!(
                "bar",
                "DataSelection",
                DataSelectionGpu {
                    color_premul,
                    metrics,
                    indices
                }
            ),
            mirror!(
                "field",
                "DataSelection",
                DataSelectionGpu {
                    color_premul,
                    metrics,
                    indices
                }
            ),
            mirror!(
                "field",
                "DataPickQuery",
                DataPickQueryGpu {
                    cursor_ndc_t,
                    limits,
                    data,
                    bases,
                    baseline,
                }
            ),
            mirror!(
                "field",
                "DataPickCandidate",
                DataPickCandidateGpu {
                    valid,
                    paint_order,
                    kind,
                    index0,
                    index1,
                    index2,
                    distance_px,
                    primitive_order,
                }
            ),
            mirror!(
                "pick_marker_line",
                "PickQueryParams",
                PickQueryParamsGpu {
                    transform,
                    cursor_chart,
                    chart_limits,
                    scatter_line,
                    data,
                    style,
                    series,
                    stream,
                    line,
                    size_map,
                    size_domain,
                    size_px,
                    line_px,
                    line_dash,
                    styled,
                    vector,
                    vector_px,
                    vector_min,
                    errorbar,
                    errorbar_lanes,
                    errorbar_rows,
                    errorbar_px,
                    area,
                    area_baseline,
                    category,
                    category_slot,
                    stem,
                    stem_dash,
                    stem_info,
                }
            ),
            mirror!(
                "pick_gate",
                "PickCandidate",
                PickCandidateGpu {
                    valid,
                    series_order,
                    point_index,
                    primitive_kind,
                    primitive_index,
                    distance_sq,
                    distance_px,
                    _pad,
                }
            ),
            // `projection_steps` is the reserved lane the Rust side calls `_pad0`.
            mirror!("contour_anchor", "AnchorParams", AnchorParamsGpu {
                lattice_x, lattice_y, level_count, kept_capacity, area_x, area_y, area_w, area_h,
                clip_x, clip_y, clip_w, clip_h, spacing_px, label_h_px, label_vertices,
            }; "projection_steps" = _pad0 + 0),
            mirror!(
                "contour_anchor",
                "LabelAnchor",
                LabelAnchorGpu {
                    x,
                    y,
                    dir,
                    level,
                    width_px
                }
            ),
            mirror!(
                "bar",
                "ValueLabelParams",
                ValueLabelParamsGpu {
                    atlas,
                    shape,
                    metrics,
                    selection_color,
                    selection,
                    radar,
                    ring,
                }
            ),
            mirror!(
                "bar",
                "LabelParams",
                LabelParamsGpu {
                    atlas_w,
                    atlas_h,
                    cell_stride_w,
                    cell_stride_h,
                    columns,
                    rows,
                    gutter,
                    level_count,
                }
            ),
            mirror!(
                "contour_label",
                "LabelParams",
                LabelParamsGpu {
                    atlas_w,
                    atlas_h,
                    cell_stride_w,
                    cell_stride_h,
                    columns,
                    rows,
                    gutter,
                    level_count,
                }
            ),
        ];
        for mirror in mirrors {
            let source = super::module(mirror.module).unwrap();
            let module = parse(mirror.module, source);
            let (members, span) = module
                .types
                .iter()
                .find_map(|(_, ty)| match &ty.inner {
                    wgpu::naga::TypeInner::Struct { members, span }
                        if ty.name.as_deref() == Some(mirror.wgsl) =>
                    {
                        Some((members.clone(), *span))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{}: no struct `{}`", mirror.module, mirror.wgsl));
            let mut wgsl: Vec<(String, usize)> = members
                .iter()
                .map(|m| (m.name.clone().unwrap_or_default(), m.offset as usize))
                .collect();
            let mut rust: Vec<(String, usize)> = mirror
                .members
                .iter()
                .map(|(name, offset)| (name.to_string(), *offset))
                .collect();
            wgsl.sort_by_key(|(_, offset)| *offset);
            rust.sort_by_key(|(_, offset)| *offset);
            assert_eq!(wgsl, rust, "{}: `{}` members", mirror.module, mirror.wgsl);
            assert_eq!(
                span as usize, mirror.size,
                "{}: `{}` size",
                mirror.module, mirror.wgsl
            );
        }
    }

    impl Mirror {
        fn with(self, module: &'static str, wgsl: &'static str) -> Self {
            Mirror {
                module,
                wgsl,
                size: self.size,
                members: self.members,
            }
        }
    }

    /// Every module's source is its fragment files concatenated in the
    /// listed order, one newline between two, each file verbatim.
    #[test]
    fn every_module_is_its_fragments_concatenated() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for (name, assembled) in &every_assembly() {
            let parts: Vec<String> = assembled
                .fragments
                .iter()
                .map(|fragment| std::fs::read_to_string(src.join(fragment)).unwrap())
                .collect();
            assert_eq!(assembled.source, parts.join("\n"), "{name}");
        }
    }

    /// Every name in [`super::MODULES`] is listed once.
    #[test]
    fn assembled_module_names_are_unique() {
        for (i, (name, _)) in super::MODULES.iter().enumerate() {
            assert!(
                super::MODULES[..i].iter().all(|(other, _)| other != name),
                "{name} is listed twice"
            );
        }
    }
}
