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
//! A module that places data is assembled twice, by `assembled_by_set!`
//! (#299): basic, without the code only some charts need, and full
//! (`ShaderSet`). `MODULES_BY_SET` names those.

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

/// Which of its two assemblies a data module is compiled from (#299).
///
/// Three features cost every data pipeline most of its compile time in a
/// browser and are used by few charts: the cut remap of a broken axis
/// (#200), a probability scale (#209) and polar coordinates. Every module
/// that places data is therefore assembled twice, from one fragment list
/// (`assembled_by_set!`): [`Self::Full`] with the fragments that hold
/// that code and [`Self::Basic`] with their basic twins
/// ([`SET_FRAGMENTS`]), which have the same signatures and none of it. A
/// renderer compiles the basic modules until it is first handed a chart with
/// an axis break, a probability axis or polar coordinates, and the full
/// modules from then on (`Renderer::admit_full_shaders`); such a chart is
/// never drawn or picked through a basic module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShaderSet {
    /// The basic twins: no chart with an axis break, a probability axis or
    /// polar coordinates has been seen.
    #[default]
    Basic,
    /// Every feature.
    Full,
}

/// The fragments the two sets differ in, as `(basic, full)` pairs: the
/// full fragment holds a feature's code and its basic twin the same
/// signatures without it. The first four go where a module's list says
/// `variant`, in this order; the last where it says `field_polar`, in the
/// modules that splice `field_grid.wgsl`.
pub const SET_FRAGMENTS: &[(&str, &str)] = &[
    // The cut remap of a broken axis (#200).
    (
        "data_render/common/axis_cuts_basic.wgsl",
        "data_render/common/axis_cuts.wgsl",
    ),
    // A probability scale (#209).
    (
        "data_render/common/probability_basic.wgsl",
        "data_render/common/probability.wgsl",
    ),
    // Where a value sits on its axis: linear and logarithmic in both, and
    // the two above in the full one.
    (
        "data_render/common/axis_scale_basic.wgsl",
        "data_render/common/axis_scale.wgsl",
    ),
    // Polar coordinates (`Config.polar`).
    (
        "data_render/common/polar_basic.wgsl",
        "data_render/common/polar.wgsl",
    ),
    // The polar half of the field grid's cell lookup (#208).
    (
        "data_render/common/field_polar_basic.wgsl",
        "data_render/common/field_polar.wgsl",
    ),
];

/// One module assembled twice from the same fragment list, apart from the
/// fragments of [`SET_FRAGMENTS`] ([`ShaderSet`]).
#[derive(Clone, Copy, Debug)]
pub struct AssembledBySet {
    /// With the basic twins.
    pub basic: Assembled,
    /// With the full fragments.
    pub full: Assembled,
}

impl AssembledBySet {
    /// The assembly for `set`.
    pub const fn of(&self, set: ShaderSet) -> Assembled {
        match set {
            ShaderSet::Basic => self.basic,
            ShaderSet::Full => self.full,
        }
    }
}

/// An [`AssembledBySet`] from one literal list of fragment files, with
/// the word `set` where the axis and polar fragments go and the word
/// `field_polar` after `field_grid.wgsl`: [`assembled!`] over the list with
/// the basic fragments of [`SET_FRAGMENTS`] there and again with the full
/// ones, so the two modules cannot differ in anything else.
macro_rules! assembled_by_set {
    ($($list:tt)+) => {
        $crate::shader_assembly::assembled_by_set_step!([] [] $($list)+)
    };
}
pub(crate) use assembled_by_set;

/// [`assembled_by_set!`], one list entry at a time: the basic and the
/// full list so far, then the entries left.
macro_rules! assembled_by_set_step {
    ([$($basic:literal,)*] [$($full:literal,)*]) => {
        $crate::shader_assembly::AssembledBySet {
            basic: $crate::shader_assembly::assembled!($($basic),*),
            full: $crate::shader_assembly::assembled!($($full),*),
        }
    };
    ([$($basic:literal,)*] [$($full:literal,)*] set $(, $($rest:tt)*)?) => {
        $crate::shader_assembly::assembled_by_set_step!(
            [
                $($basic,)*
                "data_render/common/axis_cuts_basic.wgsl",
                "data_render/common/probability_basic.wgsl",
                "data_render/common/axis_scale_basic.wgsl",
                "data_render/common/polar_basic.wgsl",
            ]
            [
                $($full,)*
                "data_render/common/axis_cuts.wgsl",
                "data_render/common/probability.wgsl",
                "data_render/common/axis_scale.wgsl",
                "data_render/common/polar.wgsl",
            ]
            $($($rest)*)?
        )
    };
    ([$($basic:literal,)*] [$($full:literal,)*] field_polar $(, $($rest:tt)*)?) => {
        $crate::shader_assembly::assembled_by_set_step!(
            [$($basic,)* "data_render/common/field_polar_basic.wgsl",]
            [$($full,)* "data_render/common/field_polar.wgsl",]
            $($($rest)*)?
        )
    };
    ([$($basic:literal,)*] [$($full:literal,)*] $fragment:literal $(, $($rest:tt)*)?) => {
        $crate::shader_assembly::assembled_by_set_step!(
            [$($basic,)* $fragment,]
            [$($full,)* $fragment,]
            $($($rest)*)?
        )
    };
}
pub(crate) use assembled_by_set_step;

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
    (
        "column_expand",
        crate::data_render::column_pool::COLUMN_EXPAND_SHADER,
    ),
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
    ("pick_nearest", crate::gpu_pick::NEAREST_PICK_MODULE.1.full),
    (
        "pick_nearest_3d",
        crate::gpu_pick::NEAREST_PICK_3D_MODULE.1.full,
    ),
];

/// Every module assembled by variant ([`AssembledBySet`], #299), by
/// name: the modules that splice `projection.wgsl`. [`MODULES`] lists the
/// full assembly of each under the same name where it has one.
pub const MODULES_BY_SET: &[(&str, AssembledBySet)] = &[
    ("scatter", crate::data_render::SCATTER_SHADER_BY_SET),
    ("line", crate::data_render::LINE_SHADER_BY_SET),
    ("errorbar", crate::data_render::ERRORBAR_SHADER_BY_SET),
    ("bar", crate::data_render::BAR_SHADER_BY_SET),
    ("field", crate::data_render::FIELD_SHADER_BY_SET),
    (
        "tri_mesh",
        crate::data_render::tri_mesh::TRI_MESH_SHADER_BY_SET,
    ),
    (
        "surface",
        crate::data_render::surface_mesh::SURFACE_SHADER_BY_SET,
    ),
    (
        "tri_surface",
        crate::data_render::surface_mesh::TRI_SURFACE_SHADER_BY_SET,
    ),
    (
        "ternary_region",
        crate::data_render::ternary_region::TERNARY_REGION_SHADER_BY_SET,
    ),
    (
        "line_arc",
        crate::data_render::line_arc::LINE_ARC_SHADER_BY_SET,
    ),
    (
        "contour_anchor",
        crate::gpu_contour::CONTOUR_ANCHOR_SHADER_BY_SET,
    ),
    (
        "contour_label",
        crate::gpu_contour::CONTOUR_LABEL_SHADER_BY_SET,
    ),
    (
        "vector_points",
        crate::data_render::vector::VECTOR_POINTS_SHADER_BY_SET,
    ),
    (
        "vector_grid",
        crate::data_render::vector::VECTOR_GRID_SHADER_BY_SET,
    ),
    (
        "vector_path",
        crate::data_render::vector::VECTOR_PATH_SHADER_BY_SET,
    ),
    (
        "vector_points_3d",
        crate::data_render::vector::VECTOR_POINTS_3D_SHADER_BY_SET,
    ),
    ("pick_gate", crate::gpu_pick::PickShader::Gate.by_set()),
    ("pick_marker", crate::gpu_pick::PickShader::Marker.by_set()),
    (
        "pick_marker_line",
        crate::gpu_pick::PickShader::MarkerLine.by_set(),
    ),
    ("pick_line", crate::gpu_pick::PickShader::Line.by_set()),
    ("pick_vector", crate::gpu_pick::PickShader::Vector.by_set()),
    ("pick_nearest", crate::gpu_pick::NEAREST_PICK_MODULE.1),
    ("pick_nearest_3d", crate::gpu_pick::NEAREST_PICK_3D_MODULE.1),
];

/// The source of the module called `name` in [`MODULES_BY_SET`] assembled
/// for `set`, or `None` when no module there has that name.
pub fn module_by_set(name: &str, set: ShaderSet) -> Option<&'static str> {
    MODULES_BY_SET
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, by_set)| by_set.of(set).source)
}

/// The basic assembly's source for a full assembly's `source`, and `source`
/// itself for any other: the two compile to pipelines with the same
/// descriptors, so the browser prewarm contract
/// (`shader_pipeline_contract.json`) stores each pipeline once, under the
/// basic source.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn contract_source(source: &str) -> &str {
    MODULES_BY_SET
        .iter()
        .find(|(_, by_set)| by_set.full.source == source)
        .map_or(source, |(_, by_set)| by_set.basic.source)
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
    /// assemblies of [`super::MODULES_BY_SET`] (#299), each under a name
    /// that says which it is.
    fn every_assembly() -> Vec<(&'static str, super::Assembled)> {
        let mut all: Vec<(&'static str, super::Assembled)> = super::MODULES.to_vec();
        for (name, by_set) in super::MODULES_BY_SET {
            all.push((
                Box::leak(format!("{name} (basic)").into_boxed_str()),
                by_set.basic,
            ));
            all.push((
                Box::leak(format!("{name} (full)").into_boxed_str()),
                by_set.full,
            ));
        }
        all
    }

    const FIELD_GRID: &str = "data_render/common/field_grid.wgsl";
    const PROJECTION: &str = "data_render/common/projection.wgsl";

    /// The two assemblies of a module differ in the fragments of
    /// [`super::SET_FRAGMENTS`] alone (#299): the four of `set`, in
    /// the table's order, and the field grid's where the module splices
    /// `field_grid.wgsl`. Every module that splices `projection.wgsl` is
    /// assembled by variant, and [`super::MODULES`] names the full assembly.
    #[test]
    fn modules_by_set_differ_in_the_variant_fragments_alone() {
        use super::{SET_FRAGMENTS, ShaderSet};
        let listed = |fragment: &str| {
            SET_FRAGMENTS
                .iter()
                .any(|(basic, full)| *basic == fragment || *full == fragment)
        };
        for (name, by_set) in super::MODULES_BY_SET {
            let (basic, full) = (by_set.basic.fragments, by_set.full.fragments);
            assert_eq!(basic.len(), full.len(), "{name}");
            let mut pairs = Vec::new();
            for (a, b) in basic.iter().zip(full) {
                if a == b {
                    assert!(!listed(a), "{name}: {a}");
                    continue;
                }
                pairs.push((*a, *b));
            }
            let field = basic.contains(&FIELD_GRID);
            let expected = if field {
                SET_FRAGMENTS
            } else {
                &SET_FRAGMENTS[..SET_FRAGMENTS.len() - 1]
            };
            assert_eq!(pairs, expected, "{name}");
            assert!(basic.contains(&PROJECTION), "{name}");
            assert_eq!(by_set.of(ShaderSet::Basic).source, by_set.basic.source);
            assert_eq!(by_set.of(ShaderSet::Full).source, by_set.full.source);
            assert_ne!(by_set.basic.source, by_set.full.source, "{name}");
            assert_eq!(
                super::contract_source(by_set.full.source),
                by_set.basic.source,
                "{name}"
            );
            assert_eq!(
                super::contract_source(by_set.basic.source),
                by_set.basic.source,
                "{name}"
            );
            assert_eq!(
                super::module_by_set(name, ShaderSet::Basic),
                Some(by_set.basic.source)
            );
            // The plain name is the full assembly: the full contract.
            if let Some(plain) = super::module(name) {
                assert_eq!(plain, by_set.full.source, "{name}");
            }
        }
        // No module outside the table splices a variant fragment, the
        // projection that calls them or the field grid.
        for (name, assembled) in super::MODULES {
            let by_set = super::MODULES_BY_SET.iter().any(|(n, _)| n == name);
            for fragment in assembled.fragments {
                let basic = SET_FRAGMENTS.iter().any(|(basic, _)| basic == fragment);
                assert!(!basic, "{name}: {fragment}");
                if listed(fragment) || *fragment == PROJECTION || *fragment == FIELD_GRID {
                    assert!(by_set, "{name}: {fragment}");
                }
            }
        }
    }

    /// The functions a fragment file declares, in order.
    fn declared_functions(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| line.starts_with("fn "))
            .map(|line| line[3..].split('(').next().unwrap().trim())
            .collect()
    }

    /// Every `.wgsl` file under `path`.
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

    /// A basic fragment declares what its full twin's callers use, with the
    /// same signatures, and nothing else (#299); a type both declare has the
    /// same members unless only its own fragment reads them (`AxisCuts`).
    /// No other fragment calls a function only a full fragment has, so no
    /// caller can come to depend on one set.
    #[test]
    fn a_basic_fragment_has_its_full_twins_signatures() {
        use wgpu::naga::{Module, TypeInner};
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let read = |fragment: &str| std::fs::read_to_string(src.join(fragment)).unwrap();
        // The field module splices every pair.
        let field = crate::data_render::FIELD_SHADER_BY_SET;
        let basic_module = parse("field (basic)", field.basic.source);
        let full_module = parse("field (full)", field.full.source);
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
        let members = |module: &Module, name: &str| -> Option<Vec<(String, String)>> {
            let (_, ty) = module
                .types
                .iter()
                .find(|(_, ty)| ty.name.as_deref() == Some(name))?;
            let TypeInner::Struct { members, .. } = &ty.inner else {
                return None;
            };
            Some(
                members
                    .iter()
                    .map(|member| (member.name.clone().unwrap(), type_text(module, member.ty)))
                    .collect(),
            )
        };
        let mut files = Vec::new();
        wgsl_files(&src, &mut files);
        let is_full = |file: &std::path::Path| {
            super::SET_FRAGMENTS
                .iter()
                .any(|(_, full)| file.ends_with(full))
        };
        let mut shared_total = 0;
        for (basic, full) in super::SET_FRAGMENTS {
            let (basic_text, full_text) = (read(basic), read(full));
            let shared = declared_functions(&basic_text);
            let full_names = declared_functions(&full_text);
            assert!(!shared.is_empty(), "{basic}");
            for name in &shared {
                assert!(full_names.contains(name), "{full} lacks {name}");
                let found = signature(&basic_module, name);
                assert!(found.is_some(), "{basic}: {name}");
                assert_eq!(found, signature(&full_module, name), "{name}");
            }
            shared_total += shared.len();
            // A function only the full fragment has is called by full
            // fragments alone.
            for private in full_names.iter().filter(|name| !shared.contains(name)) {
                let call = format!("{private}(");
                for file in files.iter().filter(|file| !is_full(file)) {
                    let text = std::fs::read_to_string(file).unwrap();
                    let called = text.match_indices(&call).any(|(at, _)| {
                        !text[..at]
                            .chars()
                            .next_back()
                            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    });
                    assert!(!called, "{} calls `{private}` of {full}", file.display());
                }
            }
            // A struct both declare.
            for line in basic_text
                .lines()
                .filter(|line| line.starts_with("struct "))
            {
                let name = line["struct ".len()..].split([' ', '{']).next().unwrap();
                assert!(full_text.contains(&format!("struct {name} ")), "{name}");
                if name != "AxisCuts" {
                    let found = members(&basic_module, name);
                    assert!(found.is_some(), "{name}");
                    assert_eq!(found, members(&full_module, name), "{name}");
                }
            }
        }
        // The count the docs give (SHADER_COMMON.md §4).
        assert_eq!(shared_total, 5 + 4 + 2 + 4 + 3);
        // No other fragment reads a member of `AxisCuts` or builds one.
        let cut_fragments = super::SET_FRAGMENTS[0];
        for file in &files {
            if file.ends_with(cut_fragments.0) || file.ends_with(cut_fragments.1) {
                continue;
            }
            let text = std::fs::read_to_string(file).unwrap();
            for private in ["count_scale", ".k0", ".k1", ".k2", "AxisCuts("] {
                assert!(
                    !text.contains(private),
                    "{} uses `{private}` of axis_cuts.wgsl",
                    file.display()
                );
            }
        }
    }

    /// A draw function that hands the whole `transform` uniform to a call
    /// does so once (#299): each read of the whole uniform compiles as its
    /// own copy of the struct, so a function with several such calls reads
    /// it into a local first (`transform.wgsl`). Reading a member is not a
    /// read of the whole.
    #[test]
    fn a_draw_function_reads_the_whole_transform_once() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        wgsl_files(&src, &mut files);
        // The pick modules read `pick_query_params.transform`.
        files.retain(|file| !file.starts_with(src.join("gpu_pick")));
        let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
        let mut checked = 0;
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let code: String = text
                .lines()
                .map(|line| line.split("//").next().unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            // Each function: from its `fn` to the next one.
            for function in code.split("\nfn ").skip(1) {
                let whole = function
                    .match_indices("transform")
                    .filter(|(at, word)| {
                        let before = function[..*at].chars().next_back();
                        let after = function[at + word.len()..].trim_start().chars().next();
                        !before.is_some_and(|c| is_name(c) || c == '.')
                            && !after.is_some_and(|c| is_name(c) || matches!(c, '.' | '[' | ':'))
                    })
                    .count();
                let name = function.split('(').next().unwrap();
                assert!(
                    whole <= 1,
                    "{}: `{name}` reads the whole transform {whole} times",
                    file.display()
                );
                checked += whole;
            }
        }
        assert!(checked > 40, "{checked}");
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

    /// The names of the functions `name` calls, directly or not.
    fn reached(module: &wgpu::naga::Module, name: &str) -> Vec<String> {
        use wgpu::naga::{Block, Statement};
        fn calls(block: &Block, out: &mut Vec<wgpu::naga::Handle<wgpu::naga::Function>>) {
            for statement in block.iter() {
                match statement {
                    Statement::Call { function, .. } => out.push(*function),
                    Statement::Block(inner) => calls(inner, out),
                    Statement::If { accept, reject, .. } => {
                        calls(accept, out);
                        calls(reject, out);
                    }
                    Statement::Loop {
                        body, continuing, ..
                    } => {
                        calls(body, out);
                        calls(continuing, out);
                    }
                    Statement::Switch { cases, .. } => {
                        for case in cases {
                            calls(&case.body, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let body = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some(name))
            .map(|(_, f)| &f.body)
            .or_else(|| {
                module
                    .entry_points
                    .iter()
                    .find(|entry| entry.name == name)
                    .map(|entry| &entry.function.body)
            })
            .unwrap_or_else(|| panic!("{name}"));
        let mut pending = Vec::new();
        calls(body, &mut pending);
        let mut seen = Vec::new();
        while let Some(handle) = pending.pop() {
            if seen.contains(&handle) {
                continue;
            }
            seen.push(handle);
            calls(&module.functions[handle].body, &mut pending);
        }
        seen.into_iter()
            .filter_map(|handle| module.functions[handle].name.clone())
            .collect()
    }

    /// The basic set's contour entries compile none of the polar contour
    /// distance's foot-point walk (#453): their `contour_foot_advance` calls
    /// nothing, where the full set's searches the cell
    /// (`field_polar.wgsl`). Both reach it from every contour entry.
    #[test]
    fn the_basic_contour_entries_compile_no_polar_walk() {
        let field = crate::data_render::FIELD_SHADER_BY_SET;
        let basic = parse("field (basic)", field.basic.source);
        let full = parse("field (full)", field.full.source);
        for entry in [
            "fs_contour",
            "fs_contour_labelled",
            "fs_floor_contour",
            "pick_field_contour",
        ] {
            for module in [&basic, &full] {
                assert!(
                    reached(module, entry)
                        .iter()
                        .any(|f| f == "contour_foot_advance"),
                    "{entry}"
                );
            }
        }
        assert_eq!(
            reached(&basic, "contour_foot_advance"),
            Vec::<String>::new()
        );
        let walk = reached(&full, "contour_foot_advance");
        for searched in ["field_lookup", "locate_step", "contour_sample_cells"] {
            assert!(walk.iter().any(|f| f == searched), "{searched}");
        }
        assert!(!field.basic.source.contains("CONTOUR_POLAR_FOOT_STEPS"));
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
            ("RANGE_COLOR_ROW", dr::RANGE_COLOR_ROW),
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
                    ink_dark,
                    ink_light,
                    ink_back,
                    ink_base,
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
