// Columnar errorbar shader.
//
// Six columns (x, y, err_y_lo, err_y_hi, err_x_lo, err_x_hi) bound as
// per-instance slots. Each instance produces 36 vertices on a TriangleList:
// six axis-aligned quads (6 vertices each) — Y stem, cap @ y_lo, cap @ y_hi,
// X stem, cap @ x_lo, cap @ x_hi. Stem endpoints live in data space; caps
// span ±style.cap_half_px (pixels) perpendicular to their stem. Each quad is
// expanded perpendicular to its own axis by half its stroke width in pixels
// (stems: style.line_width_px, caps: style.cap_width_px); pixel offsets are
// converted to NDC via transform.pixel_to_ndc. The quads are the shared
// fragment `common/errorbar_segment.wgsl` (SHADER_COMMON.md §5.11), which
// the picker measures too.
//
// `style.primitive_flags` is the direction authority: bit 0 enables Y and
// bit 1 enables X. Inactive vertex slots may bind any sufficiently long
// column because the shader collapses that direction before reading errors.
//
// `scale_log.{x,y}` = 1.0 applies log10 to both the input coords and the
// computed err endpoints.

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) x: vec2<f32>,
    @location(1) y: vec2<f32>,
    @location(2) err_y_lo: vec2<f32>,
    @location(3) err_y_hi: vec2<f32>,
    @location(4) err_x_lo: vec2<f32>,
    @location(5) err_x_hi: vec2<f32>,
};

fn errorbar_sample(in: VsIn) -> ErrorBarSample {
    return ErrorBarSample(in.x, in.y, in.err_y_lo, in.err_y_hi, in.err_x_lo, in.err_x_hi);
}

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    // segment: 0 Y-stem, 1 cap@y_lo, 2 cap@y_hi, 3 X-stem, 4 cap@x_lo,
    // 5 cap@x_hi.
    let seg = in.vi / 6u;

    // Disabled directions collapse at the anchor, caps included.
    if (!errorbar_segment_enabled(style.primitive_flags, seg)) {
        let anchor = data_to_ndc(transform, in.x, in.y);
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let g = errorbar_segment(
        seg,
        errorbar_sample(in),
        errorbar_stroke(style.line_width_px, style.cap_half_px, style.cap_width_px),
    );
    let corner = errorbar_corner(in.vi);
    let ndc = errorbar_corner_ndc(transform, g, corner.at_a, corner.side * g.half_stroke);
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return style.color_premul;
}

// Per-point errorbar style mapping. The mapped
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

struct MappedOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color_premul: vec4<f32>,
};

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

    if (!errorbar_segment_enabled(style.primitive_flags, seg)) {
        let anchor = data_to_ndc(transform, in.x, in.y);
        out.pos = vec4<f32>(anchor, 0.0, 1.0);
        return out;
    }

    let g = errorbar_segment(
        seg,
        ErrorBarSample(in.x, in.y, in.err_y_lo, in.err_y_hi, in.err_x_lo, in.err_x_hi),
        errorbar_stroke(resolved.line_width_px, resolved.cap_half_px, resolved.cap_width_px),
    );
    let corner = errorbar_corner(in.vi);
    let ndc = errorbar_corner_ndc(transform, g, corner.at_a, corner.side * g.half_stroke);
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    return out;
}

@fragment
fn fs_mapped(in: MappedOut) -> @location(0) vec4<f32> {
    return in.color_premul;
}

// ──────────────── sketch mode ────────────────
// Hand-drawn entry point selected as a separate pipeline variant. The precise
// entries above never read the sketch Transform fields.

// The sketch errorbar uses the same six-quad construction as vs_main, but each
// quad end (A/B along the bar-length parameter) is displaced perpendicular to
// its stroke by `errorbar_sketch_offset`: amplitude · noise(2·seg + end,
// seed + instance_index). The integer lattice input samples the noise at
// lattice points, giving every one of the 12 quad ends an independent
// deterministic offset; both corners of an end share it, so strokes tilt
// without changing width. No subdivision — bars are short, corner
// perturbation suffices. A disabled direction stays collapsed to the
// anchor WITHOUT wobble (a displaced zero-area quad would gain area and
// rasterize).
@vertex
fn vs_sketch(in: VsIn, @builtin(instance_index) inst: u32) -> @builtin(position) vec4<f32> {
    // segment: 0 Y-stem, 1 cap@y_lo, 2 cap@y_hi, 3 X-stem, 4 cap@x_lo,
    // 5 cap@x_hi.
    let seg = in.vi / 6u;

    if (!errorbar_segment_enabled(style.primitive_flags, seg)) {
        let anchor = data_to_ndc(transform, in.x, in.y);
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let g = errorbar_segment(
        seg,
        errorbar_sample(in),
        errorbar_stroke(style.line_width_px, style.cap_half_px, style.cap_width_px),
    );
    let corner = errorbar_corner(in.vi);

    let amp = max(transform.style_params[0].x, 0.0);
    let seed = (u32(transform.style_params[0].z) ^ style.series_salt) + styled_point_index(inst);
    let disp = errorbar_sketch_offset(amp, seed, seg, corner.at_a);

    let ndc = errorbar_corner_ndc(transform, g, corner.at_a, corner.side * g.half_stroke + disp);
    return vec4<f32>(ndc, 0.0, 1.0);
}

// ────────────── Milkyway jets ───────────
// Bipolar-jet errorbars render the error range as a glowing astrophysical jet:
// stem quads become tapered beams brightest at the data point, and the cap
// quads become terminal shock KNOTS — diffuse radial glows that mark the
// exact interval bounds. Series color tints the plasma; the hot cores
// whiten. Rendered ADDITIVELY; the indicated range is the precise
// errorbar's, and the quads are `errorbar_jet_stroke`'s halo sizes
// (`JET_BEAM_HALO`, `JET_KNOT_HALO`).

struct JetOut {
    @builtin(position) pos: vec4<f32>,
    // Beams: x = cross-strip [-1,1], y = longitudinal t in [0,1] (A→B).
    // Knots: full square local coords [-1,1]².
    @location(0) local: vec2<f32>,
    // 0 = beam, 1 = knot.
    @location(1) @interpolate(flat) kind: u32,
    // Beam only: the data point's position along the bar (0..1) — the jet
    // source the brightness tapers away from.
    @location(2) @interpolate(flat) t_src: f32,
    @location(3) @interpolate(flat) seed_inst: u32,
};

@vertex
fn vs_jet(in: VsIn, @builtin(instance_index) inst: u32) -> JetOut {
    let seg = in.vi / 6u;

    var out: JetOut;
    out.seed_inst = (u32(transform.style_params[0].w) ^ style.series_salt) + styled_point_index(inst);
    out.kind = select(1u, 0u, seg == 0u || seg == 3u);
    out.t_src = 0.5;
    out.local = vec2<f32>(0.0, 0.0);

    if (!errorbar_segment_enabled(style.primitive_flags, seg)) {
        let anchor = data_to_ndc(transform, in.x, in.y);
        out.pos = vec4<f32>(anchor, 0.0, 1.0);
        return out;
    }

    let g = errorbar_segment(
        seg,
        errorbar_sample(in),
        errorbar_jet_stroke(style.line_width_px, style.cap_half_px),
    );
    if (seg == 0u) {
        out.t_src = pair_value(in.err_y_lo) / max(pair_value(in.err_y_lo) + pair_value(in.err_y_hi), 1e-6);
    } else if (seg == 3u) {
        out.t_src = pair_value(in.err_x_lo) / max(pair_value(in.err_x_lo) + pair_value(in.err_x_hi), 1e-6);
    }

    let corner = errorbar_corner(in.vi);
    let at_a = corner.at_a;
    let side = corner.side;
    let ndc = errorbar_corner_ndc(transform, g, at_a, side * g.half_stroke);

    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    // Beams: (cross side, longitudinal 0/1). Knots: square corner coords —
    // the along-axis corner sign doubles as the local x.
    let along = select(1.0, -1.0, at_a);
    out.local = select(
        vec2<f32>(along, side),
        vec2<f32>(side, select(1.0, 0.0, at_a)),
        out.kind == 0u,
    );
    return out;
}

@fragment
fn fs_jet(in: JetOut) -> @location(0) vec4<f32> {
    let tint = style.color_premul.rgb;
    var col = vec3<f32>(0.0);
    if (in.kind == 0u) {
        // Beam: gaussian cross-profile (FWHM ≈ the core stroke width inside
        // the 4× halo quad), tapering away from the jet source, with a soft
        // wisp modulation so the plasma reads organic.
        let cross = exp(-11.0 * in.local.x * in.local.x);
        let dn = abs(in.local.y - in.t_src) / max(max(in.t_src, 1.0 - in.t_src), 1e-3);
        let taper = mix(1.0, 0.30, clamp(dn, 0.0, 1.0));
        let wisp = 0.85 + 0.30 * sketch_noise(in.local.y * 6.0, in.seed_inst);
        let i = 0.6 * cross * taper * wisp;
        col = tint * i + vec3<f32>(1.0) * (0.18 * cross * cross * taper);
    } else {
        // Terminal shock knot: hot whitened core + tinted halo, clipped at
        // the quad edge.
        let r = length(in.local);
        let lim = 1.0 - smoothstep(0.85, 1.0, r);
        let core = exp(-5.5 * r * r);
        let halo = exp(-2.2 * r);
        col = (tint * (1.1 * core + 0.35 * halo) + vec3<f32>(0.95) * 0.55 * core * core) * lim;
    }
    let a = max(col.r, max(col.g, col.b));
    if (a <= 0.003) {
        discard;
    }
    return vec4<f32>(col, a);
}
