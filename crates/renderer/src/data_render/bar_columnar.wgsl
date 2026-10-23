@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(2) var<storage, read> data_pick_pool: array<f32>;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

var<workgroup> data_pick_shared: array<DataPickCandidate, DATA_PICK_WORKGROUP_SIZE>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn figgy_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        data_pick_shared[local_index] = DataPickCandidate();
        workgroupBarrier();
    }
}

fn data_pick_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

fn data_pick_pair(base: u32, index: u32) -> vec2<f32> {
    let lane = base + index * 2u;
    return vec2<f32>(data_pick_pool[lane], data_pick_pool[lane + 1u]);
}

fn data_pick_invalid() -> DataPickCandidate {
    return DataPickCandidate(
        0u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        0u,
        0u,
        0u,
        0.0,
        0u,
    );
}

// `DataPickCandidate.index2` of a bin painted by the envelope pass;
// `index1` then holds its value's order-preserving bits for the tie-break.
const DATA_PICK_ENVELOPE: u32 = 1u;

/// `f32` bits whose unsigned order is the float order.
fn data_pick_ordered_bits(v: f32) -> u32 {
    let bits = bitcast<u32>(v);
    return select(bits | 0x80000000u, ~bits, (bits & 0x80000000u) != 0u);
}

fn data_pick_better(candidate: DataPickCandidate, incumbent: DataPickCandidate) -> bool {
    if (candidate.valid == 0u) {
        return false;
    }
    if (incumbent.valid == 0u) {
        return true;
    }
    if (candidate.distance_px != incumbent.distance_px) {
        return candidate.distance_px < incumbent.distance_px;
    }
    // The envelope pass paints over the full bars.
    let candidate_envelope = candidate.index2 == DATA_PICK_ENVELOPE;
    let incumbent_envelope = incumbent.index2 == DATA_PICK_ENVELOPE;
    if (candidate_envelope != incumbent_envelope) {
        return candidate_envelope;
    }
    // A pixel column paints its envelope winner: the largest value, the
    // earliest bin on a tie (`envelope_better`, by the value's high part).
    if (candidate_envelope) {
        if (candidate.index1 != incumbent.index1) {
            return candidate.index1 > incumbent.index1;
        }
        return candidate.primitive_order < incumbent.primitive_order;
    }
    // Later bars are painted later when malformed/reversed bins overlap.
    return candidate.primitive_order > incumbent.primitive_order;
}

fn data_pick_bar(bin_index: u32) -> DataPickCandidate {
    let edge_lo = data_pick_pair(data_pick_query.bases.x, bin_index);
    let edge_hi = data_pick_pair(data_pick_query.bases.x, bin_index + 1u);
    let value = data_pick_pair(data_pick_query.bases.y, bin_index);
    if (!all(vec4<bool>(
        data_pick_finite(edge_lo.x),
        data_pick_finite(edge_lo.y),
        data_pick_finite(edge_hi.x),
        data_pick_finite(edge_hi.y),
    )) || !data_pick_finite(value.x) || !data_pick_finite(value.y)) {
        return data_pick_invalid();
    }
    let resolved = resolve_bar_style(
        ResolvedBarStyle(
            vec4<f32>(0.0),
            vec4<f32>(0.0),
            0.0,
            data_pick_query.limits.y,
            data_pick_query.limits.z,
        ),
        bin_index + data_pick_query.bases.z,
    );
    let horizontal = data_pick_query.data.w == BAR_ORIENT_HORIZONTAL;
    // A subpixel bin paints the envelope's pixel columns, baseline to value
    // (`bar_envelope_columns`); any other bin paints its own bar.
    let columns = bar_envelope_columns(
        edge_lo, edge_hi, value, horizontal, resolved.width_ratio, bar_panel_columns(horizontal),
    );
    let envelope = columns.y > columns.x;
    var outer = bar_outer_ndc_values(
        edge_lo,
        edge_hi,
        value,
        data_pick_query.baseline.xy,
        horizontal,
        resolved.gap_px,
        resolved.width_ratio,
    );
    if (envelope) {
        outer = bar_envelope_rect(columns, value, data_pick_query.baseline.xy, horizontal);
    }
    if (!all(vec4<bool>(
        data_pick_finite(outer.x),
        data_pick_finite(outer.y),
        data_pick_finite(outer.z),
        data_pick_finite(outer.w),
    )) || !(outer.z > outer.x) || !(outer.w > outer.y)) {
        return data_pick_invalid();
    }
    let cursor = data_pick_query.cursor_ndc_t.xy;
    let outside_ndc = max(max(outer.xy - cursor, cursor - outer.zw), vec2<f32>(0.0));
    let outside_px = outside_ndc / transform.pixel_to_ndc;
    let distance_px = length(outside_px);
    if (!data_pick_finite(distance_px)
        || !data_pick_finite(data_pick_query.limits.x)
        || distance_px > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        bin_index + data_pick_query.bases.z,
        select(0u, data_pick_ordered_bits(value.x), envelope),
        select(0u, DATA_PICK_ENVELOPE, envelope),
        distance_px,
        bin_index + data_pick_query.bases.z,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_histogram_bin(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var index = local_index;
    while (index < data_pick_query.data.z) {
        let candidate = data_pick_bar(index);
        if (data_pick_better(candidate, best)) {
            best = candidate;
        }
        index = index + DATA_PICK_WORKGROUP_SIZE;
    }
    data_pick_shared[local_index] = best;
    workgroupBarrier();
    for (var stride = DATA_PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if (local_index < stride) {
            let candidate = data_pick_shared[local_index + stride];
            if (data_pick_better(candidate, data_pick_shared[local_index])) {
                data_pick_shared[local_index] = candidate;
            }
        }
        workgroupBarrier();
    }
    if (local_index == 0u) {
        data_pick_output = data_pick_shared[0];
    }
}

// ───── Typed selection overlay (render entry) ───────────────────────────────

@group(2) @binding(4) var<uniform> data_selection: DataSelection;

/// Four outline quads around the exact rendered bar outer rectangle. The
/// caller draws only the selected bin instance, so no coordinate or rectangle
/// is stored in selection state.
@vertex
fn vs_bar_selection(in: VsIn) -> VsOut {
    let horizontal = style.shape_id == BAR_ORIENT_HORIZONTAL;
    var outer = bar_outer_ndc_values(
        in.edge_lo,
        in.edge_hi,
        in.value,
        style.dash[1].xy,
        horizontal,
        style.cap_half_px,
        style.cap_width_px,
    );
    // A subpixel bin is painted by the envelope pass: outline what it paints.
    let columns = bar_envelope_columns(
        in.edge_lo, in.edge_hi, in.value, horizontal, style.cap_width_px, bar_panel_columns(horizontal),
    );
    if (columns.y > columns.x) {
        outer = bar_envelope_rect(columns, in.value, style.dash[1].xy, horizontal);
    }
    var out: VsOut;
    out.color_premul = data_selection.color_premul;
    if (!(outer.z > outer.x) || !(outer.w > outer.y)) {
        out.pos = vec4<f32>(outer.xy, 0.0, 1.0);
        return out;
    }
    let border = max(data_selection.metrics.x, 0.0) * transform.pixel_to_ndc;
    let inset = min(border, (outer.zw - outer.xy) * 0.5);
    let inner = vec4<f32>(outer.xy + inset, outer.zw - inset);
    let segment = in.vi / 6u + 1u;
    let rect = bar_segment_rect(segment, outer, inner);
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let x = select(rect.x, rect.z, (corner & 1u) != 0u);
    let y = select(rect.y, rect.w, corner >= 2u);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}
