const ERRORBAR_HAS_Y: u32 = 1u;
const ERRORBAR_HAS_X: u32 = 2u;

struct DataPoint {
    x: vec2<f32>,
    y: vec2<f32>,
};

fn pair_value(v: vec2<f32>) -> f32 {
    return v.x + v.y;
}

fn pair_sum(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x + b.x, a.y + b.y);
}

fn pair_sub(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x - b.x, a.y - b.y);
}

fn point_ndc(p: DataPoint) -> vec2<f32> {
    return data_to_ndc(p.x, p.y);
}

fn select_point(b: DataPoint, a: DataPoint, pick_a: bool) -> DataPoint {
    return DataPoint(select(b.x, a.x, pick_a), select(b.y, a.y, pick_a));
}

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) x: vec2<f32>,
    @location(1) y: vec2<f32>,
    @location(2) err_y_lo: vec2<f32>,
    @location(3) err_y_hi: vec2<f32>,
    @location(4) err_x_lo: vec2<f32>,
    @location(5) err_x_hi: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    // segment: 0 Y-stem, 1 cap@y_lo, 2 cap@y_hi, 3 X-stem, 4 cap@x_lo,
    // 5 cap@x_hi.
    let seg = in.vi / 6u;

    // Disabled directions collapse at the anchor, caps included. Direction
    // presence is structural and must not depend on whether an error value is 0.
    let has_y = (style.primitive_flags & ERRORBAR_HAS_Y) != 0u;
    let has_x = (style.primitive_flags & ERRORBAR_HAS_X) != 0u;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(in.x, in.y);
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let y_lo = pair_sub(in.y, in.err_y_lo);
    let y_hi = pair_sum(in.y, in.err_y_hi);
    let x_lo = pair_sub(in.x, in.err_x_lo);
    let x_hi = pair_sum(in.x, in.err_x_hi);

    // Each segment is a quad between endpoints A and B: a data-space
    // position plus a pixel-space offset (caps run ±cap_half_px from a
    // single data point). `perp` is the pixel-space expansion axis; all
    // segments are axis-aligned, so no normalization is needed.
    var a_data: DataPoint;
    var b_data: DataPoint;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    if (seg == 0u) {
        // Y stem: vertical in data space, expand along X.
        a_data = DataPoint(in.x, y_lo);
        b_data = DataPoint(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 1u) {
        // Cap @ y_lo: horizontal, expand along Y.
        a_data = DataPoint(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 2u) {
        // Cap @ y_hi: horizontal, expand along Y.
        a_data = DataPoint(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 3u) {
        // X stem: horizontal in data space, expand along Y.
        a_data = DataPoint(x_lo, in.y);
        b_data = DataPoint(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 4u) {
        // Cap @ x_lo: vertical, expand along X.
        a_data = DataPoint(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    } else {
        // Cap @ x_hi: vertical, expand along X.
        a_data = DataPoint(x_hi, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    }

    // Triangle-list corner map [0,1,2, 2,1,3] over corners
    // {0: A-, 1: A+, 2: B-, 3: B+} (-/+ = perpendicular half-stroke side).
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let at_a = corner < 2u;
    let side = select(1.0, -1.0, (corner & 1u) == 0u);

    let end_data = select_point(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke);
    let ndc = point_ndc(end_data) + offset_px * transform.pixel_to_ndc;
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return style.color_premul;
}

// Per-point errorbar style mapping (NOT part of the common block). The mapped
// precise path adds a seventh per-instance f32 slot for
// error_bar_style_index_column and group 2 bindings 5..7 for style rows.
struct VsMappedIn {
    @builtin(vertex_index) vi: u32,
    @location(0) x: vec2<f32>,
    @location(1) y: vec2<f32>,
    @location(2) err_y_lo: vec2<f32>,
    @location(3) err_y_hi: vec2<f32>,
    @location(4) err_x_lo: vec2<f32>,
    @location(5) err_x_hi: vec2<f32>,
    @location(6) style_index: f32,
};

struct ErrorBarStyleSlot {
    color_premul: vec4<f32>,
    // x = stem width, y = cap half-size, z = mask, w = cap width.
    params: vec4<f32>,
};

struct ErrorBarStyleOverride {
    point_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ErrorBarStyleMapMeta {
    style_count: u32,
    override_count: u32,
    has_index: u32,
    _pad: u32,
};

@group(2) @binding(5) var<storage, read> errorbar_style_slots: array<ErrorBarStyleSlot>;
@group(2) @binding(6) var<storage, read> errorbar_style_overrides: array<ErrorBarStyleOverride>;
@group(2) @binding(7) var<uniform> errorbar_style_meta: ErrorBarStyleMapMeta;

struct ResolvedErrorBarStyle {
    color_premul: vec4<f32>,
    line_width_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
};

struct MappedOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color_premul: vec4<f32>,
};

const ERRORBAR_STYLE_MASK_COLOR: u32 = 1u;
const ERRORBAR_STYLE_MASK_STEM_WIDTH: u32 = 2u;
const ERRORBAR_STYLE_MASK_CAP_HALF: u32 = 4u;
const ERRORBAR_STYLE_MASK_CAP_WIDTH: u32 = 8u;

fn valid_errorbar_style_index(v: f32) -> bool {
    return v >= 0.0 && v <= 16777216.0 && abs(v - round(v)) <= 0.001;
}

fn apply_errorbar_style_slot(
    base: ResolvedErrorBarStyle,
    slot: ErrorBarStyleSlot,
) -> ResolvedErrorBarStyle {
    let mask = u32(slot.params.z);
    var out = base;
    if ((mask & ERRORBAR_STYLE_MASK_COLOR) != 0u) {
        out.color_premul = slot.color_premul;
    }
    if ((mask & ERRORBAR_STYLE_MASK_STEM_WIDTH) != 0u) {
        out.line_width_px = max(slot.params.x, 0.0);
    }
    if ((mask & ERRORBAR_STYLE_MASK_CAP_HALF) != 0u) {
        out.cap_half_px = max(slot.params.y, 0.0);
    }
    if ((mask & ERRORBAR_STYLE_MASK_CAP_WIDTH) != 0u) {
        out.cap_width_px = max(slot.params.w, 0.0);
    }
    return out;
}

fn resolve_errorbar_mapped_style(style_index: f32, inst: u32) -> ResolvedErrorBarStyle {
    var out = ResolvedErrorBarStyle(
        style.color_premul,
        style.line_width_px,
        style.cap_half_px,
        style.cap_width_px,
    );
    let point_index = inst + errorbar_style_meta._pad;

    if (errorbar_style_meta.has_index != 0u && valid_errorbar_style_index(style_index)) {
        let idx = u32(round(style_index));
        if (idx < errorbar_style_meta.style_count) {
            out = apply_errorbar_style_slot(out, errorbar_style_slots[idx]);
        }
    }

    for (var i = 0u; i < errorbar_style_meta.override_count; i = i + 1u) {
        let ov = errorbar_style_overrides[i];
        if (ov.point_index == point_index) {
            out = apply_errorbar_style_slot(out, ErrorBarStyleSlot(ov.color_premul, ov.params));
        }
    }
    return out;
}

@vertex
fn vs_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> MappedOut {
    let resolved = resolve_errorbar_mapped_style(in.style_index, inst);
    let seg = in.vi / 6u;

    var out: MappedOut;
    out.color_premul = resolved.color_premul;

    let has_y = (style.primitive_flags & ERRORBAR_HAS_Y) != 0u;
    let has_x = (style.primitive_flags & ERRORBAR_HAS_X) != 0u;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(in.x, in.y);
        out.pos = vec4<f32>(anchor, 0.0, 1.0);
        return out;
    }

    let y_lo = pair_sub(in.y, in.err_y_lo);
    let y_hi = pair_sum(in.y, in.err_y_hi);
    let x_lo = pair_sub(in.x, in.err_x_lo);
    let x_hi = pair_sum(in.x, in.err_x_hi);

    var a_data: DataPoint;
    var b_data: DataPoint;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    if (seg == 0u) {
        a_data = DataPoint(in.x, y_lo);
        b_data = DataPoint(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = resolved.line_width_px * 0.5;
    } else if (seg == 1u) {
        a_data = DataPoint(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-resolved.cap_half_px, 0.0);
        b_px = vec2<f32>( resolved.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = resolved.cap_width_px * 0.5;
    } else if (seg == 2u) {
        a_data = DataPoint(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-resolved.cap_half_px, 0.0);
        b_px = vec2<f32>( resolved.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = resolved.cap_width_px * 0.5;
    } else if (seg == 3u) {
        a_data = DataPoint(x_lo, in.y);
        b_data = DataPoint(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = resolved.line_width_px * 0.5;
    } else if (seg == 4u) {
        a_data = DataPoint(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -resolved.cap_half_px);
        b_px = vec2<f32>(0.0,  resolved.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = resolved.cap_width_px * 0.5;
    } else {
        a_data = DataPoint(x_hi, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -resolved.cap_half_px);
        b_px = vec2<f32>(0.0,  resolved.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = resolved.cap_width_px * 0.5;
    }

    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let at_a = corner < 2u;
    let side = select(1.0, -1.0, (corner & 1u) == 0u);

    let end_data = select_point(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke);
    let ndc = point_ndc(end_data) + offset_px * transform.pixel_to_ndc;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    return out;
}

@fragment
fn fs_mapped(in: MappedOut) -> @location(0) vec4<f32> {
    return in.color_premul;
}

// ──────────────── sketch mode (NOT part of the common block) ────────────────
// Hand-drawn entry point selected as a separate pipeline variant. The precise
// entries above never read the sketch Transform fields.
