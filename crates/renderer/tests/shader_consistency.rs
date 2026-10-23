//! Mechanical SSoT check for WGSL shader common blocks.
//!
//! WGSL has no `import`/`include`. The `Transform` / `Style` / `maybe_log` /
//! `data_to_ndc` definitions are therefore duplicated across
//! `scatter_columnar.wgsl`, `line_columnar.wgsl`, `errorbar_columnar.wgsl`,
//! `bar_columnar.wgsl`, `field_columnar.wgsl`, the `line_arc.wgsl` and
//! `contour_anchor.wgsl` compute shaders, the `contour_label.wgsl`
//! render shader and the vector shader's projection fragment
//! `vector/project_cartesian2d.wgsl`. `src/data_render/SHADER_COMMON.md` is
//! the single source of truth for those duplicates.
//!
//! This test parses SHADER_COMMON.md for fenced WGSL blocks that are marked
//! with a metadata comment of the form
//!
//!     <!-- shader-common: applies-to=scatter,line,errorbar -->
//!
//! immediately before the fence. For each such block it verifies that every
//! listed shader file's `BEGIN common block` / `END common block` region
//! exactly equals the SSoT blocks that apply to that shader, concatenated in
//! SSoT order. Extra local structs, comments, or stale definitions inside the
//! region fail the test with a clear diff-style report.
//!
//! Run manually with:
//!     cargo test --test shader_consistency
//!
//! CI and local development can run this gate whenever a `.wgsl` file or
//! `SHADER_COMMON.md` changes.
//!
//! The file also holds the #46 source guards, each with an allowlist that
//! only shrinks: one definition per WGSL function, and no `..` rest pattern
//! on a model type in the Rust product code of the model, renderer and web
//! crates.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const SSOT_PATH: &str = "src/data_render/SHADER_COMMON.md";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The point picker's assembled modules, concatenated: every twin a pick
/// module carries is in one of them.
const PICK_FRAGMENTS: &str = "src/gpu_pick/*.wgsl";

/// The assembled module that compiles a shader file's text, by the file it
/// started from: the twins and content checks below read what the GPU
/// compiles, wherever the assembler took each piece from.
fn assembled_module_for(path_rel: &str) -> Option<&'static str> {
    Some(match path_rel {
        "src/data_render/scatter_columnar.wgsl" => "scatter",
        "src/data_render/line_columnar.wgsl" => "line",
        "src/data_render/errorbar_columnar.wgsl" => "errorbar",
        "src/data_render/bar_columnar.wgsl" => "bar",
        "src/data_render/field_columnar.wgsl" => "field",
        "src/data_render/line_arc.wgsl" => "line_arc",
        "src/contour_anchor.wgsl" => "contour_anchor",
        "src/contour_label.wgsl" => "contour_label",
        "src/gpu_errorbar.wgsl" => "errorbar_fit",
        "src/data_render/vector/project_cartesian2d.wgsl"
        | "src/data_render/vector/vector_input_common.wgsl"
        | "src/data_render/common/arrow.wgsl" => "vector_grid",
        _ => return None,
    })
}

/// A source file, except that a shader is read as the assembled module it is
/// part of (see [`assembled_module_for`]), [`PICK_FRAGMENTS`] as every pick
/// module and [`SSOT_PATH`] as the document followed by the shared fragment
/// files under `src/data_render/common/`.
fn read(path_rel: &str) -> String {
    if path_rel == PICK_FRAGMENTS {
        return renderer::shader_assembly::MODULES
            .iter()
            .filter(|(name, _)| name.starts_with("pick_"))
            .map(|(_, source)| *source)
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(name) = assembled_module_for(path_rel) {
        return renderer::shader_assembly::module(name).to_string();
    }
    let p = workspace_root().join(path_rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| panic!("Failed to read {:?}: {}", p, e));
    // Normalize line endings so CRLF (Windows) vs LF doesn't trip substring
    // matching when comparing SSoT text against shader source.
    let mut text = raw.replace("\r\n", "\n");
    if path_rel == SSOT_PATH {
        let common = workspace_root().join("src/data_render/common");
        if let Ok(entries) = fs::read_dir(&common) {
            let mut fragments: Vec<_> = entries.map(|entry| entry.unwrap().path()).collect();
            fragments.sort();
            for fragment in fragments {
                text.push('\n');
                text.push_str(&fs::read_to_string(fragment).unwrap().replace("\r\n", "\n"));
            }
        }
    }
    text
}

fn shader_path_for(short: &str) -> &'static str {
    match short {
        "scatter" => "src/data_render/scatter_columnar.wgsl",
        "line" => "src/data_render/line_columnar.wgsl",
        "errorbar" => "src/data_render/errorbar_columnar.wgsl",
        "bar" => "src/data_render/bar_columnar.wgsl",
        "field" => "src/data_render/field_columnar.wgsl",
        "arc" => "src/data_render/line_arc.wgsl",
        "anchor" => "src/contour_anchor.wgsl",
        "label" => "src/contour_label.wgsl",
        "vector" => "src/data_render/vector/project_cartesian2d.wgsl",
        other => panic!(
            "Unknown shader short-name `{}` in SHADER_COMMON.md metadata. \
             Valid names: scatter | line | errorbar | bar | field | arc | \
             anchor | label | vector.",
            other
        ),
    }
}

#[derive(Debug)]
struct CommonBlock {
    applies_to: Vec<String>,
    body: String,
}

/// Parse all `<!-- shader-common: applies-to=... -->` markers followed by a
/// fenced ```wgsl ... ``` block in SHADER_COMMON.md.
fn parse_ssot_blocks(md: &str) -> Vec<CommonBlock> {
    let lines: Vec<&str> = md.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(applies) = parse_applies_marker(lines[i]) {
            let marker_line = i + 1;
            // Find the next ```wgsl fence (allow blank lines in between).
            let mut j = i + 1;
            while j < lines.len() && !is_wgsl_fence_open(lines[j]) {
                // Allow only blank lines between marker and fence.
                if !lines[j].trim().is_empty() {
                    panic!(
                        "SHADER_COMMON.md line {}: shader-common marker must be \
                         followed by a ```wgsl fence (only blank lines may \
                         appear in between). Found: {:?}",
                        marker_line, lines[j]
                    );
                }
                j += 1;
            }
            if j >= lines.len() {
                panic!(
                    "SHADER_COMMON.md line {}: shader-common marker has no \
                     ```wgsl fence after it.",
                    marker_line
                );
            }
            let body_start = j + 1;
            let mut k = body_start;
            while k < lines.len() && !is_fence_close(lines[k]) {
                k += 1;
            }
            if k >= lines.len() {
                panic!(
                    "SHADER_COMMON.md line {}: ```wgsl fence is never closed.",
                    j + 1
                );
            }
            let body = lines[body_start..k].join("\n");
            blocks.push(CommonBlock {
                applies_to: applies,
                body,
            });
            i = k + 1;
        } else {
            i += 1;
        }
    }
    blocks
}

fn parse_applies_marker(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    let prefix = "<!-- shader-common: applies-to=";
    let suffix = "-->";
    let inner = trimmed.strip_prefix(prefix)?.strip_suffix(suffix)?.trim();
    let names: Vec<String> = inner.split(',').map(|s| s.trim().to_string()).collect();
    if names.iter().any(|n| n.is_empty()) {
        panic!("shader-common marker has empty target name: {:?}", trimmed);
    }
    Some(names)
}

fn is_wgsl_fence_open(line: &str) -> bool {
    let t = line.trim_start();
    t == "```wgsl" || t.starts_with("```wgsl ")
}

fn is_fence_close(line: &str) -> bool {
    line.trim_start() == "```"
}

fn wgsl_function(source: &str, name: &str) -> String {
    let needle = format!("fn {name}(");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("missing WGSL function `{name}`"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("WGSL function `{name}` has no body"));
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return source[start..=open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("WGSL function `{name}` has an unclosed body")
}

/// Fragments that some module still carries as a pasted copy, until that
/// module is assembled from the fragment too: (fragment, modules).
const INLINE_COPIES: &[(&str, &[&str])] = &[];

/// Every block SHADER_COMMON.md still fences is pasted verbatim into each
/// module its `applies-to` names, and every fragment a module still carries
/// as a copy ([`INLINE_COPIES`]) is pasted there verbatim.
#[test]
fn shader_common_blocks_match_ssot() {
    let md = read(SSOT_PATH);
    let blocks = parse_ssot_blocks(&md);
    let mut failures: Vec<String> = Vec::new();
    for block in &blocks {
        for short in &block.applies_to {
            let path = shader_path_for(short);
            if !read(path).contains(&block.body) {
                failures.push(format!(
                    "{path} does not carry this SHADER_COMMON.md block verbatim:\n{}",
                    block.body
                ));
            }
        }
    }
    for (fragment, modules) in INLINE_COPIES {
        let text = read(fragment);
        for path in *modules {
            if !read(path).contains(&text) {
                failures.push(format!("{path} does not carry {fragment} verbatim"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "\nSHADER_COMMON.md SSoT check failed:\n{}",
        failures.join("\n")
    );
}

#[test]
fn field_fit_uses_the_ssot_grid_pair_normalization() {
    let ssot = read(SSOT_PATH);
    let fit = read("src/gpu_errorbar.wgsl");
    for function in [
        "grid_rounded_add",
        "grid_rounded_subtract",
        "normalize_grid_pair",
        "add_grid_pairs",
        "subtract_grid_pairs",
        "scale_grid_pair",
        "midpoint_grid_pair",
    ] {
        assert_eq!(
            wgsl_function(&fit, function),
            wgsl_function(&ssot, function),
            "field fit and rendered field bounds must share `{function}` byte-for-byte"
        );
    }
    assert!(fit.contains("struct FieldBound {\n    pair: vec2<f32>,\n    valid: bool,"));
    assert!(fit.contains("if (!x_lo.valid || !x_hi.valid || !y_lo.valid || !y_hi.valid)"));
    assert!(
        !fit.contains("0x7fc00000u"),
        "field fit must carry invalidity explicitly instead of relying on a NaN sentinel"
    );
}

/// The colour ramp and the size-map radius are function twins (SHADER_COMMON.md
/// §5.4): one parameterised function set copied into every shader that maps a
/// value through the colourbar or a size domain, so a fix in one cannot leave
/// the others behind.
#[test]
fn colour_ramp_and_size_map_radius_twins_match_the_ssot() {
    let ssot = read(SSOT_PATH);
    let ramp = ["ramp_finite", "ramp_t", "ramp_segment", "ramp_blend"];
    let radius = ["ramp_finite", "size_map_radius"];
    for (path, functions) in [
        ("src/data_render/field_columnar.wgsl", ramp.as_slice()),
        ("src/data_render/scatter_columnar.wgsl", ramp.as_slice()),
        ("src/data_render/scatter_columnar.wgsl", radius.as_slice()),
        (
            "src/data_render/vector/vector_input_common.wgsl",
            ramp.as_slice(),
        ),
        (PICK_FRAGMENTS, radius.as_slice()),
    ] {
        let shader = read(path);
        for function in functions {
            assert_eq!(
                wgsl_function(&shader, function),
                wgsl_function(&ssot, function),
                "{path} must carry `{function}` byte-for-byte as SHADER_COMMON.md §5.4 defines it"
            );
        }
    }
}

/// One `const NAME: TYPE = VALUE;` line of a WGSL source.
fn wgsl_const(source: &str, name: &str) -> String {
    let needle = format!("const {name}:");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("missing WGSL constant `{name}`"));
    let end = source[start..]
        .find(';')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("WGSL constant `{name}` has no terminator"));
    source[start..=end].to_string()
}

/// The marker shape is a function twin (SHADER_COMMON.md §5.5): the scatter
/// shader draws with it and the pick shader picks with it, so a marker picks
/// exactly where it is drawn.
#[test]
fn marker_shape_twins_match_the_ssot() {
    let ssot = read(SSOT_PATH);
    let functions = [
        "shape_is_filled",
        "base_shape_id",
        "visual_shape_radius",
        "rotate2",
        "sd_segment",
        "sd_box",
        "regular_vertex",
        "star_vertex",
        "sd_regular_polygon",
        "sd_star",
        "sd_plus_filled",
        "sd_triangle",
        "shape_distance",
        "marker_edge",
    ];
    let constants = [
        "QUAD_MARGIN_PX",
        "STROKE_HALF_PX",
        "MARKER_PI",
        "MARKER_HALF_PI",
        "STAR_INNER_RATIO",
    ];
    for path in ["src/data_render/scatter_columnar.wgsl", PICK_FRAGMENTS] {
        let shader = read(path);
        for function in functions {
            assert_eq!(
                wgsl_function(&shader, function),
                wgsl_function(&ssot, function),
                "{path} must carry `{function}` byte-for-byte as SHADER_COMMON.md §5.5 defines it"
            );
        }
        for constant in constants {
            assert_eq!(
                wgsl_const(&shader, constant),
                wgsl_const(&ssot, constant),
                "{path} must carry `{constant}` as SHADER_COMMON.md §5.5 defines it"
            );
        }
    }
}

/// The line stroke is a function twin (SHADER_COMMON.md §5.6): the line
/// shader extrudes with it and the pick shader inverts the same extrusion,
/// so a stroke, its caps and its dashes pick exactly where they are drawn.
#[test]
fn line_stroke_twins_match_the_ssot() {
    let ssot = read(SSOT_PATH);
    let all = [
        "line_stroke_distance",
        "dash_pattern_on",
        "sketch_hash01",
        "sketch_noise",
        "sketch_line_offset",
    ];
    let noise = ["sketch_hash01", "sketch_noise"];
    for (path, functions) in [
        ("src/data_render/line_columnar.wgsl", all.as_slice()),
        (PICK_FRAGMENTS, all.as_slice()),
        ("src/data_render/scatter_columnar.wgsl", noise.as_slice()),
        ("src/data_render/errorbar_columnar.wgsl", noise.as_slice()),
    ] {
        let shader = read(path);
        for function in functions {
            assert_eq!(
                wgsl_function(&shader, function),
                wgsl_function(&ssot, function),
                "{path} must carry `{function}` byte-for-byte as SHADER_COMMON.md §5.6 defines it"
            );
        }
    }
    for path in ["src/data_render/line_columnar.wgsl", PICK_FRAGMENTS] {
        let shader = read(path);
        for constant in ["LINE_AA_EXTENT_PX", "SKETCH_SUBDIV"] {
            assert_eq!(
                wgsl_const(&shader, constant),
                wgsl_const(&ssot, constant),
                "{path} must carry `{constant}` as SHADER_COMMON.md §5.6 defines it"
            );
        }
    }
}

/// The styled markers are function twins (SHADER_COMMON.md §5.7): the
/// sketch wobble, the planet's ring and rim and the constellation star's
/// radius are evaluated by the pick shader with the scatter shader's own
/// functions.
#[test]
fn styled_marker_twins_match_the_ssot() {
    let ssot = read(SSOT_PATH);
    for path in ["src/data_render/scatter_columnar.wgsl", PICK_FRAGMENTS] {
        let shader = read(path);
        for function in [
            "sketch_marker_wobble_px",
            "sketch_marker_distance",
            "constellation_star_radius",
            "cons_ring_angle",
            "planet_ring_phi",
            "planet_ring_frame",
            "planet_ring_rho",
            "planet_rim_glow",
        ] {
            assert_eq!(
                wgsl_function(&shader, function),
                wgsl_function(&ssot, function),
                "{path} must carry `{function}` byte-for-byte as SHADER_COMMON.md §5.7 defines it"
            );
        }
        for constant in [
            "SKETCH_TAU",
            "SKETCH_CONTOUR_WOBBLES",
            "CONSTELLATION_POINT_STAR_GAIN",
            "RING_INNER",
            "RING_OUTER",
            "RING_INCL",
            "PLANET_QUAD_EXTENT",
            "PLANET_ALPHA_FLOOR",
        ] {
            assert_eq!(
                wgsl_const(&shader, constant),
                wgsl_const(&ssot, constant),
                "{path} must carry `{constant}` as SHADER_COMMON.md §5.7 defines it"
            );
        }
    }
}

/// One `struct NAME { ... };` declaration of a WGSL source.
fn wgsl_struct(source: &str, name: &str) -> String {
    let needle = format!("struct {name} {{");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("missing WGSL struct `{name}`"));
    let end = source[start..]
        .find("};")
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("WGSL struct `{name}` has no terminator"));
    source[start..end + 2].to_string()
}

/// The arrow glyph is a function twin (SHADER_COMMON.md §5.8): the vector
/// shader draws a point vector's arrow with it and the pick shader picks
/// that arrow with it.
#[test]
fn arrow_glyph_twins_match_the_ssot() {
    let ssot = read(SSOT_PATH);
    let pick = read(PICK_FRAGMENTS);
    let glyph = read("src/data_render/common/arrow.wgsl");
    let projection = read("src/data_render/vector/project_cartesian2d.wgsl");
    let copies = [
        ("src/data_render/common/arrow.wgsl", &glyph, true),
        (
            "src/data_render/vector/project_cartesian2d.wgsl",
            &projection,
            false,
        ),
        (PICK_FRAGMENTS, &pick, true),
    ];
    for (path, shader, has_glyph) in copies {
        if has_glyph {
            for function in [
                "arrow_finite",
                "arrow_place",
                "arrow_sd_segment",
                "arrow_sd_triangle",
                "arrow_distance",
            ] {
                assert_eq!(
                    wgsl_function(shader, function),
                    wgsl_function(&ssot, function),
                    "{path} must carry `{function}` byte-for-byte as SHADER_COMMON.md §5.8 defines it"
                );
            }
            for structure in ["ArrowStyle", "ArrowPlacement"] {
                assert_eq!(
                    wgsl_struct(shader, structure),
                    wgsl_struct(&ssot, structure),
                    "{path} must declare `{structure}` as SHADER_COMMON.md §5.8 defines it"
                );
            }
            for constant in [
                "ARROW_PIVOT_TAIL",
                "ARROW_PIVOT_MIDDLE",
                "ARROW_PIVOT_TIP",
                "ARROW_SHAFT_MIN_PX",
            ] {
                assert_eq!(
                    wgsl_const(shader, constant),
                    wgsl_const(&ssot, constant),
                    "{path} must carry `{constant}` as SHADER_COMMON.md §5.8 defines it"
                );
            }
        }
        if path != "src/data_render/common/arrow.wgsl" {
            for function in ["pair_add", "axis_px_per_unit_at"] {
                assert_eq!(
                    wgsl_function(shader, function),
                    wgsl_function(&ssot, function),
                    "{path} must carry `{function}` byte-for-byte as SHADER_COMMON.md §5.8 defines it"
                );
            }
        }
    }
}

/// The pick shader applies the draw shaders' polar fragment clip at the
/// cursor with the common block's own `polar_fragment_visible`.
#[test]
fn pick_polar_clip_is_the_draw_fragment_clip() {
    let ssot = read(SSOT_PATH);
    let pick = read(PICK_FRAGMENTS);
    assert_eq!(
        wgsl_function(&pick, "polar_fragment_visible"),
        wgsl_function(&ssot, "polar_fragment_visible"),
        "the pick fragments must carry the common block's `polar_fragment_visible` byte-for-byte"
    );
}

#[test]
fn contour_level_search_constants_match_the_model_limit() {
    assert_eq!(renderer::MAX_CONTOUR_LEVELS, 1024);

    let rust = read("src/data_render/contour.rs");
    let shader = read("src/data_render/field_columnar.wgsl");
    assert!(rust.contains("pub(crate) const CONTOUR_LEVEL_BLOCK_SIZE: usize = 32;"));
    assert!(rust.contains("pub(crate) const CONTOUR_LEVEL_BLOCK_COUNT: usize = 32;"));
    assert!(
        rust.contains("crate::data_config::MAX_CONTOUR_LEVELS == CONTOUR_LEVEL_LOOKUP_CAPACITY")
    );
    assert!(shader.contains("const CONTOUR_LEVEL_BLOCK_SIZE: u32 = 32u;"));
    assert!(shader.contains("let original = firstTrailingBit(candidates);"));
}

#[test]
fn contour_label_capacity_and_private_storage_match_the_model_contract() {
    const KEPT_POSITION_BYTES: usize = 1024 * std::mem::size_of::<[f32; 2]>();
    const KEPT_RADIUS_BYTES: usize = 1024 * std::mem::size_of::<f32>();

    assert_eq!(renderer::MAX_CONTOUR_LEVELS, 1024);
    assert_eq!(renderer::gpu_contour::MAX_CONTOUR_LABELS_TOTAL, 1024);
    assert!(renderer::gpu_contour::MAX_ANCHOR_CANDIDATES >= renderer::MAX_CONTOUR_LEVELS as u32);
    assert_eq!(KEPT_POSITION_BYTES + KEPT_RADIUS_BYTES, 12_288);

    let rust = read("src/gpu_contour.rs");
    let shader = read("src/contour_anchor.wgsl");
    assert!(rust.contains("pub const MAX_CONTOUR_LABELS_TOTAL: u32 = 1024;"));
    assert!(
        rust.contains("MAX_ANCHOR_CANDIDATES >= crate::data_config::MAX_CONTOUR_LEVELS as u32")
    );
    assert!(shader.contains("const MAX_KEPT: u32 = 1024u;"));
    assert!(shader.contains("var<private> kept_px: array<vec2<f32>, 1024>;"));
    assert!(shader.contains("var<private> kept_r: array<f32, 1024>;"));
    assert!(shader.contains("@compute @workgroup_size(1)\nfn anchor_select()"));
    assert!(!shader.contains("var<workgroup>"));
}

#[test]
fn contour_lookup_metadata_binding_and_layout_match_cpu_and_wgsl() {
    let metadata = read("src/data_render/contour.rs");
    let bindings = read("src/data_render/field.rs");
    let field = read("src/data_render/field_columnar.wgsl");
    let anchor = read("src/contour_anchor.wgsl");

    assert!(metadata.contains("pub struct ContourLookupMetadataGpu"));
    assert!(metadata.contains("pub finite_count: u32"));
    assert!(metadata.contains("pub negative_infinity_count: u32"));
    assert!(metadata.contains("size_of::<ContourLookupMetadataGpu>() == 8"));
    assert!(bindings.contains("storage(6, wgpu::ShaderStages::FRAGMENT)"));
    assert!(bindings.contains("binding: 6,"));
    assert!(bindings.contains("resource: lookup_metadata_buf.as_entire_binding()"));

    assert!(field.contains("struct ContourLookupMetadata {\n    finite_count: u32,\n    negative_infinity_count: u32,\n};"));
    assert!(field.contains("@group(2) @binding(6) var<storage, read> contour_lookup_metadata"));
    assert!(field.contains("let original = u32(record.y) - start;"));
    assert!(!field.contains("bitcast<u32>(record.y)"));
    assert!(!anchor.contains("@group(2) @binding(6)"));
}

#[test]
fn field_invalid_values_use_explicit_validity_without_nan_sentinels() {
    let field = read("src/data_render/field_columnar.wgsl");
    let anchor = read("src/contour_anchor.wgsl");

    for (name, shader) in [("field", field.as_str()), ("anchor", anchor.as_str())] {
        assert!(shader.contains("struct GridValue {\n    pair: vec2<f32>,\n    valid: bool,\n};"));
        assert!(shader.contains("if (!p00.valid || !p10.valid || !p01.valid || !p11.valid)"));
        assert!(shader.contains("fn vec2_f32_is_finite(v: vec2<f32>) -> bool"));
        assert!(shader.contains("!vec2_f32_is_finite(p00.pair)"));
        assert!(
            !shader.contains("bitcast<f32>(0x7fc00000u)"),
            "{name} shader must not construct a constant NaN sentinel"
        );
    }

    assert!(field.contains("if (!sample.valid)"));
    assert!(anchor.contains("struct LevelValue {\n    value: f32,\n    valid: bool,\n};"));
    assert!(anchor.contains("if (!wanted.valid || !f32_is_finite(wanted.value))"));
}

#[test]
fn field_derived_nonfinite_arithmetic_fails_closed() {
    let field = read("src/data_render/field_columnar.wgsl");
    let anchor = read("src/contour_anchor.wgsl");

    for (name, shader) in [("field", field.as_str()), ("anchor", anchor.as_str())] {
        let locate = shader
            .split_once("fn locate(base: u32")
            .unwrap_or_else(|| panic!("{name} shader has no locate"))
            .1
            .split_once("struct GridValue")
            .unwrap_or_else(|| panic!("{name} shader has no GridValue after locate"))
            .0;
        for required in [
            "!f32_is_finite(t)",
            "!f32_is_finite(first)",
            "!f32_is_finite(last)",
            "!f32_is_finite(tm)",
            "!f32_is_finite(a)",
            "!f32_is_finite(b)",
            "!f32_is_finite(span)",
            "!f32_is_finite(frac)",
        ] {
            assert!(
                locate.contains(required),
                "{name} locate must fail closed on {required}"
            );
        }
        assert!(
            locate.find("!f32_is_finite(frac)").unwrap() < locate.find("out.hit = true;").unwrap(),
            "{name} locate must validate every derived boundary value before hit=true"
        );

        let sample = shader
            .split_once("fn contour_sample(t: vec2<f32>) -> ContourSample {")
            .unwrap_or_else(|| panic!("{name} shader has no contour_sample"))
            .1
            .split_once("fn grad_px")
            .unwrap_or_else(|| panic!("{name} shader has no grad_px after contour_sample"))
            .0;
        assert!(sample.contains("!f32_is_finite(z00)"));
        assert!(sample.contains("!f32_is_finite(du0)"));
        assert!(sample.contains("!f32_is_finite(z_delta)"));
        assert!(sample.contains("!f32_is_finite(dz_du)"));
        assert!(sample.contains("!f32_is_finite(span_product)"));
        assert!(sample.contains("!vec2_f32_is_finite(dz) || !f32_is_finite(d2)"));
        assert!(
            sample
                .find("!vec2_f32_is_finite(dz) || !f32_is_finite(d2)")
                .unwrap()
                < sample.find("out.hit = true;").unwrap(),
            "{name} must validate interpolation, gradient, and Hessian before hit=true"
        );
    }

    // The ramp's own checks live in the shared `ramp_t` (SHADER_COMMON.md §5.4).
    assert!(field.contains("if (!ramp_finite(raw))"));
    assert!(field.contains("!ramp_finite(normalized)"));
    assert!(field.contains("!vec2_f32_is_finite(low) || !vec2_f32_is_finite(high)"));
    assert!(field.contains("!f32_is_finite(coverage_unclamped)"));
    assert!(field.contains("if (!f32_is_finite(cov) || cov <= 0.0)"));
    assert!(field.contains("!vec4_f32_is_finite(c) || !vec4_f32_is_finite(composited)"));

    assert!(anchor.contains("if (!f32_is_finite(g2) || g2 <= 0.0"));
    assert!(anchor.contains("if (!vec2_f32_is_finite(correction))"));
    assert!(anchor.contains("if (!vec2_f32_is_finite(p))"));
}

/// Browser-WGSL portability lint: bare `textureSample` is FORBIDDEN in every
/// figgy shader — use `textureSampleLevel(..., 0.0)`.
///
/// Implicit-derivative sampling inside non-uniform control flow is a hard
/// COMPILE error in the browser's WGSL compiler (Tint), while native naga
/// accepts it — the v0.4.0 planet shader shipped exactly that, and on wasm
/// the invalid module took down every pipeline in the file: any chart with a
/// scatter primitive rendered a black canvas in all three draw styles.
/// figgy's textures are all single-mip, so explicit LOD 0 is always
/// pixel-identical and there is no legitimate use of the implicit form.
#[test]
fn no_bare_texture_sample_in_any_shader() {
    let dir = workspace_root().join("src/data_render");
    let mut offenders = Vec::new();
    for entry in fs::read_dir(&dir).expect("read data_render dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("wgsl") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read shader");
        for (i, line) in src.lines().enumerate() {
            // `textureSampleLevel` must not match; the lint targets the
            // implicit-derivative form only.
            if line.contains("textureSample(") {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "bare textureSample() found — use textureSampleLevel(..., 0.0) \
         (Tint rejects implicit derivatives in non-uniform control flow; \
         on wasm the whole shader module fails and scatter charts go black):\n{}",
        offenders.join("\n")
    );
}

// The step-interpolation helpers are deliberately outside the common block
// (`line_arc.wgsl` has no `Style`), but the line entries and the arc scan must
// split a segment into exactly the same pieces or the dash phase drifts from
// the drawn corners. The GPU picker carries its own `pick_` copy of the same
// rules; its agreement with the CPU picker is a GPU parity test instead.
#[test]
fn step_interpolation_helpers_match_between_the_line_and_arc_shaders() {
    let line = read(shader_path_for("line"));
    let arc = read(shader_path_for("arc"));
    for name in ["step_piece_count", "step_piece", "step_piece_len_px"] {
        assert_eq!(
            wgsl_function(&line, name),
            wgsl_function(&arc, name),
            "`{name}` differs between line_columnar.wgsl and line_arc.wgsl"
        );
    }
    for constant in [
        "const LINE_STEP_SHIFT: u32 = 2u;",
        "const LINE_STEP_MASK: u32 = 3u;",
        "const LINE_STEP_LINEAR: u32 = 0u;",
        "const LINE_STEP_PRE: u32 = 1u;",
        "const LINE_STEP_POST: u32 = 2u;",
        "const LINE_STEP_MID: u32 = 3u;",
    ] {
        assert!(line.contains(constant), "line_columnar.wgsl: {constant}");
        assert!(arc.contains(constant), "line_arc.wgsl: {constant}");
    }
    // The picker's copy keeps the data-space midpoint rule under its own
    // names; the rest of its agreement is the CPU/GPU pick parity test.
    let pick = read(PICK_FRAGMENTS);
    assert!(pick.contains("fn pick_step_piece("));
    assert!(pick.contains("let xm = (ax + bx) * 0.5;"));
    assert!(line.contains("let xm = (xa + xb) * 0.5;"));
}

// A log flag returns the logarithmic value before any `mix`: drivers that
// evaluate `mix(x, y, 1)` as `x + (y - x)` lose `y` under a large `x`,
// which put log-axis values past 2^24 at the wrong height. The linear
// arithmetic stays the original `mix`, which DX12 needs bit for bit at large
// offsets (SHADER_COMMON §3). llvmpipe evaluates `mix` exactly, so no pixel
// test here would see the difference.
#[test]
fn log_flags_return_the_logarithm_before_mixing() {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "wgsl") {
                out.push(path);
            }
        }
    }
    let mut shaders = Vec::new();
    walk(&workspace_root().join("src"), &mut shaders);
    assert!(shaders.len() > 10, "found {} shaders", shaders.len());
    let mut offending = Vec::new();
    let mut guarded = 0;
    for path in shaders {
        let source = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (number, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap();
            let Some(flag) = ["is_log", "scale_log"]
                .into_iter()
                .find(|f| code.contains(f))
            else {
                continue;
            };
            let chosen = (code.contains("mix(") || code.contains("select("))
                && !code.contains("select(0.0, 1.0,");
            if !chosen {
                continue;
            }
            // The same function returns the logarithm before the `mix`, or
            // overrides the `mix` with it right after.
            let guard = |l: &&str| l.contains("if ") && l.contains(flag) && l.contains("> 0.5");
            let early = lines[..number]
                .iter()
                .rev()
                .take_while(|l| !l.trim_start().starts_with("fn "))
                .any(guard);
            let overridden = lines[number + 1..].iter().take(2).any(guard);
            if code.contains("select(") || !(early || overridden) {
                offending.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
            guarded += 1;
        }
    }
    assert!(
        offending.is_empty(),
        "log flags chosen unsafely:\n{}",
        offending.join("\n")
    );
    // Each shared fragment is scanned once, so the floor counts the
    // choices of `maybe_log`, `axis_pair_to_t`, `ramp_t` and
    // `axis_px_per_unit_at`: the scan must keep finding them.
    assert!(guarded >= 4, "{guarded} guarded log choices found");
}

/// Rule from #46: one behaviour has one implementation. Every WGSL function
/// defined in more than one product shader under `src/` is listed below, and
/// the test fails on any change to that picture: a new copy (a name defined
/// in two files that is not listed, or listed with a different file set) or
/// a stale entry (a listed copy that no longer exists). The list therefore
/// only shrinks: each merge of item 11 in #58 removes its entries, until only
/// `ASSEMBLED_ALTERNATIVES` is left. Entry points (`@vertex`, `@fragment`,
/// `@compute`) are exempt: their names are per pipeline, not shared code.
///
/// `ASSEMBLED_ALTERNATIVES` are interface functions with one body per
/// variant, of which the Rust assembler splices exactly one into a module.
/// They are the layered design of rule 9 and stay. `PENDING_COPIES` are the
/// pasted copies, kept in sync by the tests above until the assembler
/// fragments replace them. Each entry is (files that define them, names).
#[test]
fn every_wgsl_function_is_defined_once_outside_the_allowlist() {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "wgsl") {
                out.push(path);
            }
        }
    }
    let src = workspace_root().join("src");
    let mut shaders = Vec::new();
    walk(&src, &mut shaders);
    shaders.sort();
    assert!(shaders.len() > 10, "found {} shaders", shaders.len());

    let mut defined: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in &shaders {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let text = fs::read_to_string(path).unwrap();
        let mut attributes = String::new();
        for line in text.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let entry_point = ["@vertex", "@fragment", "@compute"]
                    .iter()
                    .any(|attr| attributes.contains(attr) || trimmed.contains(attr));
                if !name.is_empty() && !entry_point {
                    defined.entry(name).or_default().push(rel.clone());
                }
            }
            if !trimmed.is_empty() {
                attributes = trimmed.to_string();
            }
        }
    }
    let actual: BTreeMap<&str, Vec<&str>> = defined
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(name, files)| (name.as_str(), files.iter().map(String::as_str).collect()))
        .collect();

    let mut expected: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (files, names) in ASSEMBLED_ALTERNATIVES.iter().chain(PENDING_COPIES) {
        let mut files: Vec<&str> = files.to_vec();
        files.sort_unstable();
        for name in *names {
            assert!(
                expected.insert(name, files.clone()).is_none(),
                "`{name}` is listed twice in the duplicate allowlist"
            );
        }
    }

    let mut problems = Vec::new();
    for (name, files) in &actual {
        match expected.get(name) {
            Some(listed) if listed == files => {}
            Some(listed) => problems.push(format!(
                "`{name}` is defined in {files:?} but the allowlist says {listed:?}: a copy moved or a new one appeared"
            )),
            None => problems.push(format!(
                "`{name}` is defined in {files:?}: a new copy of a shader function. Call the existing one or splice it in as an assembler fragment (#46)"
            )),
        }
    }
    for (name, listed) in &expected {
        if !actual.contains_key(name) {
            problems.push(format!(
                "`{name}` is no longer defined in more than one of {listed:?}: remove its allowlist entry"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "WGSL duplicate definitions changed:\n  {}",
        problems.join("\n  ")
    );
}

/// Interface functions with one body per variant; the assembler splices one.
/// Part of the layered design (rule 9); these stay. Also the per-module
/// bodies of one contract, where each module needs its own.
const ASSEMBLED_ALTERNATIVES: &[(&[&str], &[&str])] = &[
    (
        &[
            "gpu_pick/marker.wgsl",
            "gpu_pick/no_scatter.wgsl",
            "gpu_pick/vector.wgsl",
        ],
        &["pick_scatter_candidate"],
    ),
    (
        &[
            "data_render/vector/vector_input_grid_a.wgsl",
            "data_render/vector/vector_input_points.wgsl",
        ],
        &["vector_sample"],
    ),
    (
        &["gpu_pick/line.wgsl", "gpu_pick/no_line.wgsl"],
        &["pick_line_candidate"],
    ),
    (
        &[
            "stream_sample_transfer.wgsl",
            "stream_sample_transfer_msaa.wgsl",
        ],
        &["preview_texel"],
    ),
    // Not a paste: each workgroup entry zeroes its own `var<workgroup>` array
    // before computing, so the four bodies differ by design (the contract is
    // in `workgroup_init.rs`, the per-shader zeroing in each module).
    (
        &[
            "data_render/bar_columnar.wgsl",
            "data_render/line_arc.wgsl",
            "gpu_errorbar_a.wgsl",
            "gpu_pick/core.wgsl",
        ],
        &["figgy_zero_workgroup"],
    ),
];

/// Pasted copies that item 11 of #58 replaces with assembler fragments.
/// Shrinks with every merge; never grows.
const PENDING_COPIES: &[(&[&str], &[&str])] = &[
    (
        &["boxplot.wgsl", "categorical.wgsl", "radial.wgsl"],
        &["hash", "noise"],
    ),
    (
        &["boxplot.wgsl", "categorical.wgsl", "radar.wgsl"],
        &["output"],
    ),
    (
        &[
            "data_render/common/sample_validity.wgsl",
            "gpu_pick/core.wgsl",
        ],
        &["polar_fragment_visible"],
    ),
    (
        &["data_render/common/ramp.wgsl", "gpu_pick/marker.wgsl"],
        &["ramp_finite"],
    ),
    (&["boxplot.wgsl", "categorical.wgsl"], &["sdf", "shade"]),
];

/// Rule from #46 (the hand-off convention): code that builds one struct from
/// another reads the source through a pattern that names every field, with
/// the fields it leaves out written as `name: _`, so a new field is a compile
/// error at every reader and an omission is a line in the diff. A `..` rest
/// pattern, `..Default::default()` or `..template` on a model type defeats
/// that: the new field is silently ignored or silently defaulted.
///
/// This walks product Rust code (`crates/model/src`, `crates/renderer/src`,
/// `crates/web/src`), skipping `tests.rs` / `*_tests.rs` files, items under
/// `#[cfg(test)]` or `#[cfg(all(test, ..))]` and `mod tests` / `mod *_tests`
/// blocks, and fails on any struct pattern or literal of a `REST_GUARDED_TYPES`
/// type (or `Self` inside its `impl`) whose fields include a `..` item. The
/// sites not yet converted are `REST_PATTERN_ALLOWLIST`, as (file, type,
/// count, reason); the test fails on any change to that picture, so the list
/// only shrinks: a converted site removes its entry, a new site fails.
#[test]
fn model_struct_hand_offs_name_every_field_outside_the_allowlist() {
    let scanner_self_check = r##"
        // Config { a, ..Default::default() } in a comment is not code.
        /* nested /* Config { .. } */ still a comment */
        const DOC: &str = "Config { a, ..b }";
        const RAW: &str = r#"Config { .. }"#;
        const BRACE: char = '{';
        pub struct Config { a: u8 }
        impl Config { fn f(&self) -> Config { make() } }
        fn hit(c: &Config) { let Config { a, .. } = c; }
        fn range(c: &Config) -> Config { Config { a: 0..4, b } }
        fn update(b: Config) -> Config { crate::config::Config { a: 1, ..b } }
        fn variant(x: Warning) { if let Warning::Config { .. } = x {} }
        impl Default for Config { fn default() -> Self { Self { a: 0, ..zero() } } }
        #[cfg(test)]
        mod hidden { fn f(c: Config) { Config { a: 1, ..c }; } }
        #[cfg(all(test, feature = "serde"))]
        fn hidden_too(c: Config) -> Config { Config { a: 1, ..c } }
        mod tests { fn f(c: Config) { Config { a: 1, ..c }; } }
    "##;
    let found = rest_pattern_sites(scanner_self_check, &["Config"]);
    let lines: Vec<usize> = found.iter().map(|(_, line)| *line).collect();
    assert_eq!(
        lines,
        vec![9, 11, 13],
        "the rest-pattern scanner misreads its own fixture: {found:?}"
    );

    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let trees = product_source_trees();
    let mut actual: BTreeMap<(String, &str), Vec<usize>> = BTreeMap::new();
    let mut scanned = 0;
    for (crate_name, src) in &trees {
        let mut files = Vec::new();
        walk(src, &mut files);
        files.sort();
        for path in &files {
            let name = path.file_name().unwrap().to_str().unwrap();
            if name == "tests.rs" || name.ends_with("_tests.rs") {
                continue;
            }
            scanned += 1;
            let rel = path
                .strip_prefix(src)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let rel = format!("{crate_name}/src/{rel}");
            let source = fs::read_to_string(path).unwrap();
            for (ty, line) in rest_pattern_sites(&source, REST_GUARDED_TYPES) {
                actual.entry((rel.clone(), ty)).or_default().push(line);
            }
        }
    }
    assert!(scanned > 100, "scanned only {scanned} product files");
    let scanned_crate = |file: &str| {
        trees
            .iter()
            .any(|(crate_name, _)| file.starts_with(&format!("{crate_name}/")))
    };

    let mut expected: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for (file, ty, count, _reason) in REST_PATTERN_ALLOWLIST {
        assert!(
            REST_GUARDED_TYPES.contains(ty),
            "allowlist entry ({file}, {ty}) names a type the guard does not check"
        );
        assert!(
            expected.insert((file.to_string(), ty), *count).is_none(),
            "({file}, {ty}) is listed twice in the rest-pattern allowlist"
        );
    }

    let mut problems = Vec::new();
    for ((file, ty), lines) in &actual {
        match expected.get(&(file.clone(), *ty)) {
            Some(count) if *count == lines.len() => {}
            Some(count) if *count > lines.len() => problems.push(format!(
                "{file}: {} `{ty} {{ .. }}` site(s) left (lines {lines:?}), the allowlist says {count}: lower its count",
                lines.len()
            )),
            Some(count) => problems.push(format!(
                "{file}: {} `{ty} {{ .. }}` site(s) (lines {lines:?}), the allowlist says {count}: a new rest pattern. Name every field, with the ones left out as `name: _` (#46)",
                lines.len()
            )),
            None => problems.push(format!(
                "{file}: `{ty} {{ .. }}` at lines {lines:?}: a rest pattern on a model type. Name every field, with the ones left out as `name: _` (#46)"
            )),
        }
    }
    for ((file, ty), count) in &expected {
        if scanned_crate(file) && !actual.contains_key(&(file.clone(), *ty)) {
            problems.push(format!(
                "{file}: no `{ty} {{ .. }}` site is left of the {count} listed: remove its allowlist entry"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "rest patterns on model types changed:\n  {}",
        problems.join("\n  ")
    );
}

/// The product source trees the rest-pattern guard reads, as (crate, `src`
/// directory). In the workspace these are `crates/{model,renderer,web}/src`,
/// all required. `ci/verify.py packages` runs the tests from the published
/// archives instead, extracted side by side as `figgy-model-<version>` and
/// `figgy-renderer-<version>`, without the unpublished web crate; the guard
/// then reads those two. A renderer package on its own reads its own source.
fn product_source_trees() -> Vec<(&'static str, PathBuf)> {
    let renderer = workspace_root();
    let parent = renderer.join("..");
    let mut trees = vec![("renderer", renderer.join("src"))];
    if renderer.file_name().is_some_and(|name| name == "renderer") {
        for crate_name in ["model", "web"] {
            let src = parent.join(crate_name).join("src");
            assert!(
                src.is_dir(),
                "workspace crate source {} is missing",
                src.display()
            );
            trees.push((crate_name, src));
        }
        return trees;
    }
    let model = fs::read_dir(&parent).ok().and_then(|entries| {
        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("figgy-model-"))
                    && path.join("src").is_dir()
            })
    });
    match model {
        Some(model) => trees.push(("model", model.join("src"))),
        None => eprintln!(
            "rest-pattern guard: no figgy-model source beside {}; checking the renderer only",
            renderer.display()
        ),
    }
    trees
}

/// Model types whose struct patterns and literals must name every field.
/// `LegendOptions` and `SubplotLayer` of the #46 list do not exist; their
/// types are `Legend` and `Subplot`. `AxisTitleOptions`, `PolarOptions`,
/// `PickedPointsConfig`, `DataSelectionsConfig`, `DataScatterStyleConfig` and
/// `DataErrorBarPointStyleConfig` are read by converted hand-offs too.
const REST_GUARDED_TYPES: &[&str] = &[
    "Config",
    "AxisOptions",
    "AxisTitleOptions",
    "SeriesConfig",
    "ColorBarOptions",
    "ChartTitleOptions",
    "Legend",
    "GridOptions",
    "LabelStyle",
    "PolarOptions",
    "PickedPointsConfig",
    "DataSelectionsConfig",
    "MilkywayOptions",
    "SketchOptions",
    "ConstellationOptions",
    "DrawStyle",
    "VectorStyle",
    "PointSizeMap",
    "PointColorMap",
    "DataScatterStyleConfig",
    "DataScatterPointStyleConfig",
    "DataLineStyleConfig",
    "DataErrorBarStyleConfig",
    "DataErrorBarPointStyleConfig",
    "DataBarStyleConfig",
    "DataBarBinStyleConfig",
    "ContourLabelConfig",
    "ChartRevisions",
    "OverlayLayer",
    "Subplot",
    "AnnotationLayer",
    "RadarChart",
    "RadarStyle",
    "RadialChart",
    "RadialStyle",
    "CategoricalChart",
    "BoxPlotChart",
];

/// Rest patterns on guarded types that are not converted yet: (file, type,
/// count, reason). Empty since every site was converted; a new site fails
/// and is converted, not listed, unless its file is frozen (streaming) and
/// the entry says so.
const REST_PATTERN_ALLOWLIST: &[(&str, &str, usize, &str)] = &[];

/// The `(type, line)` of every struct pattern or literal of one of `types`
/// in `source` (or `Self` inside an `impl` of one of them) whose fields
/// include a `..` item. Comments, string and char literals and test-only
/// items are blanked first, so only product code is read.
fn rest_pattern_sites<'t>(source: &str, types: &[&'t str]) -> Vec<(&'t str, usize)> {
    let code = blank_test_items(&blank_comments_and_literals(source));
    let chars: Vec<char> = code.chars().collect();
    let impls = impl_blocks(&chars, types);
    let line_of = |index: usize| chars[..index].iter().filter(|c| **c == '\n').count() + 1;
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut sites = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !is_ident(chars[index]) || (index > 0 && is_ident(chars[index - 1])) {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && is_ident(chars[index]) {
            index += 1;
        }
        let word: String = chars[start..index].iter().collect();
        let mut brace = index;
        while brace < chars.len() && chars[brace].is_whitespace() {
            brace += 1;
        }
        if brace >= chars.len() || chars[brace] != '{' {
            continue;
        }
        let ty = if word == "Self" {
            // The innermost `impl` block around this `Self`.
            match impls
                .iter()
                .filter(|(open, close, _)| *open < start && start < *close)
                .max_by_key(|(open, _, _)| *open)
            {
                Some((_, _, Some(ty))) => *ty,
                _ => continue,
            }
        } else {
            match types.iter().find(|ty| **ty == word) {
                Some(ty) => *ty,
                None => continue,
            }
        };
        // What precedes the name decides between an item body and a struct
        // expression or pattern; a few tokens are enough.
        let before: String = chars[start.saturating_sub(120)..start].iter().collect();
        if !is_struct_expression_or_pattern(&before) {
            continue;
        }
        let end = matching_close(&chars, brace);
        if top_level_items(&chars[brace + 1..end])
            .iter()
            .any(|item| item.trim_start().starts_with(".."))
        {
            sites.push((ty, line_of(start)));
        }
    }
    sites
}

/// Whether a type name followed by `{` starts a struct expression or pattern
/// rather than an item body: not after `struct` / `enum` / `union` / `trait`
/// / `impl ... for` / `dyn` or a return type, and not an enum variant
/// (`Warning::Subplot { .. }`, a path segment starting upper-case).
fn is_struct_expression_or_pattern(before: &str) -> bool {
    let mut rest = before.trim_end();
    if let Some(path) = rest.strip_suffix("::") {
        let segment: String = path
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return !segment.chars().last().is_some_and(char::is_uppercase);
    }
    // A return type may be a reference: `-> &'a mut Config {`.
    loop {
        let trimmed = rest.trim_end();
        let stripped = trimmed
            .strip_suffix("mut")
            .filter(|r| r.ends_with(|c: char| c.is_whitespace() || c == '&'))
            .or_else(|| trimmed.strip_suffix('&'))
            .or_else(|| {
                let lifetime = trimmed.rfind('\'')?;
                trimmed[lifetime + 1..]
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_')
                    .then(|| &trimmed[..lifetime])
            });
        match stripped {
            Some(shorter) if shorter.len() < trimmed.len() => rest = shorter,
            _ => break,
        }
    }
    let rest = rest.trim_end();
    if rest.ends_with("->") {
        return false;
    }
    let previous: String = rest
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    !matches!(
        previous.as_str(),
        "struct" | "enum" | "union" | "trait" | "impl" | "for" | "dyn"
    )
}

/// Every `impl` block in `chars` as (open brace, close brace, the guarded
/// type it implements, if it is one of `types`).
fn impl_blocks<'t>(chars: &[char], types: &[&'t str]) -> Vec<(usize, usize, Option<&'t str>)> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut blocks = Vec::new();
    let mut index = 0;
    while index + 4 <= chars.len() {
        let keyword = chars[index..index + 4] == ['i', 'm', 'p', 'l']
            && (index == 0 || !is_ident(chars[index - 1]))
            && chars.get(index + 4).is_none_or(|c| !is_ident(*c));
        if !keyword {
            index += 1;
            continue;
        }
        let Some(open) = (index..chars.len()).find(|&i| chars[i] == '{' || chars[i] == ';') else {
            break;
        };
        if chars[open] == ';' {
            index = open;
            continue;
        }
        let header: String = chars[index + 4..open].iter().collect();
        let header = header.split(" where ").next().unwrap_or(&header);
        let name: String = header
            .trim_end_matches(|c: char| c == '>' || c.is_whitespace())
            .chars()
            .rev()
            .take_while(|c| is_ident(*c))
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let ty = types.iter().find(|ty| **ty == name).copied();
        blocks.push((open, matching_close(chars, open), ty));
        index = open;
    }
    blocks
}

/// The index of the bracket closing the one at `open`, or the end.
fn matching_close(chars: &[char], open: usize) -> usize {
    let mut depth = 0usize;
    for (index, c) in chars.iter().enumerate().skip(open) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    chars.len()
}

/// `body` split at its top-level commas.
fn top_level_items(body: &[char]) -> Vec<String> {
    let mut items = vec![String::new()];
    let mut depth = 0usize;
    for c in body {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                items.push(String::new());
                continue;
            }
            _ => {}
        }
        items.last_mut().unwrap().push(*c);
    }
    items
}

/// `source` with comments, string literals and char literals replaced by
/// spaces; line breaks are kept so line numbers stay.
fn blank_comments_and_literals(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = chars.clone();
    let blank = |out: &mut Vec<char>, from: usize, to: usize| {
        for c in &mut out[from..to.min(chars.len())] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let starts_with = |at: usize, pattern: &str| {
        pattern
            .chars()
            .enumerate()
            .all(|(offset, p)| chars.get(at + offset) == Some(&p))
    };
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let ident_before =
            index > 0 && (chars[index - 1].is_alphanumeric() || chars[index - 1] == '_');
        if starts_with(index, "//") {
            let end = (index..chars.len())
                .find(|&i| chars[i] == '\n')
                .unwrap_or(chars.len());
            blank(&mut out, index, end);
            index = end;
        } else if starts_with(index, "/*") {
            let mut depth = 0;
            let mut end = index;
            while end < chars.len() {
                if starts_with(end, "/*") {
                    depth += 1;
                    end += 2;
                } else if starts_with(end, "*/") {
                    depth -= 1;
                    end += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    end += 1;
                }
            }
            blank(&mut out, index, end);
            index = end;
        } else if (c == 'r' || (c == 'b' && chars.get(index + 1) == Some(&'r'))) && !ident_before {
            let mut at = index + if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while chars.get(at) == Some(&'#') {
                hashes += 1;
                at += 1;
            }
            if chars.get(at) != Some(&'"') {
                index += 1;
                continue;
            }
            let closing: String = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            let mut end = at + 1;
            while end < chars.len() && !starts_with(end, &closing) {
                end += 1;
            }
            let end = end + closing.chars().count();
            blank(&mut out, index, end);
            index = end;
        } else if c == '"' {
            let mut end = index + 1;
            while end < chars.len() && chars[end] != '"' {
                end += if chars[end] == '\\' { 2 } else { 1 };
            }
            blank(&mut out, index, end + 1);
            index = end + 1;
        } else if c == '\'' {
            // A char literal, not a lifetime: `'x'` or an escape `'\n'`.
            let end = if chars.get(index + 1) == Some(&'\\') {
                (index + 2..chars.len()).find(|&i| chars[i] == '\'')
            } else if chars.get(index + 2) == Some(&'\'') {
                Some(index + 2)
            } else {
                None
            };
            match end {
                Some(end) => {
                    blank(&mut out, index, end + 1);
                    index = end + 1;
                }
                None => index += 1,
            }
        } else {
            index += 1;
        }
    }
    out.into_iter().collect()
}

/// `code` (already without comments and literals) with every item under
/// `#[cfg(test)]` or `#[cfg(all(test, ..))]` and every `mod tests` /
/// `mod *_tests` block blanked.
fn blank_test_items(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mut out = chars.clone();
    let text: String = chars.iter().collect();
    // Character offsets of the starts of test-only items.
    let mut starts = Vec::new();
    for marker in ["#[cfg(test)]", "#[cfg(all(test,"] {
        let mut search = 0;
        while let Some(offset) = text[search..].find(marker) {
            let at = search + offset;
            starts.push(text[..at].chars().count());
            search = at + marker.len();
        }
    }
    let mut search = 0;
    while let Some(offset) = text[search..].find("mod ") {
        let at = search + offset;
        search = at + 4;
        if at > 0 && text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let name: String = text[at + 4..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name == "tests" || name.ends_with("_tests") {
            starts.push(text[..at].chars().count());
        }
    }
    for start in starts {
        let Some(open) = (start..chars.len()).find(|&i| chars[i] == '{' || chars[i] == ';') else {
            continue;
        };
        let end = if chars[open] == '{' {
            matching_close(&chars, open)
        } else {
            open
        };
        for c in &mut out[start..(end + 1).min(chars.len())] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    }
    out.into_iter().collect()
}
