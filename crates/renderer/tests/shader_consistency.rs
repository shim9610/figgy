//! Mechanical SSoT check for WGSL shader common blocks.
//!
//! WGSL has no `import`/`include`. The `Transform` / `Style` / `maybe_log` /
//! `data_to_ndc` definitions are therefore duplicated across
//! `scatter_columnar.wgsl`, `line_columnar.wgsl`, `errorbar_columnar.wgsl`,
//! `bar_columnar.wgsl`, `field_columnar.wgsl`, the `line_arc.wgsl` and
//! `contour_anchor.wgsl` compute shaders, and the `contour_label.wgsl`
//! render shader. `src/data_render/SHADER_COMMON.md` is the single source of
//! truth for those duplicates.
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
    "scatter", "line", "errorbar", "bar", "field", "arc", "anchor", "label",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path_rel: &str) -> String {
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
        other => panic!(
            "Unknown shader short-name `{}` in SHADER_COMMON.md metadata. \
             Valid names: scatter | line | errorbar | bar | field | arc | \
             anchor | label.",
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
fn contour_label_capacity_and_workgroup_storage_match_the_model_contract() {
    const WEBGPU_MIN_WORKGROUP_STORAGE_BYTES: usize = 16_384;
    const KEPT_POSITION_BYTES: usize = 1024 * std::mem::size_of::<[f32; 2]>();
    const KEPT_RADIUS_BYTES: usize = 1024 * std::mem::size_of::<f32>();

    assert_eq!(renderer::MAX_CONTOUR_LEVELS, 1024);
    assert_eq!(renderer::gpu_contour::MAX_CONTOUR_LABELS_TOTAL, 1024);
    assert!(renderer::gpu_contour::MAX_ANCHOR_CANDIDATES >= renderer::MAX_CONTOUR_LEVELS as u32);
    assert_eq!(KEPT_POSITION_BYTES + KEPT_RADIUS_BYTES, 12_288);
    const {
        assert!(KEPT_POSITION_BYTES + KEPT_RADIUS_BYTES <= WEBGPU_MIN_WORKGROUP_STORAGE_BYTES);
    }

    let rust = read("src/gpu_contour.rs");
    let shader = read("src/contour_anchor.wgsl");
    assert!(rust.contains("pub const MAX_CONTOUR_LABELS_TOTAL: u32 = 1024;"));
    assert!(
        rust.contains("MAX_ANCHOR_CANDIDATES >= crate::data_config::MAX_CONTOUR_LEVELS as u32")
    );
    assert!(shader.contains("const MAX_KEPT: u32 = 1024u;"));
    assert!(shader.contains("var<workgroup> kept_px: array<vec2<f32>, 1024>;"));
    assert!(shader.contains("var<workgroup> kept_r: array<f32, 1024>;"));
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

    assert!(field.contains("if (!f32_is_finite(raw))"));
    assert!(field.contains("!f32_is_finite(normalized)"));
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
