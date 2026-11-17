struct Transform {
    float2 data_min;
    float2 data_max;
    float2 data_min_lo;
    float2 data_max_lo;
    float2 scale_log;
    float2 pixel_to_ndc;
    float2 data_to_panel_scale;
    float2 data_to_panel_offset;
    float4 style_params[3];
    float4 polar_angle;
    float4 polar_extent;
    float4 ternary_lt;
    float4 ternary_r;
    float4 breaks[6];
    float4 break_info;
    float4 scale_prob;
};

struct Style {
    float4 color_premul;
    float line_width_px;
    float point_radius_px;
    float cap_half_px;
    float cap_width_px;
    uint shape_id;
    uint dash_len;
    uint series_salt;
    uint primitive_flags;
    float4 dash[2];
};

struct AxisCuts {
    float2 count_scale;
};

struct PolarUnproject {
    float angle;
    float ty;
    float2 d_angle;
    float2 d_ty;
};

struct ArrowStyle {
    float width_px;
    float head_length_px;
    float head_width_px;
    float min_length_px;
    uint pivot;
};

struct ArrowPlacement {
    float2 tail;
    float2 tip;
    float head_length_px;
    float head_width_px;
    bool valid;
    int _end_pad_0;
};

struct VectorSample {
    float3 origin_hi;
    int _pad1_0;
    float3 origin_lo;
    int _pad2_0;
    float3 vector_;
    int _pad3_0;
    float2 value;
    bool value_valid;
    bool valid;
    int _end_pad_0;
};

struct ArrowPx {
    float2 tail;
    float2 tip;
    float2 depth;
    bool valid;
    int _end_pad_0;
};

struct GridColumn {
    uint base;
    uint len;
};

struct VectorParams {
    uint x_base;
    uint y_base;
    uint x_len;
    uint y_len;
    uint u_base;
    uint u_len;
    uint v_base;
    uint v_len;
    uint c_base;
    uint c_len;
    uint cols;
    uint rows;
    uint flags;
    uint count;
    uint stop_count;
    uint pivot;
    float scale;
    float width_px;
    float head_length_px;
    float head_width_px;
    float min_length_px;
    float aa_px;
    float spacing_px;
    float pad1_;
    uint height_base;
    uint height_len;
    uint w_base;
    uint w_len;
    float2 z_min;
    float2 z_max;
    float4 nan_color_premul;
};

struct VectorVsOut {
    float4 pos : SV_Position;
    float2 px : LOC0;
    nointerpolation float2 tail : LOC1;
    nointerpolation float2 tip : LOC2;
    nointerpolation float2 head : LOC3;
    nointerpolation float4 color : LOC4;
};

static const uint ARROW_VERTICES = 6u;
static const uint ARROW_PIVOT_TAIL = 0u;
static const uint ARROW_PIVOT_MIDDLE = 1u;
static const uint ARROW_PIVOT_TIP = 2u;
static const float ARROW_SHAFT_MIN_PX = 0.01;
static const uint VECTOR_SCALE_DATA = 0u;
static const uint VECTOR_SCALE_PIXELS = 1u;
static const uint VECTOR_FLAG_GRID = 1u;
static const uint VECTOR_FLAG_COLUMNS_ARE_Y = 2u;
static const uint VECTOR_FLAG_CENTERS = 4u;
static const uint VECTOR_FLAG_SCALE_PIXELS = 8u;
static const uint VECTOR_FLAG_LOG_Z = 16u;
static const uint VECTOR_FLAG_COLOR_MAP = 32u;
static const uint VECTOR_FLAG_THREE_D = 64u;

cbuffer transform : register(b0) { Transform transform; }
cbuffer style_1 : register(b0, space1) { Style style_1; }
ByteAddressBuffer vector_pool : register(t0, space2);
ByteAddressBuffer vector_grid : register(t1, space2);
ByteAddressBuffer vector_stops : register(t2, space2);
cbuffer vector_params : register(b3, space2) { VectorParams vector_params; }

struct VertexOutput_vs_main {
    float2 px_3 : LOC0;
    nointerpolation float2 tail_1 : LOC1;
    nointerpolation float2 tip_2 : LOC2;
    nointerpolation float2 head : LOC3;
    nointerpolation float4 color : LOC4;
    float4 pos : SV_Position;
};

struct FragmentInput_fs_main {
    float2 px_4 : LOC0;
    nointerpolation float2 tail_2 : LOC1;
    nointerpolation float2 tip_3 : LOC2;
    nointerpolation float2 head_1 : LOC3;
    nointerpolation float4 color_1 : LOC4;
    float4 pos_1 : SV_Position;
};

uint styled_point_index(uint local_index)
{
    float _e5 = transform.style_params[2].z;
    return (asuint(_e5) + local_index);
}

AxisCuts ConstructAxisCuts(float2 arg0) {
    AxisCuts ret = (AxisCuts)0;
    ret.count_scale = arg0;
    return ret;
}

AxisCuts no_cuts()
{
    const AxisCuts axiscuts = ConstructAxisCuts(float2(0.0, 1.0));
    return axiscuts;
}

AxisCuts cuts_of(float4 breaks[6], float4 info, uint axis)
{
    const AxisCuts _e3 = no_cuts();
    const AxisCuts axiscuts_1 = _e3;
    return axiscuts_1;
}

float cut_remap(AxisCuts c, float t)
{
    return t;
}

float cut_unmap(AxisCuts c_1, float drawn)
{
    return drawn;
}

float cut_scale_at(AxisCuts c_2, float t_1)
{
    return 1.0;
}

bool prob_in_domain(float2 pair)
{
    return false;
}

float prob_forward(float2 pair_1, float kind)
{
    return 0.0;
}

float prob_slope(float2 pair_2, float kind_1)
{
    return 1.0;
}

float2 prob_inverse(float y, float kind_2)
{
    return float2(y, 0.0);
}

float maybe_log(float v, float is_log)
{
    float lv = (log(max(v, 1e-30)) / 2.3025851);
    if ((is_log > 0.5)) {
        return lv;
    }
    return lerp(v, lv, is_log);
}

float axis_data_t(float2 v_1, float min_hi, float max_hi, float min_lo, float max_lo, float is_log_1, float prob)
{
    float range_1 = mad(1.0, (max_hi - min_hi), (max_lo - min_lo));
    float raw = (v_1.x + v_1.y);
    float linear_num = mad(1.0, (v_1.x - min_hi), (v_1.y - min_lo));
    const float _e20 = maybe_log(raw, is_log_1);
    float log_num = ((_e20 - min_hi) - min_lo);
    if ((is_log_1 > 0.5)) {
        return (log_num / range_1);
    }
    return lerp((linear_num / range_1), (log_num / range_1), is_log_1);
}

float axis_pair_to_t(float2 v_2, float min_hi_1, float max_hi_1, float min_lo_1, float max_lo_1, float is_log_2, float prob_1, float panel_scale, float panel_offset, AxisCuts cuts)
{
    const float _e10 = axis_data_t(v_2, min_hi_1, max_hi_1, min_lo_1, max_lo_1, is_log_2, prob_1);
    return mad(_e10, panel_scale, panel_offset);
}

float2 polar_ndc(Transform t_2, float ty, float2 xv)
{
    return ((abs(ty) + 2.0)).xx;
}

PolarUnproject ConstructPolarUnproject(float arg0, float arg1, float2 arg2, float2 arg3) {
    PolarUnproject ret = (PolarUnproject)0;
    ret.angle = arg0;
    ret.ty = arg1;
    ret.d_angle = arg2;
    ret.d_ty = arg3;
    return ret;
}

PolarUnproject polar_unproject(Transform t_3, float2 px)
{
    const PolarUnproject polarunproject = ConstructPolarUnproject(0.0, 0.0, float2(0.0, 0.0), float2(0.0, 0.0));
    return polarunproject;
}

bool polar_sample_valid(Transform t_4, float2 xv_1, float2 yv)
{
    return (t_4.polar_angle.x < 0.5);
}

bool polar_fragment_visible(float2 px_1, float4 limits, float4 sector)
{
    return (limits.x < 0.5);
}

float2 data_to_ndc(Transform t_5, float2 xv_2, float2 yv_1)
{
    const AxisCuts _e22 = cuts_of(t_5.breaks, t_5.break_info, 0u);
    const float _e23 = axis_pair_to_t(xv_2, t_5.data_min.x, t_5.data_max.x, t_5.data_min_lo.x, t_5.data_max_lo.x, t_5.scale_log.x, t_5.scale_prob.x, t_5.data_to_panel_scale.x, t_5.data_to_panel_offset.x, _e22);
    const AxisCuts _e43 = cuts_of(t_5.breaks, t_5.break_info, 1u);
    const float _e44 = axis_pair_to_t(yv_1, t_5.data_min.y, t_5.data_max.y, t_5.data_min_lo.y, t_5.data_max_lo.y, t_5.scale_log.y, t_5.scale_prob.y, t_5.data_to_panel_scale.y, t_5.data_to_panel_offset.y, _e43);
    if ((t_5.polar_angle.x > 0.5)) {
        const float2 _e49 = polar_ndc(t_5, _e44, xv_2);
        return _e49;
    }
    return ((float2(_e23, _e44) * 2.0) - (1.0).xx);
}

float2 ternary_ndc(Transform t_6, float2 xv_3, float2 yv_2, float2 cv)
{
    float x = (xv_3.x + xv_3.y);
    float y_1 = (yv_2.x + yv_2.y);
    float c_3 = (cv.x + cv.y);
    return ((((t_6.ternary_lt.xy * x) + (t_6.ternary_lt.zw * y_1)) + (t_6.ternary_r.xy * c_3)) / (((x + y_1) + c_3)).xx);
}

float axis_baseline_ndc(Transform t_7, float2 base, uint axis_1)
{
    bool local = (bool)0;
    bool local_1 = (bool)0;
    bool local_2 = (bool)0;

    float value = (base.x + base.y);
    float2 one = float2(1.0, 0.0);
    if ((t_7.scale_prob[min(uint(axis_1), 3u)] > 0.5)) {
        const bool _e15 = prob_in_domain(base);
        local = !(_e15);
    } else {
        local = false;
    }
    bool _e18 = local;
    if (_e18) {
        float range_2 = ((t_7.data_max[min(uint(axis_1), 1u)] - t_7.data_min[min(uint(axis_1), 1u)]) + (t_7.data_max_lo[min(uint(axis_1), 1u)] - t_7.data_min_lo[min(uint(axis_1), 1u)]));
        bool low = (value < 0.5);
        float data_t = ((low == (range_2 > 0.0)) ? 0.0 : 1.0);
        return ((mad(data_t, t_7.data_to_panel_scale[min(uint(axis_1), 1u)], t_7.data_to_panel_offset[min(uint(axis_1), 1u)]) * 2.0) - 1.0);
    }
    if ((axis_1 == 0u)) {
        if ((t_7.scale_log.x != 0.0)) {
            local_1 = !((value > 0.0));
        } else {
            local_1 = false;
        }
        bool _e59 = local_1;
        if (_e59) {
            return ((t_7.data_to_panel_offset.x * 2.0) - 1.0);
        }
        const float2 _e66 = data_to_ndc(t_7, base, one);
        return _e66.x;
    }
    if ((t_7.scale_log.y != 0.0)) {
        local_2 = !((value > 0.0));
    } else {
        local_2 = false;
    }
    bool _e78 = local_2;
    if (_e78) {
        return ((t_7.data_to_panel_offset.y * 2.0) - 1.0);
    }
    const float2 _e85 = data_to_ndc(t_7, one, base);
    return _e85.y;
}

float2 data_to_px(Transform t_8, float2 xv_4, float2 yv_3)
{
    const float2 _e3 = data_to_ndc(t_8, xv_4, yv_3);
    return (_e3 / t_8.pixel_to_ndc);
}

bool sample_finite(float v_3)
{
    return ((asuint(v_3) & 2139095040u) != 2139095040u);
}

bool ternary_sample_valid(float2 xv_5, float2 yv_4, float2 cv_1)
{
    bool local_3 = (bool)0;
    bool local_4 = (bool)0;
    bool local_5 = (bool)0;
    bool local_6 = (bool)0;
    bool local_7 = (bool)0;
    bool local_8 = (bool)0;
    bool local_9 = (bool)0;
    bool local_10 = (bool)0;
    bool local_11 = (bool)0;
    bool local_12 = (bool)0;
    bool local_13 = (bool)0;
    bool local_14 = (bool)0;

    float x_1 = (xv_5.x + xv_5.y);
    float y_2 = (yv_4.x + yv_4.y);
    float c_4 = (cv_1.x + cv_1.y);
    const bool _e13 = sample_finite(xv_5.x);
    if (_e13) {
        const bool _e17 = sample_finite(xv_5.y);
        local_3 = _e17;
    } else {
        local_3 = false;
    }
    bool _e19 = local_3;
    if (_e19) {
        const bool _e23 = sample_finite(yv_4.x);
        local_4 = _e23;
    } else {
        local_4 = false;
    }
    bool _e25 = local_4;
    if (_e25) {
        const bool _e29 = sample_finite(yv_4.y);
        local_5 = _e29;
    } else {
        local_5 = false;
    }
    bool _e31 = local_5;
    if (_e31) {
        const bool _e35 = sample_finite(cv_1.x);
        local_6 = _e35;
    } else {
        local_6 = false;
    }
    bool _e37 = local_6;
    if (_e37) {
        const bool _e41 = sample_finite(cv_1.y);
        local_7 = _e41;
    } else {
        local_7 = false;
    }
    bool _e43 = local_7;
    if (_e43) {
        const bool _e46 = sample_finite(x_1);
        local_8 = _e46;
    } else {
        local_8 = false;
    }
    bool _e48 = local_8;
    if (_e48) {
        const bool _e51 = sample_finite(y_2);
        local_9 = _e51;
    } else {
        local_9 = false;
    }
    bool _e53 = local_9;
    if (_e53) {
        const bool _e56 = sample_finite(c_4);
        local_10 = _e56;
    } else {
        local_10 = false;
    }
    bool _e58 = local_10;
    if (_e58) {
        local_11 = (x_1 >= 0.0);
    } else {
        local_11 = false;
    }
    bool _e64 = local_11;
    if (_e64) {
        local_12 = (y_2 >= 0.0);
    } else {
        local_12 = false;
    }
    bool _e70 = local_12;
    if (_e70) {
        local_13 = (c_4 >= 0.0);
    } else {
        local_13 = false;
    }
    bool _e76 = local_13;
    if (_e76) {
        local_14 = (((x_1 + y_2) + c_4) > 0.0);
    } else {
        local_14 = false;
    }
    bool _e84 = local_14;
    return _e84;
}

bool sample_valid(Transform t_9, float2 xv_6, float2 yv_5)
{
    bool local_15 = (bool)0;
    bool local_16 = (bool)0;
    bool local_17 = (bool)0;
    bool local_18 = (bool)0;
    bool local_19 = (bool)0;
    bool local_20 = (bool)0;
    bool local_21 = (bool)0;
    bool local_22 = (bool)0;
    bool local_23 = (bool)0;
    bool local_24 = (bool)0;
    bool local_25 = (bool)0;
    bool local_26 = (bool)0;
    bool local_27 = (bool)0;
    bool local_28 = (bool)0;

    float x_2 = (xv_6.x + xv_6.y);
    float y_3 = (yv_5.x + yv_5.y);
    const bool _e10 = sample_finite(xv_6.x);
    if (_e10) {
        const bool _e14 = sample_finite(xv_6.y);
        local_15 = _e14;
    } else {
        local_15 = false;
    }
    bool _e16 = local_15;
    if (_e16) {
        const bool _e20 = sample_finite(yv_5.x);
        local_16 = _e20;
    } else {
        local_16 = false;
    }
    bool _e22 = local_16;
    if (_e22) {
        const bool _e26 = sample_finite(yv_5.y);
        local_17 = _e26;
    } else {
        local_17 = false;
    }
    bool _e28 = local_17;
    if (_e28) {
        const bool _e31 = sample_finite(x_2);
        local_18 = _e31;
    } else {
        local_18 = false;
    }
    bool _e33 = local_18;
    if (_e33) {
        const bool _e36 = sample_finite(y_3);
        local_19 = _e36;
    } else {
        local_19 = false;
    }
    bool _e38 = local_19;
    if (_e38) {
        if (!((t_9.scale_log.x == 0.0))) {
            local_21 = (x_2 > 0.0);
        } else {
            local_21 = true;
        }
        bool _e51 = local_21;
        local_20 = _e51;
    } else {
        local_20 = false;
    }
    bool _e53 = local_20;
    if (_e53) {
        if (!((t_9.scale_log.y == 0.0))) {
            local_23 = (y_3 > 0.0);
        } else {
            local_23 = true;
        }
        bool _e66 = local_23;
        local_22 = _e66;
    } else {
        local_22 = false;
    }
    bool _e68 = local_22;
    if (_e68) {
        if (!((t_9.scale_prob.x < 0.5))) {
            const bool _e78 = prob_in_domain(xv_6);
            local_25 = _e78;
        } else {
            local_25 = true;
        }
        bool _e80 = local_25;
        local_24 = _e80;
    } else {
        local_24 = false;
    }
    bool _e82 = local_24;
    if (_e82) {
        if (!((t_9.scale_prob.y < 0.5))) {
            const bool _e92 = prob_in_domain(yv_5);
            local_27 = _e92;
        } else {
            local_27 = true;
        }
        bool _e94 = local_27;
        local_26 = _e94;
    } else {
        local_26 = false;
    }
    bool _e96 = local_26;
    if (_e96) {
        const bool _e99 = polar_sample_valid(t_9, xv_6, yv_5);
        local_28 = _e99;
    } else {
        local_28 = false;
    }
    bool _e101 = local_28;
    return _e101;
}

float2 polar_clip_offset(Transform t_10, float2 ndc)
{
    float2 center_ = (((t_10.data_to_panel_offset + (t_10.data_to_panel_scale * 0.5)) * 2.0) - (1.0).xx);
    return ((ndc - center_) / t_10.pixel_to_ndc);
}

float4 polar_clip_limits(Transform t_11)
{
    float4 p_4 = t_11.polar_angle;
    float4 e = t_11.polar_extent;
    return float4(p_4.x, e.w, (e.w * p_4.w), ((e.y > e.x) ? 1.0 : 0.0));
}

float4 polar_clip_sector(Transform t_12)
{
    float4 p_5 = t_12.polar_angle;
    float4 e_1 = t_12.polar_extent;
    return float4(p_5.y, p_5.z, e_1.x, e_1.y);
}

bool ramp_finite(float v_4)
{
    return ((asuint(v_4) & 2139095040u) != 2139095040u);
}

float ramp_t(float2 zv, float2 z_min, float2 z_max, bool log_z)
{
    bool local_29 = (bool)0;
    bool local_30 = (bool)0;
    bool local_31 = (bool)0;
    bool local_32 = (bool)0;
    bool local_33 = (bool)0;
    float numerator = (float)0;
    bool local_34 = (bool)0;

    const bool _e5 = ramp_finite(zv.x);
    if (!(!(_e5))) {
        const bool _e11 = ramp_finite(zv.y);
        local_29 = !(_e11);
    } else {
        local_29 = true;
    }
    bool _e14 = local_29;
    if (_e14) {
        return -1.0;
    }
    float raw_1 = (zv.x + zv.y);
    const bool _e19 = ramp_finite(raw_1);
    if (!(_e19)) {
        return -1.0;
    }
    if (log_z) {
        local_30 = (raw_1 <= 0.0);
    } else {
        local_30 = false;
    }
    bool _e27 = local_30;
    if (_e27) {
        return -1.0;
    }
    float is_log_4 = (log_z ? 1.0 : 0.0);
    float linear_num_1 = ((zv.x - z_min.x) + (zv.y - z_min.y));
    const float _e40 = maybe_log(raw_1, 1.0);
    float log_num_1 = ((_e40 - z_min.x) - z_min.y);
    float span = ((z_max.x - z_min.x) + (z_max.y - z_min.y));
    const bool _e52 = ramp_finite(linear_num_1);
    if (!(!(_e52))) {
        const bool _e57 = ramp_finite(log_num_1);
        local_31 = !(_e57);
    } else {
        local_31 = true;
    }
    bool _e60 = local_31;
    if (!(_e60)) {
        const bool _e64 = ramp_finite(span);
        local_32 = !(_e64);
    } else {
        local_32 = true;
    }
    bool _e67 = local_32;
    if (!(_e67)) {
        local_33 = (span <= 0.0);
    } else {
        local_33 = true;
    }
    bool _e74 = local_33;
    if (_e74) {
        return -1.0;
    }
    numerator = lerp(linear_num_1, log_num_1, is_log_4);
    if ((is_log_4 > 0.5)) {
        numerator = log_num_1;
    }
    float _e80 = numerator;
    float normalized = (_e80 / span);
    float _e82 = numerator;
    const bool _e83 = ramp_finite(_e82);
    if (!(!(_e83))) {
        const bool _e88 = ramp_finite(normalized);
        local_34 = !(_e88);
    } else {
        local_34 = true;
    }
    bool _e91 = local_34;
    if (_e91) {
        return -1.0;
    }
    return clamp(normalized, 0.0, 1.0);
}

uint naga_f2u32(float value) {
    return uint(clamp(value, 0.0, 4294967000.0));
}

float2 ramp_segment(float t_13, uint n)
{
    const bool _e5 = ramp_finite(t_13);
    float tc = (!(_e5) ? 0.0 : clamp(t_13, 0.0, 1.0));
    float scaled = (tc * float((n - 1u)));
    uint index = min(naga_f2u32(floor(scaled)), (n - 2u));
    return float2(float(index), (scaled - float(index)));
}

float4 ramp_blend(float4 a, float4 b, float frac_)
{
    if ((frac_ <= 0.0)) {
        return a;
    }
    if ((frac_ >= 1.0)) {
        return b;
    }
    return (a + ((b - a) * frac_));
}

float grid_rounded_add(float a_1, float b_1)
{
    return mad(1.0, a_1, b_1);
}

float grid_rounded_subtract(float a_2, float b_2)
{
    return mad(1.0, a_2, -(b_2));
}

float2 normalize_grid_pair(float2 v_5)
{
    const float _e3 = grid_rounded_add(v_5.x, v_5.y);
    const float _e5 = grid_rounded_subtract(_e3, v_5.x);
    const float _e6 = grid_rounded_subtract(_e3, _e5);
    const float _e8 = grid_rounded_subtract(v_5.y, _e5);
    const float _e10 = grid_rounded_subtract(v_5.x, _e6);
    const float _e11 = grid_rounded_add(_e10, _e8);
    return float2(_e3, _e11);
}

float2 pair_add(float2 pair_3, float s)
{
    const float2 _e4 = normalize_grid_pair(float2(pair_3.x, s));
    return float2(_e4.x, (_e4.y + pair_3.y));
}

float2 add_grid_pairs(float2 a_3, float2 b_3)
{
    const float2 _e5 = normalize_grid_pair(float2(a_3.x, b_3.x));
    const float _e8 = grid_rounded_add(a_3.y, b_3.y);
    const float _e10 = grid_rounded_add(_e8, _e5.y);
    const float2 _e13 = normalize_grid_pair(float2(_e5.x, _e10));
    return _e13;
}

float2 subtract_grid_pairs(float2 a_4, float2 b_4)
{
    const float2 _e3 = add_grid_pairs(a_4, -(b_4));
    return _e3;
}

float2 scale_grid_pair(float2 v_6, float factor)
{
    const float2 _e7 = normalize_grid_pair(float2((v_6.x * factor), (v_6.y * factor)));
    return _e7;
}

float2 midpoint_grid_pair(float2 a_5, float2 b_5)
{
    const float2 _e3 = scale_grid_pair(a_5, 0.5);
    const float2 _e5 = scale_grid_pair(b_5, 0.5);
    const float2 _e6 = add_grid_pairs(_e3, _e5);
    return _e6;
}

float pair_value(float2 v_7)
{
    return (v_7.x + v_7.y);
}

bool arrow_finite(float v_8)
{
    return ((asuint(v_8) & 2139095040u) != 2139095040u);
}

ArrowPlacement arrow_place(float2 tail, float2 tip, ArrowStyle style)
{
    ArrowPlacement out_ = (ArrowPlacement)0;
    bool local_35 = (bool)0;
    bool local_36 = (bool)0;
    float shift = 0.0;

    out_.tail = tail;
    out_.tip = tip;
    out_.head_length_px = 0.0;
    out_.head_width_px = 0.0;
    out_.valid = false;
    float2 delta = (tip - tail);
    float len = length(delta);
    const bool _e14 = arrow_finite(len);
    if (!(!(_e14))) {
        local_35 = (len <= 0.0);
    } else {
        local_35 = true;
    }
    bool _e22 = local_35;
    if (!(_e22)) {
        local_36 = (len < style.min_length_px);
    } else {
        local_36 = true;
    }
    bool _e29 = local_36;
    if (_e29) {
        ArrowPlacement _e30 = out_;
        const ArrowPlacement arrowplacement = _e30;
        return arrowplacement;
    }
    if ((style.pivot == ARROW_PIVOT_MIDDLE)) {
        shift = -0.5;
    } else {
        if ((style.pivot == ARROW_PIVOT_TIP)) {
            shift = -1.0;
        }
    }
    float _e42 = shift;
    out_.tail = (tail + (delta * _e42));
    float _e46 = shift;
    out_.tip = (tip + (delta * _e46));
    float head_length = min(max(style.head_length_px, 0.0), len);
    out_.head_length_px = head_length;
    if ((style.head_length_px > 0.0)) {
        out_.head_width_px = (max(style.head_width_px, 0.0) * (head_length / style.head_length_px));
    }
    out_.valid = true;
    ArrowPlacement _e66 = out_;
    const ArrowPlacement arrowplacement_1 = _e66;
    return arrowplacement_1;
}

float2 arrow_quad_corner(uint vertex_index_1, ArrowPlacement placed_1, float width_px, float margin_px)
{
    float cx = -1.0;
    float cy = -1.0;

    float2 delta_1 = (placed_1.tip - placed_1.tail);
    float len_1 = length(delta_1);
    float2 dir = (delta_1 / (max(len_1, 1e-12)).xx);
    float2 nrm = float2(-(dir.y), dir.x);
    float2 center_1 = ((placed_1.tail + placed_1.tip) * 0.5);
    float half_len = (((len_1 * 0.5) + (max(width_px, 0.0) * 0.5)) + margin_px);
    float half_wid = (max((width_px * 0.5), (placed_1.head_width_px * 0.5)) + margin_px);
    switch(vertex_index_1) {
        case 1u: {
            cx = 1.0;
            cy = -1.0;
            break;
        }
        case 2u:
        case 4u: {
            cx = 1.0;
            cy = 1.0;
            break;
        }
        case 5u: {
            cx = -1.0;
            cy = 1.0;
            break;
        }
        default: {
            cx = -1.0;
            cy = -1.0;
            break;
        }
    }
    float _e48 = cx;
    float _e52 = cy;
    return ((center_1 + (dir * (_e48 * half_len))) + (nrm * (_e52 * half_wid)));
}

float arrow_sd_segment(float2 p, float2 a_6, float2 b_6)
{
    float2 pa = (p - a_6);
    float2 ba = (b_6 - a_6);
    float h = clamp((dot(pa, ba) / max(dot(ba, ba), 1e-12)), 0.0, 1.0);
    return length((pa - (ba * h)));
}

float arrow_sd_triangle(float2 p_1, float2 p0_, float2 p1_, float2 p2_)
{
    float2 e0_ = (p1_ - p0_);
    float2 e1_ = (p2_ - p1_);
    float2 e2_ = (p0_ - p2_);
    float2 v0_ = (p_1 - p0_);
    float2 v1_ = (p_1 - p1_);
    float2 v2_ = (p_1 - p2_);
    float2 pq0_ = (v0_ - (e0_ * clamp((dot(v0_, e0_) / max(dot(e0_, e0_), 1e-12)), 0.0, 1.0)));
    float2 pq1_ = (v1_ - (e1_ * clamp((dot(v1_, e1_) / max(dot(e1_, e1_), 1e-12)), 0.0, 1.0)));
    float2 pq2_ = (v2_ - (e2_ * clamp((dot(v2_, e2_) / max(dot(e2_, e2_), 1e-12)), 0.0, 1.0)));
    float s_1 = sign(((e0_.x * e2_.y) - (e0_.y * e2_.x)));
    float2 d = min(min(float2(dot(pq0_, pq0_), (s_1 * ((v0_.x * e0_.y) - (v0_.y * e0_.x)))), float2(dot(pq1_, pq1_), (s_1 * ((v1_.x * e1_.y) - (v1_.y * e1_.x))))), float2(dot(pq2_, pq2_), (s_1 * ((v2_.x * e2_.y) - (v2_.y * e2_.x)))));
    return (-(sqrt(d.x)) * sign(d.y));
}

float arrow_distance(float2 p_2, ArrowPlacement placed_2, float width_px_1)
{
    float distance_ = 1e20;
    bool local_37 = (bool)0;
    bool local_38 = (bool)0;

    float2 delta_2 = (placed_2.tip - placed_2.tail);
    float len_2 = length(delta_2);
    float2 dir_1 = (delta_2 / (max(len_2, 1e-12)).xx);
    float2 nrm_1 = float2(-(dir_1.y), dir_1.x);
    float2 head_base = (placed_2.tip - (dir_1 * placed_2.head_length_px));
    if (((len_2 - placed_2.head_length_px) > ARROW_SHAFT_MIN_PX)) {
        local_37 = (width_px_1 > 0.0);
    } else {
        local_37 = false;
    }
    bool _e30 = local_37;
    if (_e30) {
        float _e31 = distance_;
        const float _e33 = arrow_sd_segment(p_2, placed_2.tail, head_base);
        distance_ = min(_e31, (_e33 - (width_px_1 * 0.5)));
    }
    if ((placed_2.head_length_px > 0.0)) {
        local_38 = (placed_2.head_width_px > 0.0);
    } else {
        local_38 = false;
    }
    bool _e47 = local_38;
    if (_e47) {
        float2 half_ = (nrm_1 * (placed_2.head_width_px * 0.5));
        float _e52 = distance_;
        const float _e56 = arrow_sd_triangle(p_2, placed_2.tip, (head_base + half_), (head_base - half_));
        distance_ = min(_e52, _e56);
    }
    float _e58 = distance_;
    return _e58;
}

float arrow_coverage(float2 p_3, ArrowPlacement placed_3, float width_px_2, float aa_px)
{
    const float _e4 = arrow_distance(p_3, placed_3, width_px_2);
    float aa = max(aa_px, 0.001);
    return clamp((0.5 - (_e4 / aa)), 0.0, 1.0);
}

float axis_px_per_unit_at(float range, float panel_scale_1, float is_log_3, float prob_2, float pixel_to_ndc, float2 pair_4)
{
    float linear_ = ((2.0 * panel_scale_1) / (range * pixel_to_ndc));
    if ((prob_2 > 0.5)) {
        const float _e12 = prob_slope(pair_4, prob_2);
        return (linear_ * _e12);
    }
    float value_1 = (pair_4.x + pair_4.y);
    float log_slope = (linear_ / (value_1 * 2.3025851));
    if ((is_log_3 > 0.5)) {
        return log_slope;
    }
    return lerp(linear_, log_slope, is_log_3);
}

float2 ndc_to_px(float2 ndc_1)
{
    float2 _e3 = transform.pixel_to_ndc;
    return (ndc_1 / _e3);
}

float4 px_to_clip(float2 px_2)
{
    float2 _e3 = transform.pixel_to_ndc;
    return float4((px_2 * _e3), 0.0, 1.0);
}

float4 arrow_clip(float2 corner, ArrowPx arrow)
{
    const float4 _e2 = px_to_clip(corner);
    return _e2;
}

float axis_px_per_unit(uint axis_2, float2 pair_5)
{
    if ((axis_2 == 0u)) {
        float _e7 = transform.data_max.x;
        float _e11 = transform.data_min.x;
        float _e16 = transform.data_max_lo.x;
        float _e20 = transform.data_min_lo.x;
        float range_3 = mad(1.0, (_e7 - _e11), (_e16 - _e20));
        float _e27 = transform.data_min.x;
        float _e31 = transform.data_max.x;
        float _e35 = transform.data_min_lo.x;
        float _e39 = transform.data_max_lo.x;
        float _e43 = transform.scale_log.x;
        float _e47 = transform.scale_prob.x;
        const float _e48 = axis_data_t(pair_5, _e27, _e31, _e35, _e39, _e43, _e47);
        float _e52 = transform.data_to_panel_scale.x;
        float4 _e55[6] = transform.breaks;
        float4 _e58 = transform.break_info;
        const AxisCuts _e60 = cuts_of(_e55, _e58, 0u);
        const float _e61 = cut_scale_at(_e60, _e48);
        float _e66 = transform.scale_log.x;
        float _e70 = transform.scale_prob.x;
        float _e74 = transform.pixel_to_ndc.x;
        const float _e75 = axis_px_per_unit_at(range_3, (_e52 * _e61), _e66, _e70, _e74, pair_5);
        return _e75;
    }
    float _e79 = transform.data_max.y;
    float _e83 = transform.data_min.y;
    float _e88 = transform.data_max_lo.y;
    float _e92 = transform.data_min_lo.y;
    float range_4 = mad(1.0, (_e79 - _e83), (_e88 - _e92));
    float _e99 = transform.data_min.y;
    float _e103 = transform.data_max.y;
    float _e107 = transform.data_min_lo.y;
    float _e111 = transform.data_max_lo.y;
    float _e115 = transform.scale_log.y;
    float _e119 = transform.scale_prob.y;
    const float _e120 = axis_data_t(pair_5, _e99, _e103, _e107, _e111, _e115, _e119);
    float _e124 = transform.data_to_panel_scale.y;
    float4 _e127[6] = transform.breaks;
    float4 _e130 = transform.break_info;
    const AxisCuts _e132 = cuts_of(_e127, _e130, 1u);
    const float _e133 = cut_scale_at(_e132, _e120);
    float _e138 = transform.scale_log.y;
    float _e142 = transform.scale_prob.y;
    float _e146 = transform.pixel_to_ndc.y;
    const float _e147 = axis_px_per_unit_at(range_4, (_e124 * _e133), _e138, _e142, _e146, pair_5);
    return _e147;
}

ArrowPx project_arrow(VectorSample sample_, uint mode, float scale)
{
    ArrowPx out_1 = (ArrowPx)0;
    bool local_39 = (bool)0;
    bool local_40 = (bool)0;
    float2 tip_1 = (float2)0;
    bool local_41 = (bool)0;
    bool local_42 = (bool)0;
    bool local_43 = (bool)0;
    bool local_44 = (bool)0;
    bool local_45 = (bool)0;

    Transform tf = transform;
    out_1.tail = float2(0.0, 0.0);
    out_1.tip = float2(0.0, 0.0);
    out_1.depth = float2(0.0, 0.0);
    out_1.valid = false;
    float2 ox = float2(sample_.origin_hi.x, sample_.origin_lo.x);
    float2 oy = float2(sample_.origin_hi.y, sample_.origin_lo.y);
    if (!(!(sample_.valid))) {
        const bool _e35 = sample_valid(tf, ox, oy);
        local_39 = !(_e35);
    } else {
        local_39 = true;
    }
    bool _e38 = local_39;
    if (_e38) {
        ArrowPx _e39 = out_1;
        const ArrowPx arrowpx = _e39;
        return arrowpx;
    }
    float u = sample_.vector_.x;
    float v_10 = sample_.vector_.y;
    const bool _e44 = sample_finite(u);
    if (!(!(_e44))) {
        const bool _e49 = sample_finite(v_10);
        local_40 = !(_e49);
    } else {
        local_40 = true;
    }
    bool _e52 = local_40;
    if (_e52) {
        ArrowPx _e53 = out_1;
        const ArrowPx arrowpx_1 = _e53;
        return arrowpx_1;
    }
    const float2 _e54 = data_to_ndc(tf, ox, oy);
    const float2 _e55 = ndc_to_px(_e54);
    if ((mode == VECTOR_SCALE_PIXELS)) {
        const float _e60 = axis_px_per_unit(0u, ox);
        const float _e63 = axis_px_per_unit(1u, oy);
        float2 direction = float2((_e60 * u), (_e63 * v_10));
        float direction_len = length(direction);
        float len_px = (scale * length(float2(u, v_10)));
        const bool _e70 = sample_finite(direction_len);
        if (!(!(_e70))) {
            const bool _e75 = sample_finite(len_px);
            local_41 = !(_e75);
        } else {
            local_41 = true;
        }
        bool _e78 = local_41;
        if (_e78) {
            ArrowPx _e79 = out_1;
            const ArrowPx arrowpx_2 = _e79;
            return arrowpx_2;
        }
        if ((direction_len > 0.0)) {
            local_42 = (len_px != 0.0);
        } else {
            local_42 = false;
        }
        bool _e87 = local_42;
        if (_e87) {
            tip_1 = (_e55 + (direction * (len_px / direction_len)));
        } else {
            tip_1 = _e55;
        }
    } else {
        const float2 _e92 = pair_add(ox, (scale * u));
        const float2 _e94 = pair_add(oy, (scale * v_10));
        const bool _e95 = sample_valid(tf, _e92, _e94);
        if (!(_e95)) {
            ArrowPx _e97 = out_1;
            const ArrowPx arrowpx_3 = _e97;
            return arrowpx_3;
        }
        const float2 _e98 = data_to_ndc(tf, _e92, _e94);
        const float2 _e99 = ndc_to_px(_e98);
        tip_1 = _e99;
    }
    const bool _e101 = sample_finite(_e55.x);
    if (!(!(_e101))) {
        const bool _e107 = sample_finite(_e55.y);
        local_43 = !(_e107);
    } else {
        local_43 = true;
    }
    bool _e110 = local_43;
    if (!(_e110)) {
        float _e115 = tip_1.x;
        const bool _e116 = sample_finite(_e115);
        local_44 = !(_e116);
    } else {
        local_44 = true;
    }
    bool _e119 = local_44;
    if (!(_e119)) {
        float _e124 = tip_1.y;
        const bool _e125 = sample_finite(_e124);
        local_45 = !(_e125);
    } else {
        local_45 = true;
    }
    bool _e128 = local_45;
    if (_e128) {
        ArrowPx _e129 = out_1;
        const ArrowPx arrowpx_4 = _e129;
        return arrowpx_4;
    }
    out_1.tail = _e55;
    float2 _e132 = tip_1;
    out_1.tip = _e132;
    out_1.valid = true;
    ArrowPx _e135 = out_1;
    const ArrowPx arrowpx_5 = _e135;
    return arrowpx_5;
}

bool vector_flag(uint bit)
{
    uint _e3 = vector_params.flags;
    return ((_e3 & bit) != 0u);
}

float2 vector_pool_pair(uint base_1, uint i)
{
    uint j = (base_1 + (i * 2u));
    float _e7 = asfloat(vector_pool.Load(j*4));
    float _e12 = asfloat(vector_pool.Load((j + 1u)*4));
    return float2(_e7, _e12);
}

bool vector_pair_finite(float2 v_9)
{
    bool local_46 = (bool)0;
    bool local_47 = (bool)0;

    const bool _e2 = sample_finite(v_9.x);
    if (_e2) {
        const bool _e6 = sample_finite(v_9.y);
        local_46 = _e6;
    } else {
        local_46 = false;
    }
    bool _e8 = local_46;
    if (_e8) {
        const bool _e14 = sample_finite((v_9.x + v_9.y));
        local_47 = _e14;
    } else {
        local_47 = false;
    }
    bool _e16 = local_47;
    return _e16;
}

uint vector_scale_mode()
{
    const bool _e3 = vector_flag(VECTOR_FLAG_SCALE_PIXELS);
    return (_e3 ? VECTOR_SCALE_PIXELS : VECTOR_SCALE_DATA);
}

float vector_ramp_t(float2 zv_1)
{
    float2 _e3 = vector_params.z_min;
    float2 _e6 = vector_params.z_max;
    const bool _e8 = vector_flag(VECTOR_FLAG_LOG_Z);
    const float _e9 = ramp_t(zv_1, _e3, _e6, _e8);
    return _e9;
}

float4 vector_ramp(float t_14)
{
    uint n_1 = vector_params.stop_count;
    if ((n_1 == 0u)) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    if ((n_1 == 1u)) {
        float4 _e15 = asfloat(vector_stops.Load4(0));
        return _e15;
    }
    const float2 _e16 = ramp_segment(t_14, n_1);
    uint index_1 = naga_f2u32(_e16.x);
    float4 _e21 = asfloat(vector_stops.Load4(index_1*16));
    float4 _e26 = asfloat(vector_stops.Load4((index_1 + 1u)*16));
    const float4 _e28 = ramp_blend(_e21, _e26, _e16.y);
    return _e28;
}

float4 vector_color(VectorSample sample_1)
{
    bool local_48 = (bool)0;

    const bool _e2 = vector_flag(VECTOR_FLAG_COLOR_MAP);
    if (!(_e2)) {
        float4 _e6 = style_1.color_premul;
        return _e6;
    }
    const float _e8 = vector_ramp_t(sample_1.value);
    if (!(!(sample_1.value_valid))) {
        local_48 = (_e8 < 0.0);
    } else {
        local_48 = true;
    }
    bool _e17 = local_48;
    if (_e17) {
        float4 _e20 = vector_params.nan_color_premul;
        return _e20;
    }
    const float4 _e21 = vector_ramp(_e8);
    return float4((_e21.xyz * _e21.w), _e21.w);
}

VectorSample vector_sample(uint i_1)
{
    VectorSample sample_2 = (VectorSample)0;
    bool local_49 = (bool)0;

    sample_2.origin_hi = float3(0.0, 0.0, 0.0);
    sample_2.origin_lo = float3(0.0, 0.0, 0.0);
    sample_2.vector_ = float3(0.0, 0.0, 0.0);
    sample_2.value = float2(0.0, 0.0);
    sample_2.value_valid = false;
    sample_2.valid = false;
    uint _e27 = vector_params.count;
    if ((i_1 >= _e27)) {
        VectorSample _e29 = sample_2;
        const VectorSample vectorsample = _e29;
        return vectorsample;
    }
    uint _e32 = vector_params.x_base;
    const float2 _e33 = vector_pool_pair(_e32, i_1);
    uint _e36 = vector_params.y_base;
    const float2 _e37 = vector_pool_pair(_e36, i_1);
    uint _e40 = vector_params.u_base;
    const float2 _e41 = vector_pool_pair(_e40, i_1);
    uint _e44 = vector_params.v_base;
    const float2 _e45 = vector_pool_pair(_e44, i_1);
    const bool _e46 = vector_pair_finite(_e41);
    if (!(!(_e46))) {
        const bool _e51 = vector_pair_finite(_e45);
        local_49 = !(_e51);
    } else {
        local_49 = true;
    }
    bool _e54 = local_49;
    if (_e54) {
        VectorSample _e55 = sample_2;
        const VectorSample vectorsample_1 = _e55;
        return vectorsample_1;
    }
    sample_2.origin_hi = float3(_e33.x, _e37.x, 0.0);
    sample_2.origin_lo = float3(_e33.y, _e37.y, 0.0);
    sample_2.vector_ = float3((_e41.x + _e41.y), (_e45.x + _e45.y), 0.0);
    const bool _e76 = vector_flag(VECTOR_FLAG_THREE_D);
    if (_e76) {
        uint _e79 = vector_params.height_base;
        const float2 _e80 = vector_pool_pair(_e79, i_1);
        uint _e83 = vector_params.w_base;
        const float2 _e84 = vector_pool_pair(_e83, i_1);
        const bool _e85 = vector_pair_finite(_e84);
        if (!(_e85)) {
            VectorSample _e87 = sample_2;
            const VectorSample vectorsample_2 = _e87;
            return vectorsample_2;
        }
        sample_2.origin_hi.z = _e80.x;
        sample_2.origin_lo.z = _e80.y;
        sample_2.vector_.z = (_e84.x + _e84.y);
    }
    const bool _e100 = vector_flag(VECTOR_FLAG_COLOR_MAP);
    if (_e100) {
        uint _e104 = vector_params.c_base;
        const float2 _e105 = vector_pool_pair(_e104, i_1);
        sample_2.value = _e105;
        sample_2.value_valid = true;
    }
    sample_2.valid = true;
    VectorSample _e110 = sample_2;
    const VectorSample vectorsample_3 = _e110;
    return vectorsample_3;
}

ArrowStyle ConstructArrowStyle(float arg0, float arg1, float arg2, float arg3, uint arg4) {
    ArrowStyle ret = (ArrowStyle)0;
    ret.width_px = arg0;
    ret.head_length_px = arg1;
    ret.head_width_px = arg2;
    ret.min_length_px = arg3;
    ret.pivot = arg4;
    return ret;
}

ArrowStyle vector_arrow_style()
{
    float _e2 = vector_params.width_px;
    float _e5 = vector_params.head_length_px;
    float _e8 = vector_params.head_width_px;
    float _e11 = vector_params.min_length_px;
    uint _e14 = vector_params.pivot;
    const ArrowStyle arrowstyle = ConstructArrowStyle(_e2, _e5, _e8, _e11, _e14);
    return arrowstyle;
}

VectorVsOut vector_vertex(uint vertex_index_2, uint instance_index_1)
{
    VectorVsOut out_2 = (VectorVsOut)0;

    out_2.pos = float4(2.0, 2.0, 0.0, 1.0);
    out_2.px = float2(0.0, 0.0);
    out_2.tail = float2(0.0, 0.0);
    out_2.tip = float2(0.0, 0.0);
    out_2.head = float2(0.0, 0.0);
    out_2.color = float4(0.0, 0.0, 0.0, 0.0);
    const VectorSample _e31 = vector_sample(instance_index_1);
    const uint _e32 = vector_scale_mode();
    float _e35 = vector_params.scale;
    const ArrowPx _e36 = project_arrow(_e31, _e32, _e35);
    if (!(_e36.valid)) {
        VectorVsOut _e39 = out_2;
        const VectorVsOut vectorvsout = _e39;
        return vectorvsout;
    }
    const ArrowStyle _e42 = vector_arrow_style();
    const ArrowPlacement _e43 = arrow_place(_e36.tail, _e36.tip, _e42);
    if (!(_e43.valid)) {
        VectorVsOut _e46 = out_2;
        const VectorVsOut vectorvsout_1 = _e46;
        return vectorvsout_1;
    }
    float _e49 = vector_params.aa_px;
    float margin = (_e49 + 1.0);
    float _e54 = vector_params.width_px;
    const float2 _e55 = arrow_quad_corner(vertex_index_2, _e43, _e54, margin);
    const float4 _e57 = arrow_clip(_e55, _e36);
    out_2.pos = _e57;
    out_2.px = _e55;
    out_2.tail = _e43.tail;
    out_2.tip = _e43.tip;
    out_2.head = float2(_e43.head_length_px, _e43.head_width_px);
    const float4 _e68 = vector_color(_e31);
    out_2.color = _e68;
    VectorVsOut _e69 = out_2;
    const VectorVsOut vectorvsout_2 = _e69;
    return vectorvsout_2;
}

VertexOutput_vs_main vs_main(uint vertex_index : SV_VertexID, uint instance_index : SV_InstanceID)
{
    const VectorVsOut _e2 = vector_vertex(vertex_index, instance_index);
    const VectorVsOut vectorvsout_3 = _e2;
    const VertexOutput_vs_main vectorvsout_3_ = { vectorvsout_3.px, vectorvsout_3.tail, vectorvsout_3.tip, vectorvsout_3.head, vectorvsout_3.color, vectorvsout_3.pos };
    return vectorvsout_3_;
}

float4 fs_main(FragmentInput_fs_main fragmentinput_fs_main) : SV_Target0
{
    VectorVsOut in_ = { fragmentinput_fs_main.pos_1, fragmentinput_fs_main.px_4, fragmentinput_fs_main.tail_2, fragmentinput_fs_main.tip_3, fragmentinput_fs_main.head_1, fragmentinput_fs_main.color_1 };
    ArrowPlacement placed = (ArrowPlacement)0;

    placed.tail = in_.tail;
    placed.tip = in_.tip;
    placed.head_length_px = in_.head.x;
    placed.head_width_px = in_.head.y;
    placed.valid = true;
    ArrowPlacement _e15 = placed;
    float _e18 = vector_params.width_px;
    float _e21 = vector_params.aa_px;
    const float _e22 = arrow_coverage(in_.px, _e15, _e18, _e21);
    if ((_e22 <= 0.0)) {
        discard;
    }
    return (in_.color * _e22);
}
