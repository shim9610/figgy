// Scalar-only reduction ABI. Geometry remains in the point and bar shaders.
struct StreamDataCandidate {
    valid: u32,
    paint_order: u32,
    kind: u32,
    index0: u32,
    index1: u32,
    index2: u32,
    distance_px: f32,
    primitive_order: u32,
};

@group(0) @binding(0) var<storage, read> input_words: array<u32>;
@group(0) @binding(1) var<storage, read_write> running_best: StreamDataCandidate;

fn better(candidate: StreamDataCandidate, incumbent: StreamDataCandidate) -> bool {
    if candidate.valid == 0u { return false; }
    if incumbent.valid == 0u { return true; }
    if candidate.distance_px != incumbent.distance_px {
        return candidate.distance_px < incumbent.distance_px;
    }
    if candidate.paint_order != incumbent.paint_order {
        return candidate.paint_order > incumbent.paint_order;
    }
    // Point/line has already used distance-squared and source-order ties in
    // its own accumulator. Cross-kind ties retain the resident point answer.
    if candidate.kind != incumbent.kind { return candidate.kind < incumbent.kind; }
    return candidate.kind == 1u && candidate.primitive_order > incumbent.primitive_order;
}

@compute @workgroup_size(1)
fn accumulate_data() {
    let candidate = StreamDataCandidate(
        input_words[0], input_words[1], input_words[2], input_words[3],
        input_words[4], input_words[5], bitcast<f32>(input_words[6]), input_words[7],
    );
    if better(candidate, running_best) { running_best = candidate; }
}

@compute @workgroup_size(1)
fn accumulate_point() {
    let candidate = StreamDataCandidate(
        input_words[0], input_words[1], 0u, input_words[2],
        0u, 0u, bitcast<f32>(input_words[6]), input_words[4],
    );
    if better(candidate, running_best) { running_best = candidate; }
}
