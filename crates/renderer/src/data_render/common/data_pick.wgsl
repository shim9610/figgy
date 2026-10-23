// The typed data pick's query (`gpu_data_pick.rs::DataPickQueryGpu`, 80 B);
// each primitive reads the lanes it needs.
struct DataPickQuery {
    // xy = cursor NDC, zw = cursor axis-t.
    cursor_ndc_t: vec4<f32>,
    // x = max distance px; bar: y = scaled bar gap px, z = bar width ratio;
    // field: z = contour width px.
    limits: vec4<f32>,
    // x = flags, y = source paint order; bar: z = item count,
    // w = orientation.
    data: vec4<u32>,
    // bar: x = edge-column f32 lane base, y = value-column lane base,
    // z = global bin offset.
    bases: vec4<u32>,
    // bar: xy = histogram baseline (hi, lo).
    baseline: vec4<f32>,
};

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

// The highlight of a selected datum (`selection.rs::DataSelectionGpu`).
struct DataSelection {
    color_premul: vec4<f32>,
    // x = cell/bar outline width, y = selected contour extra width.
    metrics: vec4<f32>,
    // x = kind, y/z/w = kind-specific indices.
    indices: vec4<u32>,
};
