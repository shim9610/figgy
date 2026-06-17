//! Mechanical SSoT check for WGSL shader common blocks.
//!
//! WGSL has no `import`/`include`. The `Transform` / `Style` / `maybe_log` /
//! `data_to_ndc` definitions are therefore duplicated across
//! `scatter_columnar.wgsl`, `line_columnar.wgsl`, `errorbar_columnar.wgsl`,
//! and the `line_arc.wgsl` compute shader. `src/data_render/SHADER_COMMON.md`
//! is the single source of truth for those duplicates.
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
//! The repository's pre-commit hook (`.githooks/pre-commit`) runs this test
//! automatically whenever any `.wgsl` file or `SHADER_COMMON.md` is staged.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const SSOT_PATH: &str = "src/data_render/SHADER_COMMON.md";
const BEGIN_MARKER: &str = "// ───── BEGIN common block";
const END_MARKER: &str = "// ───── END common block";
const TARGETS: &[&str] = &["scatter", "line", "errorbar", "arc"];

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
        "arc" => "src/data_render/line_arc.wgsl",
        other => panic!(
            "Unknown shader short-name `{}` in SHADER_COMMON.md metadata. \
             Valid names: scatter | line | errorbar | arc.",
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
