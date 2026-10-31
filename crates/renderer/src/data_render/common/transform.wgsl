struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // milkyway:      [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, star_brightness, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    // constellation: [0] = (star_opacity, line_opacity, 0, 0)
    // All styles reserve [2].z for the global point-base u32 BIT PATTERN.
    // Resident draws write zero; streamed point/errorbar draws bitcast it.
    style_params: array<vec4<f32>, 3>,
    // Polar coordinates (Config.polar); all zero in a rectangular chart.
    // polar_angle:  (flag, zero angle in radians, radians per angle unit
    //               signed by the direction, inner radius fraction)
    // polar_extent: (sector start, sector end in angle units, end <= start
    //               for the full circle; arc pieces per segment, 0 for
    //               chords; outer radius in pixels)
    polar_angle: vec4<f32>,
    polar_extent: vec4<f32>,
    // Ternary coordinates (Config.ternary); all zero otherwise.
    // ternary_lt: (L.x, L.y, T.x, T.y), the corners of x's and y's 100 %
    //             in NDC
    // ternary_r:  (R.x, R.y, flag, 0), the corner of c's 100 % in NDC and
    //             1 for a ternary chart
    ternary_lt: vec4<f32>,
    ternary_r: vec4<f32>,
};  // 176 B (vec4 array at offset 64, stride 16; polar vec4s at 112 and 128; ternary vec4s at 144 and 160)

@group(0) @binding(0) var<uniform> transform: Transform;

fn styled_point_index(local_index: u32) -> u32 {
    return bitcast<u32>(transform.style_params[2].z) + local_index;
}
