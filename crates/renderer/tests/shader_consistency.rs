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

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const SSOT_PATH: &str = "src/data_render/SHADER_COMMON.md";
const BEGIN_MARKER: &str = "// ───── BEGIN common block";
const END_MARKER: &str = "// ───── END common block";
const TARGETS: &[&str] = &[
    "scatter", "line", "errorbar", "bar", "field", "arc", "anchor", "label", "vector",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The point picker's fragments (`src/gpu_pick/*.wgsl`), concatenated in
/// file-name order: every twin a pick module carries lives in one of them.
const PICK_FRAGMENTS: &str = "src/gpu_pick/*.wgsl";

fn read(path_rel: &str) -> String {
    if let Some(dir) = path_rel.strip_suffix("/*.wgsl") {
        let mut names: Vec<_> = fs::read_dir(workspace_root().join(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "wgsl"))
            .collect();
        names.sort();
        return names
            .iter()
            .map(|path| {
                read(
                    path.strip_prefix(workspace_root())
                        .unwrap()
                        .to_str()
                        .unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            );
    }
    let p = workspace_root().join(path_rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| panic!("Failed to read {:?}: {}", p, e));
    // Normalize line endings so CRLF (Windows) vs LF doesn't trip substring
    // matching when comparing SSoT text against shader source.
    raw.replace("\r\n", "\n")
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

/// Extract the substring of `shader` strictly between the BEGIN and END common
/// block markers (exclusive of the marker lines themselves).
fn shader_common_region(shader: &str, path: &str) -> String {
    let begin = shader.find(BEGIN_MARKER).unwrap_or_else(|| {
        panic!(
            "{}: missing `{}` marker. Every duplicated shader must wrap its \
             common section with BEGIN/END markers.",
            path, BEGIN_MARKER
        )
    });
    // Advance past the marker's full line so the body starts cleanly.
    let after_begin = shader[begin..]
        .find('\n')
        .map(|n| begin + n + 1)
        .expect("BEGIN marker line must end with a newline");
    let end = shader[after_begin..]
        .find(END_MARKER)
        .map(|n| after_begin + n)
        .unwrap_or_else(|| {
            panic!(
                "{}: missing `{}` marker (or it appears before BEGIN).",
                path, END_MARKER
            )
        });
    shader[after_begin..end].to_string()
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

fn expected_region_for(blocks: &[CommonBlock], short: &str) -> String {
    let chunks: Vec<&str> = blocks
        .iter()
        .filter(|block| block.applies_to.iter().any(|target| target == short))
        .map(|block| block.body.as_str())
        .collect();
    assert!(
        !chunks.is_empty(),
        "{} contains no SSoT block for shader target `{}`",
        SSOT_PATH,
        short
    );
    chunks.join("\n\n")
}

fn trim_outer_blank_lines(s: &str) -> &str {
    s.trim_matches('\n')
}

#[test]
fn shader_common_blocks_match_ssot() {
    let md = read(SSOT_PATH);
    let blocks = parse_ssot_blocks(&md);
    assert!(
        !blocks.is_empty(),
        "{} produced 0 SSoT blocks. Did the metadata-marker syntax change?",
        SSOT_PATH
    );

    let mut shader_cache: HashMap<&str, String> = HashMap::new();
    let mut failures: Vec<String> = Vec::new();

    for short in TARGETS {
        let path = shader_path_for(short);
        let region = shader_cache.entry(path).or_insert_with(|| {
            let raw = read(path);
            shader_common_region(&raw, path)
        });
        let expected = expected_region_for(&blocks, short);
        let actual = trim_outer_blank_lines(region);

        if actual != expected {
            failures.push(format!(
                "----------------------------------------------------------------\n\
                 SHADER_COMMON.md → {}\n\
                 Common region does NOT exactly equal the SSoT blocks for `{}`.\n\
                 Extra local WGSL inside the common block is not allowed.\n\
                 \n\
                 --- Expected common region (SSoT order) ---\n\
                 {}\n\
                 --- Actual common region of {} ---\n\
                 {}\n",
                path, short, expected, path, actual
            ));
        }
    }

    if !failures.is_empty() {
        panic!(
            "\nSHADER_COMMON.md SSoT check failed for {} shader(s).\n\
             Fix order:\n  \
             1. Open src/data_render/SHADER_COMMON.md and confirm the canonical text.\n  \
             2. Copy only the SSoT blocks for that shader into its BEGIN/END common region.\n  \
             3. Move shader-local structs/comments outside the common region.\n  \
             4. Re-run `cargo test --test shader_consistency`.\n\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
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
    let glyph = read("src/data_render/vector/glyph_arrow.wgsl");
    let projection = read("src/data_render/vector/project_cartesian2d.wgsl");
    let copies = [
        ("src/data_render/vector/glyph_arrow.wgsl", &glyph, true),
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
        if path != "src/data_render/vector/glyph_arrow.wgsl" {
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

    let rust = read("src/data_render/mod.rs");
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
    let rust = read("src/data_render/mod.rs");
    let field = read("src/data_render/field_columnar.wgsl");
    let anchor = read("src/contour_anchor.wgsl");

    assert!(rust.contains("pub struct ContourLookupMetadataGpu"));
    assert!(rust.contains("pub finite_count: u32"));
    assert!(rust.contains("pub negative_infinity_count: u32"));
    assert!(rust.contains("size_of::<ContourLookupMetadataGpu>() == 8"));
    assert!(rust.contains("storage(6, wgpu::ShaderStages::FRAGMENT)"));
    assert!(rust.contains("binding: 6,"));
    assert!(rust.contains("resource: lookup_metadata_buf.as_entire_binding()"));

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

#[test]
fn every_targeted_shader_has_begin_end_markers() {
    for short in TARGETS {
        let path = shader_path_for(short);
        let raw = read(path);
        assert!(
            raw.contains(BEGIN_MARKER),
            "{}: missing `{}` marker.",
            path,
            BEGIN_MARKER
        );
        assert!(
            raw.contains(END_MARKER),
            "{}: missing `{}` marker.",
            path,
            END_MARKER
        );
    }
}

// A log flag picks between the linear and the logarithmic value with
// `select`, never `mix`: drivers that evaluate `mix(x, y, 1)` as
// `x + (y - x)` lose `y` under a large `x`, which put log-axis values past
// 2^24 at the wrong height (SHADER_COMMON §3). llvmpipe evaluates it
// exactly, so no pixel test here would see the difference.
#[test]
fn log_flags_select_and_never_blend() {
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
    for path in shaders {
        let source = fs::read_to_string(&path).unwrap();
        for (number, line) in source.lines().enumerate() {
            let code = line.split("//").next().unwrap();
            if code.contains("mix(") && (code.contains("is_log") || code.contains("scale_log")) {
                offending.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offending.is_empty(),
        "log flags blended with mix:\n{}",
        offending.join("\n")
    );
}
