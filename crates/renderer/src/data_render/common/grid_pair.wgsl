fn grid_rounded_add(a: f32, b: f32) -> f32 {
    return fma(1.0, a, b);
}

fn grid_rounded_subtract(a: f32, b: f32) -> f32 {
    return fma(1.0, a, -b);
}

/// Normalize a finite pair with error-free addition. Explicit fma additions
/// preserve rounding boundaries on Metal; round-trip bitcasts are eliminated
/// by optimizers and do not prevent reassociation.
fn normalize_grid_pair(v: vec2<f32>) -> vec2<f32> {
    let sum = grid_rounded_add(v.x, v.y);
    let b_virtual = grid_rounded_subtract(sum, v.x);
    let a_virtual = grid_rounded_subtract(sum, b_virtual);
    let b_roundoff = grid_rounded_subtract(v.y, b_virtual);
    let a_roundoff = grid_rounded_subtract(v.x, a_virtual);
    return vec2<f32>(sum, grid_rounded_add(a_roundoff, b_roundoff));
}

fn add_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let high = normalize_grid_pair(vec2<f32>(a.x, b.x));
    let low = grid_rounded_add(grid_rounded_add(a.y, b.y), high.y);
    return normalize_grid_pair(vec2<f32>(high.x, low));
}

fn subtract_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return add_grid_pairs(a, -b);
}

fn scale_grid_pair(v: vec2<f32>, factor: f32) -> vec2<f32> {
    return normalize_grid_pair(vec2<f32>(v.x * factor, v.y * factor));
}

fn midpoint_grid_pair(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    // Scale first so two same-sign maximum f32 coordinates can still have a
    // finite midpoint.
    return add_grid_pairs(scale_grid_pair(a, 0.5), scale_grid_pair(b, 0.5));
}
