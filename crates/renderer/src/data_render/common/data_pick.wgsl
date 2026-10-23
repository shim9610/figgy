// Thirty-two bytes, byte-for-byte with Rust's `DataPickCandidateGpu`.
struct DataPickCandidate {
    valid: u32,
    paint_order: u32,
    kind: u32,
    index0: u32,
    index1: u32,
    index2: u32,
    distance_px: f32,
    primitive_order: u32,
};
