//! Source checks for the WGSL shaders.
//!
//! WGSL has no `import`. Code that several shaders need lives once, as a
//! fragment under `src/data_render/common/` (or a layer of the vector and pick
//! shaders), and `wgsl_module!` (`src/shader_assembly.rs`) splices it into
//! every module that needs it at compile time; `SHADER_COMMON.md` documents
//! the contracts. There is nothing left to keep in sync by hand, so the
//! checks here are the guards of #46 (one definition per WGSL function, each
//! remaining copy byte-identical to its fragment, no `..` rest pattern on a
//! model type) and the content checks of individual modules, which read the
//! assembled source a pipeline compiles. That every module parses and
//! validates is `shader_assembly::tests::every_assembled_module_parses_and_validates`.
//!
//! Run with:
//!     cargo test --test shader_consistency

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The assembled module a shader file is the entry of: the content checks
/// below read what the GPU compiles, fragments included.
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
        _ => return None,
    })
}

/// A source file, except that a shader is read as the assembled module it is
/// part of (see [`assembled_module_for`]).
fn read(path_rel: &str) -> String {
    if let Some(name) = assembled_module_for(path_rel) {
        return renderer::shader_assembly::module(name).to_string();
    }
    let p = workspace_root().join(path_rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| panic!("Failed to read {:?}: {}", p, e));
    // Normalize line endings so CRLF (Windows) vs LF doesn't trip substring
    // matching.
    raw.replace("\r\n", "\n")
}

/// Every `.wgsl` file under `dir`, fragments included.
fn wgsl_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            wgsl_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "wgsl") {
            out.push(path);
        }
    }
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

#[test]
fn field_fit_uses_the_ssot_grid_pair_normalization() {
    let ssot = read("src/data_render/common/grid_pair.wgsl");
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
    let mut shaders = Vec::new();
    wgsl_files(&workspace_root().join("src"), &mut shaders);
    assert!(shaders.len() > 10, "found {} shaders", shaders.len());
    let mut offenders = Vec::new();
    for path in shaders {
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

/// The 0/1 flags a shader chooses between two values by (SHADER_COMMON §3.1):
/// a name, and the component it must carry when only one lane of a vector
/// is the flag. A name without a component may carry any `[i]` index or
/// `.x` swizzle. Locals bound as `select(0.0, 1.0, …)` or
/// `select(1.0, 0.0, …)` join the table per function.
const BLEND_FLAGS: [(&str, Option<&str>); 3] = [
    ("scale_log", None),
    ("polar_angle", Some(".x")),
    ("is_log", None),
];

/// The flags that choose between a linear and a logarithmic value.
const LOG_FLAGS: [&str; 2] = ["scale_log", "is_log"];

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// `source` with its `//` and `/* */` comments blanked to spaces, so offsets
/// and line numbers still match the file.
fn strip_wgsl_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if depth == 0 && bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") || (depth > 0 && bytes[i..].starts_with(b"*/")) {
            if bytes[i] == b'/' {
                depth += 1;
            } else {
                depth -= 1;
            }
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
        } else {
            if depth > 0 && bytes[i] != b'\n' {
                out[i] = b' ';
            }
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// The offset of the bracket that closes the `(`, `[` or `{` at `open`.
fn closing_bracket(code: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, &byte) in code.iter().enumerate().skip(open) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced bracket at byte {open}")
}

/// The offset of the bracket that opens the `)` or `]` at `close`.
fn opening_bracket(code: &[u8], close: usize) -> usize {
    let mut depth = 0usize;
    for i in (0..=close).rev() {
        match code[i] {
            b')' | b']' => depth += 1,
            b'(' | b'[' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced bracket at byte {close}")
}

/// `span` without its surrounding whitespace.
fn trimmed_span(code: &str, (start, end): (usize, usize)) -> (usize, usize) {
    let text = &code[start..end];
    let start = start + (text.len() - text.trim_start().len());
    (start, start + text.trim().len())
}

/// The arguments of the call whose `(` is at `open`, split at its top-level
/// commas, each without surrounding whitespace, and the offset of its `)`.
fn call_arguments(code: &str, open: usize) -> (Vec<(usize, usize)>, usize) {
    let bytes = code.as_bytes();
    let close = closing_bracket(bytes, open);
    let mut arguments = Vec::new();
    let mut depth = 0usize;
    let mut start = open + 1;
    for i in open + 1..close {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                arguments.push(trimmed_span(code, (start, i)));
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = trimmed_span(code, (start, close));
    if last.0 < last.1 {
        arguments.push(last);
    }
    (arguments, close)
}

/// The `(` of each call to `name` in `code[from..to]`.
fn call_openings(code: &str, from: usize, to: usize, name: &str) -> Vec<usize> {
    let bytes = code.as_bytes();
    code[from..to]
        .match_indices(name)
        .filter_map(|(offset, _)| {
            let at = from + offset;
            if at > 0 && is_ident(bytes[at - 1]) {
                return None;
            }
            let rest = &code[at + name.len()..to];
            let open = at + name.len() + (rest.len() - rest.trim_start().len());
            (bytes.get(open) == Some(&b'(')).then_some(open)
        })
        .collect()
}

/// Whether the argument spans `a` and `b` are the literals `x` and `y`.
fn literals_are(code: &str, a: (usize, usize), b: (usize, usize), x: f32, y: f32) -> bool {
    let value = |(s, e): (usize, usize)| code[s..e].trim_end_matches('f').parse::<f32>().ok();
    value(a) == Some(x) && value(b) == Some(y)
}

/// Each operand in `code[from..to]` that is `flag`: the span from the start
/// of the `a.b[i].` path the flag is a field of to the end of its `[i]`
/// index or `.x` swizzle.
fn flag_operands(
    code: &str,
    from: usize,
    to: usize,
    (name, component): (&str, Option<&str>),
) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut operands = Vec::new();
    for (offset, _) in code[from..to].match_indices(name) {
        let at = from + offset;
        let mut end = at + name.len();
        if (at > 0 && is_ident(bytes[at - 1])) || bytes.get(end).is_some_and(|b| is_ident(*b)) {
            continue;
        }
        match component {
            Some(component) => {
                let past = end + component.len();
                if !code[end..].starts_with(component)
                    || bytes.get(past).is_some_and(|b| is_ident(*b))
                {
                    continue;
                }
                end = past;
            }
            None if bytes.get(end) == Some(&b'[') => end = closing_bracket(bytes, end) + 1,
            None if bytes.get(end) == Some(&b'.') => {
                let lanes = bytes[end + 1..]
                    .iter()
                    .take_while(|b| is_ident(**b))
                    .count();
                if (1..=4).contains(&lanes)
                    && bytes[end + 1..end + 1 + lanes]
                        .iter()
                        .all(|b| b"xyzwrgba".contains(b))
                {
                    end += 1 + lanes;
                }
            }
            None => {}
        }
        let mut start = at;
        while start > 0 && bytes[start - 1] == b'.' {
            let mut p = start - 1;
            if p > 0 && bytes[p - 1] == b']' {
                p = opening_bracket(bytes, p - 1);
            }
            while p > 0 && is_ident(bytes[p - 1]) {
                p -= 1;
            }
            start = p;
        }
        operands.push((start, end));
    }
    operands
}

/// The last non-whitespace byte before `at` (not before `floor`), with its offset.
fn byte_before(code: &[u8], at: usize, floor: usize) -> Option<(usize, u8)> {
    (floor..at)
        .rev()
        .find(|&i| !code[i].is_ascii_whitespace())
        .map(|i| (i, code[i]))
}

/// The first non-whitespace byte at or after `at`.
fn byte_after(code: &[u8], at: usize) -> Option<u8> {
    code[at..]
        .iter()
        .copied()
        .find(|b| !b.is_ascii_whitespace())
}

/// Whether the operand at `span` is a factor of a product, directly
/// (`f * y`, `y * f`, `y *= f`) or as a term of a parenthesized sum that is
/// one (`(1.0 - f) * y`). `floor` is the start of the function body.
fn is_product_factor(code: &str, (mut start, mut end): (usize, usize), floor: usize) -> bool {
    let bytes = code.as_bytes();
    loop {
        let before = byte_before(bytes, start, floor);
        let after = byte_after(bytes, end);
        if before.map(|(_, b)| b) == Some(b'*') || after == Some(b'*') {
            return true;
        }
        if let Some((i, b'=')) = before
            && i > 0
            && bytes[i - 1] == b'*'
        {
            return true;
        }
        let term = matches!(before, Some((_, b'(' | b'+' | b'-')))
            && matches!(after, Some(b')' | b'+' | b'-'));
        if !term {
            return false;
        }
        // The parenthesis the sum sits in, unless the sum is a call argument.
        let mut depth = 0usize;
        let mut open = None;
        for i in (floor..start).rev() {
            match bytes[i] {
                b')' | b']' => depth += 1,
                b'(' if depth == 0 => {
                    open = Some(i);
                    break;
                }
                b'(' | b'[' if depth > 0 => depth -= 1,
                b'[' | b';' | b'{' | b'}' if depth == 0 => break,
                _ => {}
            }
        }
        let Some(open) = open else {
            return false;
        };
        if let Some((i, byte)) = byte_before(bytes, open, floor) {
            let word_start = (floor..=i).rev().take_while(|&j| is_ident(bytes[j])).last();
            let word = word_start.map_or("", |j| &code[j..=i]);
            let keyword = ["return", "if", "while", "switch", "case"].contains(&word);
            if byte == b'>' || (!word.is_empty() && !keyword) {
                return false;
            }
        }
        start = open;
        end = closing_bracket(bytes, open) + 1;
    }
}

/// Whether an `if` starting in `code[from..to]` tests `operand > 0.5`, and,
/// when `returns`, returns from its block.
fn has_flag_guard(code: &str, from: usize, to: usize, operand: &str, returns: bool) -> bool {
    let bytes = code.as_bytes();
    code[from..to].match_indices("if").any(|(offset, _)| {
        let at = from + offset;
        if (at > 0 && is_ident(bytes[at - 1])) || bytes.get(at + 2).is_some_and(|b| is_ident(*b)) {
            return false;
        }
        let Some(open) = code[at..].find('{').map(|o| at + o) else {
            return false;
        };
        let tests = flag_operands(code, at + 2, open, (operand, None))
            .into_iter()
            .any(|(s, e)| {
                &code[s..e] == operand && code[e..open].trim_start().starts_with("> 0.5")
            });
        let block = &code[open..=closing_bracket(bytes, open)];
        tests && (!returns || block.contains("return"))
    })
}

/// The bodies (`{` to `}`) of the functions in `code`.
fn function_bodies(code: &str) -> Vec<(usize, usize)> {
    let bytes = code.as_bytes();
    let mut bodies = Vec::new();
    let mut from = 0;
    while let Some(offset) = code[from..].find("fn ") {
        let at = from + offset;
        if at > 0 && is_ident(bytes[at - 1]) {
            from = at + 3;
            continue;
        }
        let Some(open) = code[at..].find('{').map(|o| at + o) else {
            break;
        };
        let close = closing_bracket(bytes, open);
        bodies.push((open, close));
        from = close;
    }
    bodies
}

/// The locals of `code[from..to]` bound as `let|var NAME = select(0.0, 1.0,
/// …)` or `select(1.0, 0.0, …)`.
fn select_bound_flags(code: &str, from: usize, to: usize) -> Vec<&str> {
    let bytes = code.as_bytes();
    let mut names = Vec::new();
    for keyword in ["let", "var"] {
        for (offset, _) in code[from..to].match_indices(keyword) {
            let at = from + offset;
            let mut p = at + keyword.len();
            if (at > 0 && is_ident(bytes[at - 1])) || is_ident(bytes[p]) {
                continue;
            }
            if bytes[p] == b'<' {
                p += code[p..].find('>').unwrap() + 1;
            }
            let name_start = p + (code[p..].len() - code[p..].trim_start().len());
            let name_end = name_start
                + bytes[name_start..]
                    .iter()
                    .take_while(|b| is_ident(**b))
                    .count();
            let Some(eq) = code[name_end..to].find(['=', ';']).map(|o| name_end + o) else {
                continue;
            };
            if name_start == name_end || bytes[eq] != b'=' {
                continue;
            }
            let Some(&open) = call_openings(code, eq + 1, to, "select").first() else {
                continue;
            };
            if code[eq + 1..open].trim() != "select" {
                continue;
            }
            let (arguments, _) = call_arguments(code, open);
            if arguments.len() == 3
                && (literals_are(code, arguments[0], arguments[1], 0.0, 1.0)
                    || literals_are(code, arguments[0], arguments[1], 1.0, 0.0))
            {
                names.push(&code[name_start..name_end]);
            }
        }
    }
    names
}

// A flag chooses between two values only through a guarded `mix`. Drivers
// that evaluate `mix(x, y, 1)` as `x + (y - x)` lose `y` under a large `x`,
// which put log-axis values past 2^24 at the wrong height, so the chosen
// value is returned (or assigned) by an `if flag > 0.5` before the `mix`
// sees a 1. The linear arithmetic stays the original `mix`, which DX12 needs
// bit for bit at large offsets (SHADER_COMMON §3). A hand-written blend
// (`x + f * (y - x)`, `f * y + (1.0 - f) * x`) has the same failure and no
// guard can make it exact, so a flag is never a factor of a product, and a
// log flag never picks through `select`. llvmpipe evaluates `mix` exactly,
// so no pixel test here would see the difference. The flag table is
// `BLEND_FLAGS` plus each function's `select(0.0, 1.0, …)` locals.
#[test]
fn flag_choices_never_blend_by_arithmetic() {
    let mut shaders = Vec::new();
    wgsl_files(&workspace_root().join("src"), &mut shaders);
    shaders.sort();
    assert!(shaders.len() > 10, "found {} shaders", shaders.len());
    let mut offending = Vec::new();
    let mut found = [0usize; BLEND_FLAGS.len()];
    let mut select_locals = 0;
    let mut guarded = 0;
    for path in shaders {
        let source = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        let code = strip_wgsl_comments(&source);
        let mut offend = |at: usize, why: &str| {
            let line = code[..at].matches('\n').count();
            let text = source.lines().nth(line).unwrap().trim();
            offending.push(format!("{}:{}: {why}: {text}", path.display(), line + 1));
        };
        for (count, flag) in found.iter_mut().zip(BLEND_FLAGS) {
            *count += flag_operands(&code, 0, code.len(), flag).len();
        }
        for (lo, hi) in function_bodies(&code) {
            let locals = select_bound_flags(&code, lo, hi);
            select_locals += locals.len();
            let flags: Vec<(&str, Option<&str>)> = BLEND_FLAGS
                .into_iter()
                .chain(locals.into_iter().map(|name| (name, None)))
                .collect();

            // (a) `mix(a, b, flag)` behind a guard; no other weight from a flag.
            for open in call_openings(&code, lo, hi, "mix") {
                let (arguments, close) = call_arguments(&code, open);
                let Some(&(mut s, mut e)) = arguments.get(2) else {
                    continue;
                };
                while code.as_bytes()[s] == b'(' && closing_bracket(code.as_bytes(), s) == e - 1 {
                    (s, e) = trimmed_span(&code, (s + 1, e - 1));
                }
                let Some(operands) = flags
                    .iter()
                    .map(|&flag| flag_operands(&code, s, e, flag))
                    .find(|operands| !operands.is_empty())
                else {
                    continue;
                };
                if operands != [(s, e)] {
                    offend(open, "`mix` weight computed from a flag");
                    continue;
                }
                let operand = &code[s..e];
                // The override sits within the two lines after the `mix`.
                let line_end = |at: usize| code[at..].find('\n').map_or(code.len(), |o| at + o);
                let mut next_two = line_end(close);
                for _ in 0..2 {
                    if next_two < code.len() {
                        next_two = line_end(next_two + 1);
                    }
                }
                if has_flag_guard(&code, lo, open, operand, true)
                    || has_flag_guard(&code, close, next_two, operand, false)
                {
                    guarded += 1;
                } else {
                    offend(open, "`mix` by a flag without an `if flag > 0.5` guard");
                }
            }

            // (b) A flag is never a factor: that is a hand-written blend.
            for &flag in &flags {
                for operand in flag_operands(&code, lo + 1, hi, flag) {
                    if is_product_factor(&code, operand, lo + 1) {
                        offend(operand.0, "flag used as a `*` factor");
                    }
                }
            }

            // (c) A log flag never picks through `select`.
            for open in call_openings(&code, lo, hi, "select") {
                let (arguments, close) = call_arguments(&code, open);
                if arguments.len() == 3 && literals_are(&code, arguments[0], arguments[1], 0.0, 1.0)
                {
                    continue;
                }
                if LOG_FLAGS
                    .iter()
                    .any(|&name| !flag_operands(&code, open, close, (name, None)).is_empty())
                {
                    offend(open, "`select` by a log flag");
                }
            }
        }
    }
    for (count, (name, component)) in found.iter().zip(BLEND_FLAGS) {
        assert!(
            *count > 0,
            "flag `{name}{}` of BLEND_FLAGS is in no shader: update the table and SHADER_COMMON §3.1",
            component.unwrap_or("")
        );
    }
    assert!(
        select_locals > 0,
        "no local bound as `select(0.0, 1.0, …)` is left: update the table and SHADER_COMMON §3.1"
    );
    assert!(
        offending.is_empty(),
        "flags blended by arithmetic:\n{}",
        offending.join("\n")
    );
    // Each shared fragment is scanned once, so the floor counts the guarded
    // choices of `maybe_log`, `axis_pair_to_t`, `ramp_t` and
    // `axis_px_per_unit_at`: the scan must keep finding them.
    assert!(guarded >= 4, "{guarded} guarded flag choices found");
}

/// Rule from #46: one behaviour has one implementation. Every WGSL function
/// defined in more than one product shader under `src/` is listed below, and
/// the test fails on any change to that picture: a new copy (a name defined
/// in two files that is not listed, or listed with a different file set) or
/// a stale entry (a listed copy that no longer exists). The list therefore
/// only shrinks. Entry points (`@vertex`, `@fragment`, `@compute`) are
/// exempt: their names are per pipeline, not shared code. A function that
/// several modules need is a fragment under `src/data_render/common/`,
/// spliced by `wgsl_module!`, and so defined once.
///
/// `ASSEMBLED_ALTERNATIVES` are interface functions with one body per
/// variant, of which the Rust assembler splices exactly one into a module.
/// They are the layered design of rule 9 and stay. Each entry is (files that
/// define them, names).
#[test]
fn every_wgsl_function_is_defined_once_outside_the_allowlist() {
    let src = workspace_root().join("src");
    let mut shaders = Vec::new();
    wgsl_files(&src, &mut shaders);
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
    for (files, names) in ASSEMBLED_ALTERNATIVES {
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
            "data_render/vector/vector_input_grid.wgsl",
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
            "gpu_errorbar.wgsl",
            "gpu_pick/core.wgsl",
        ],
        &["figgy_zero_workgroup"],
    ),
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
