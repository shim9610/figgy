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

/// Every assembled module, by name: the source each pipeline compiles and
/// its fragments.
pub const MODULES: &[(&str, Assembled)] = &[
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
    ("radial_pick", crate::radial::RADIAL_PICK_SHADER),
    (
        "vector_points",
        crate::data_render::vector::VECTOR_POINTS_SHADER,
    ),
    (
        "vector_grid",
        crate::data_render::vector::VECTOR_GRID_SHADER,
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

/// The source of the assembled module called `name` in [`MODULES`].
///
/// # Panics
///
/// When no module has that name.
pub fn module(name: &str) -> &'static str {
    match MODULES.iter().find(|(n, _)| *n == name) {
        Some((_, assembled)) => assembled.source,
        None => panic!("no assembled WGSL module named `{name}`"),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    /// Every assembled module parses and validates as WGSL, with no GPU: a
    /// fragment that breaks a module fails here, not on the first draw.
    #[test]
    fn every_assembled_module_parses_and_validates() {
        for (name, assembled) in super::MODULES {
            let source = assembled.source;
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
        }
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
        use crate::{data_render as dr, gpu_data_pick as data_pick, gpu_errorbar as fit};
        let rust: &[(&str, u32)] = &[
            ("FIELD_COLUMNS_ARE_Y", dr::FIELD_FLAG_COLUMNS_ARE_Y),
            ("FIELD_CENTERS", dr::FIELD_FLAG_CENTERS),
            ("FIELD_INTERPOLATED", dr::FIELD_FLAG_INTERPOLATED),
            ("FIELD_BANDS", dr::FIELD_FLAG_BANDS),
            ("FIELD_LOG_Z", dr::FIELD_FLAG_LOG_Z),
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
        for (module_name, assembled) in super::MODULES {
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
        use crate::data_render::{BarStyleOverrideGpu, BarStyleSlotGpu, CategoryBarParamsGpu};
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
                    pad0,
                    pad1,
                    z_min,
                    z_max,
                    nan_color_premul,
                }
            ),
            material("boxplot"),
            material("categorical"),
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
                    values,
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
            let source = super::module(mirror.module);
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
        for (name, assembled) in super::MODULES {
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
