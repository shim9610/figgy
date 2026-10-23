// Columnar SDF scatter shader.
//
// slot 0: unit quad (per-vertex, ±1 NDC).
// slot 1: X column (per-instance, f32 hi/lo pair).
// slot 2: Y column (per-instance, f32 hi/lo pair).
//
// Marker shapes — style.shape_id is the stable code from mod.rs::shape_id().
// Existing ids 0..8 are kept compatible; newer scientific-plot symbols append
// after them. Shape decoding below maps those ids to compact SDF base shapes
// plus an open/filled flag.
