// ───── end shared marker shape ─────

@vertex
fn vs_main(in: VsIn) -> VsOut {
    if (!sample_valid(in.x, in.y)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);
    let half_px =
        visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let r = style.point_radius_px;
    let filled = shape_is_filled(style.shape_id);
    let base = base_shape_id(style.shape_id);
    let visual_r = visual_shape_radius(base, r);
    // Pixel-space position — same expanded scale as the vs_main quad, so
    // one unit here is one screen pixel.
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX);

    let d = shape_distance(base, p, visual_r, filled);
    let aa = fwidth(d);
    // Filled: cover d < 0. Outline/Cross: ~1.5 px stroke centered on the
    // d == 0 contour.
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry points selected as a separate pipeline variant. The precise
// entries above never read the sketch Transform fields.

// Deliberately duplicated per data shader (scatter/line/errorbar) and NOT in
// the SHADER_COMMON.md common block: line_arc.wgsl shares that block but has
// no use for noise. Keep these helpers byte-identical in all three shaders.
// Per-point style mapping (NOT part of the common block). The default precise
// path above remains the three-slot fast path; this entry is selected only for
// scatter series with a style table, style-index column, or sparse overrides.
struct VsMappedIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
    @location(3) style_index: f32,
    // Value columns of the continuous maps (`point_color_map` /
    // `point_size_map`), per-instance hi/lo pairs read straight from the
    // column pool. A series without the map binds its X column here as inert
    // filler; the flags in `point_value_map` say which of them are real.
    @location(4) color_value: vec2<f32>,
    @location(5) size_value: vec2<f32>,
};

struct ScatterStyleSlot {
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ScatterStyleOverride {
    point_index: u32,
    // Keep this layout byte-for-byte with Rust's ScatterStyleOverrideGpu:
    // u32 + three scalar u32 pads, then two vec4<f32> fields (48 bytes).
    // A WGSL vec3<u32> would align to 16 bytes and shift color_premul.
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ScatterStyleMapMeta {
    style_count: u32,
    override_count: u32,
    has_index: u32,
    _pad: u32,
};

@group(2) @binding(5) var<storage, read> scatter_style_slots: array<ScatterStyleSlot>;
@group(2) @binding(6) var<storage, read> scatter_style_overrides: array<ScatterStyleOverride>;
@group(2) @binding(7) var<uniform> scatter_style_meta: ScatterStyleMapMeta;

// Continuous value maps (group 3, NOT part of the common block). The colour
// side mirrors the field shader: the colourbar's bounds (already log10 when
// `VALUE_MAP_LOG_Z` is set), its `nan_color` and its ramp control points, so a
// colour-mapped point and a heatmap cell with the same z draw the same colour.
// The size side is `PointSizeMap`: value domain as hi/lo pairs and the pixel
// range already multiplied by the export scale. CPU twin:
// `mod.rs::PointValueMapParamsGpu`, 64 B.
struct PointValueMapParams {
    z_min: vec2<f32>,
    z_max: vec2<f32>,
    size_domain_min: vec2<f32>,
    size_domain_max: vec2<f32>,
    // (min_px, max_px)
    size_px: vec2<f32>,
    flags: u32,
    stop_count: u32,
    nan_color_premul: vec4<f32>,
};

const VALUE_MAP_COLOR: u32 = 1u;
const VALUE_MAP_SIZE: u32 = 2u;
const VALUE_MAP_LOG_Z: u32 = 4u;
const VALUE_MAP_SIZE_AREA: u32 = 8u;

@group(3) @binding(0) var<uniform> point_value_map: PointValueMapParams;
@group(3) @binding(1) var<storage, read> point_value_stops: array<vec4<f32>>;

fn value_map_flag(bit: u32) -> bool {
    return (point_value_map.flags & bit) != 0u;
}


// ───── shared colour ramp and size-map radius (SHADER_COMMON.md §5.4;
// `shader_consistency` compares these function bodies with the field, vector
// and pick shaders) ─────
