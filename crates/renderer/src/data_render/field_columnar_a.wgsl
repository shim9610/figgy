// Label-gap inputs exist only in `fs_contour_labelled`'s call graph. The plain
// `fs_contour` entry therefore keeps the original three-group layout, while a
// labelled contour binds the exact selected-anchor buffers as group 3.
struct LabelGapAnchor {
    x: vec2<f32>,
    y: vec2<f32>,
    dir: vec2<f32>,
    level: u32,
    width_px: f32,
};

struct LabelGapParams {
    atlas_w: f32,
    atlas_h: f32,
    cell_stride_w: f32,
    cell_stride_h: f32,
    columns: u32,
    rows: u32,
    gutter: f32,
    level_count: u32,
};

@group(3) @binding(0) var<storage, read> label_gap_anchors: array<LabelGapAnchor>;
@group(3) @binding(1) var<storage, read> label_gap_draw_args: array<u32>;
@group(3) @binding(3) var<uniform> label_gap_params: LabelGapParams;

/// One block's portable lookup bounds. CPU twin:
/// `mod.rs::ContourLookupMetadataGpu` (`#[repr(C)]`, 8 B).
struct ContourLookupMetadata {
    finite_count: u32,
    negative_infinity_count: u32,
};

/// Field-fragment-only lookup metadata. The anchor pipeline shares group 2 but
/// never searches contour levels, so binding 6 is not compute-visible.
@group(2) @binding(6) var<storage, read> contour_lookup_metadata: array<ContourLookupMetadata>;

// ───── shared colour ramp (common/ramp.wgsl) ─────
