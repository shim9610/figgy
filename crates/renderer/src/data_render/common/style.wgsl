struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    // Per-series decorrelation salt (FNV-1a of series_id). Styled entries
    // (sketch/milkyway/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    // Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    // for X. Line entries read bits 2-3 as the step interpolation mode
    // (0 linear, 1 step-pre, 2 step-post, 3 step-mid) and the area entry
    // also reads bit 4 (fill to a series instead of a baseline); every
    // other primitive ignores this field.
    primitive_flags: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;
