// Sketch hash and value noise (SHADER_COMMON.md §5.6): the sketch stroke, the
// sketch errorbar and the styled markers draw and pick with it.
fn sketch_hash01(i: u32, seed: u32) -> f32 {
    var h = (i * 0x9E3779B9u) ^ (seed * 0x85EBCA6Bu);
    h = (h ^ (h >> 16u)) * 0x45D9F3Bu;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967296.0;
}

// [-1, 1], C1-continuous. Negative t is fine: the lattice index wraps in u32
// (floor(-0.3) → -1 → 0xFFFFFFFF, and +1u wraps back to 0), so continuity at
// integer boundaries is preserved.
fn sketch_noise(t: f32, seed: u32) -> f32 {
    return sketch_noise_lattice(u32(i32(floor(t))), fract(t), seed);
}

// The noise in lattice cell `i` at fraction `f` in [0, 1): `sketch_noise`
// at `t` is cell `floor(t)` at `fract(t)`. The arc readers (`arc_noise`,
// line_stroke.wgsl) give the cell of an arc pair directly.
fn sketch_noise_lattice(i: u32, f: f32, seed: u32) -> f32 {
    let u = f * f * (3.0 - 2.0 * f);
    return mix(sketch_hash01(i, seed), sketch_hash01(i + 1u, seed), u) * 2.0 - 1.0;
}
