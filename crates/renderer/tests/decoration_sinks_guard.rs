//! Drawing that bypasses placement does not ship: every decoration, axis,
//! polar, colourbar, annotation and bounded layout file places its parts
//! through a `Sink`, and only the canvas sinks turn parts into pixels. A
//! direct text, rectangle or line draw in one of these files is a second
//! placement the hit geometry cannot see (#38, #46).

/// The files whose every text and rectangle goes through a sink, with the
/// direct line draws each may keep: chrome that belongs to no element (the
/// bounded charts' grid lines, a radar's rings and spokes, a radial chart's
/// leader lines), listed so a new one is a decision here.
const PLACED_FILES: &[(&str, &str, &[&str])] = &[
    ("axis_render.rs", include_str!("../src/axis_render.rs"), &[]),
    (
        "axis_render/grid.rs",
        include_str!("../src/axis_render/grid.rs"),
        &[],
    ),
    (
        "axis_render/line.rs",
        include_str!("../src/axis_render/line.rs"),
        &[],
    ),
    ("polar_axes.rs", include_str!("../src/polar_axes.rs"), &[]),
    (
        "decoration_render.rs",
        include_str!("../src/decoration_render.rs"),
        &[],
    ),
    (
        "decoration_render/tick_labels.rs",
        include_str!("../src/decoration_render/tick_labels.rs"),
        &[],
    ),
    (
        "decoration_render/colorbar.rs",
        include_str!("../src/decoration_render/colorbar.rs"),
        &["draw_rect"],
    ),
    (
        "decoration_render/axis_titles.rs",
        include_str!("../src/decoration_render/axis_titles.rs"),
        &[],
    ),
    ("annotation.rs", include_str!("../src/annotation.rs"), &[]),
    ("decoration.rs", include_str!("../src/decoration.rs"), &[]),
    (
        "bounded_title.rs",
        include_str!("../src/bounded_title.rs"),
        &[],
    ),
    (
        "bounded_axes.rs",
        include_str!("../src/bounded_axes.rs"),
        &[],
    ),
    (
        "radial_layout.rs",
        include_str!("../src/radial_layout.rs"),
        &["draw_line"],
    ),
    (
        "radar_layout.rs",
        include_str!("../src/radar_layout.rs"),
        &["draw_line", "draw_polyline"],
    ),
];

/// Canvas calls that paint: text, filled or stroked shapes, masks.
const DRAW_CALLS: &[&str] = &[
    "draw_plain_text(",
    "draw_rich_text(",
    ".draw_rect(",
    ".draw_line(",
    ".draw_polyline(",
    ".draw_polyline_smooth(",
    ".fill_polygon(",
    ".blit_mask(",
    ".blit_mask_orthogonal(",
    ".draw_mask(",
];

/// Lines of the product part of `source` (before its inline tests) that
/// are code, not comments.
fn product_code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let product_end = source.find("#[cfg(test)]").unwrap_or(source.len());
    source[..product_end]
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim_start()))
        .filter(|(_, line)| !line.starts_with("//"))
}

#[test]
fn decorations_are_painted_only_through_the_canvas_sinks() {
    let mut violations = Vec::new();
    for (name, source, allowed) in PLACED_FILES {
        for (line_number, line) in product_code_lines(source) {
            for call in DRAW_CALLS {
                let bare = call.trim_start_matches('.').trim_end_matches('(');
                if line.contains(call) && !allowed.contains(&bare) {
                    violations.push(format!("{name}:{line_number}: {line}"));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "direct canvas draws outside the canvas sinks:\n{}",
        violations.join("\n")
    );
}

/// The colourbar strip's bands are coverage the canvas sink paints
/// (`draw_colorbar_strip`); nothing else in the colourbar file draws.
#[test]
fn the_colorbar_file_draws_only_its_ramp_bands() {
    let source = include_str!("../src/decoration_render/colorbar.rs");
    let strip = source
        .find("pub(crate) fn draw_colorbar_strip(")
        .expect("the ramp painter is in the colourbar file");
    for (line_number, line) in product_code_lines(&source[..strip]) {
        assert!(
            !line.contains(".draw_rect("),
            "colorbar.rs:{line_number} draws outside the ramp painter: {line}"
        );
    }
}
