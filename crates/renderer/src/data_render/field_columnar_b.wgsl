// ── Field grid sampling (group 2) ───────────────────────────────────────────
//
// No textures and no vertex buffers: everything is read from storage. The pool
// is bound whole, with the coordinate columns located by `FieldParams` lane
// bases, exactly as the constellation star pass does it.

/// One constituent grid column: where it starts in the pool (as an f32 lane
/// index) and how many logical values it holds.
struct GridColumn {
    base: u32,
    len: u32,
};

/// Bit flags in `FieldParams.flags`.
const FIELD_COLUMNS_ARE_Y: u32 = 1u;
const FIELD_CENTERS: u32 = 2u;
const FIELD_INTERPOLATED: u32 = 4u;
const FIELD_BANDS: u32 = 8u;
const FIELD_LOG_Z: u32 = 16u;

/// Everything the field needs that is not already in `Transform` / `Style`.
///
/// `z_min` / `z_max` arrive **already log-transformed** when `FIELD_LOG_Z` is
/// set: the colourbar's bounds are f64 on the host, so taking their logarithm
/// there keeps the precision that doing it here would lose. Both are `(hi, lo)`
/// f32 pairs, the pool's own representation.
///
/// CPU twin: `mod.rs::FieldParamsGpu` (`#[repr(C)]`, 64 B).
struct FieldParams {
    x_base: u32,
    y_base: u32,
    x_len: u32,
    y_len: u32,
    cols: u32,
    rows: u32,
    level_count: u32,
    stop_count: u32,
    flags: u32,
    opacity: f32,
    /// Contour stroke width in pixels, already multiplied by the render scale.
    /// Read by `fs_contour`; the fill ignores it.
    line_width_px: f32,
    /// Entries in `level_colors`. A level past the end takes the last one.
    level_color_count: u32,
    z_min: vec2<f32>,
    z_max: vec2<f32>,
};

@group(2) @binding(0) var<storage, read> field_pool: array<f32>;
@group(2) @binding(1) var<storage, read> grid: array<GridColumn>;
@group(2) @binding(2) var<storage, read> levels: array<f32>;
/// The first `max(field.stop_count, 1)` entries are colourmap stops or the
/// required empty-table padding. The remaining entries preserve one record per
/// declared contour level. Each 32-record block starts with its finite keys,
/// sorted by x; y is the original declaration index as a normal numeric f32.
/// Non-finite slots are zero padding and binding 6 carries the searchable prefix
/// length. Binding 2 remains the positional SSoT.
@group(2) @binding(3) var<storage, read> stops: array<vec4<f32>>;
@group(2) @binding(4) var<uniform> field: FieldParams;
/// Per-level contour stroke colour, **premultiplied**. One entry per level; the
/// fill never reads it.
@group(2) @binding(5) var<storage, read> level_colors: array<vec4<f32>>;

fn field_flag(bit: u32) -> bool {
    return (field.flags & bit) != 0u;
}

fn pool_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(field_pool[j], field_pool[j + 1u]);
}
