@group(0) @binding(0) var prefix: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}

@fragment fn transfer(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(prefix, vec2<i32>(p.xy), 0);
}
