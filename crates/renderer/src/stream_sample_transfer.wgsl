@group(0) @binding(0) var prefix: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}

@fragment fn transfer(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(prefix, vec2<i32>(p.xy), 0);
}

// Preview-only rescaling. Final output uses an exact source replay.
struct PreviewMap {
    source_from_destination: vec4<f32>,
    source_clip: vec4<f32>,
};
@group(0) @binding(1) var<uniform> preview: PreviewMap;

fn preview_texel(p: vec2<i32>, sample: i32) -> vec4<f32> {
    let center = vec2<f32>(p) + vec2(0.5);
    if any(center < preview.source_clip.xy) || any(center >= preview.source_clip.zw) {
        return vec4(0.0);
    }
    return textureLoad(prefix, p, 0);
}

@fragment fn rescale(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let source = p.xy * preview.source_from_destination.xy + preview.source_from_destination.zw - vec2(0.5);
    let base = vec2<i32>(floor(source));
    let fraction = fract(source);
    return mix(
        mix(preview_texel(base, 0), preview_texel(base + vec2(1, 0), 0), fraction.x),
        mix(preview_texel(base + vec2(0, 1), 0), preview_texel(base + vec2(1, 1), 0), fraction.x),
        fraction.y);
}
