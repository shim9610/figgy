//! The CPU twin of the polar mapping in the shaders' common `data_to_ndc`
//! block (`polar_ndc`, `polar_sample_valid` in `SHADER_COMMON.md`): the
//! transform constants a polar `Config` writes, and the projection the CPU
//! picker uses so it agrees with the GPU.

use crate::config::Config;
use crate::data_render::ScatterTransform;

/// `(polar_angle, polar_extent)` of [`ScatterTransform`] for `config`: all
/// zero for a rectangular chart.
pub(crate) fn transform_constants(config: &Config) -> ([f32; 4], [f32; 4]) {
    let (Some(polar), Some(geometry)) = (&config.polar, config.polar_geometry()) else {
        return ([0.0; 4], [0.0; 4]);
    };
    let [start, end] = polar.sector.unwrap_or([0.0, 0.0]);
    (
        [
            1.0,
            polar.zero_angle_degrees.to_radians() as f32,
            (polar.direction() * polar.angle_unit.radians_per_unit()) as f32,
            polar.inner_radius_fraction,
        ],
        [
            start as f32,
            end as f32,
            crate::line_step::arc_pieces(config.polar.as_ref()) as f32,
            geometry.radius,
        ],
    )
}

/// Whether the transform maps polar coordinates.
pub(crate) fn is_polar(transform: &ScatterTransform) -> bool {
    transform.polar_angle[0] > 0.5
}

/// The axis fraction of a y value through the transform's y axis, with the
/// panel affine removed: 0 at the axis start, 1 at its end.
fn radius_fraction(transform: &ScatterTransform, panel_ty: f32) -> f32 {
    (panel_ty - transform.data_to_panel_offset[1]) / transform.data_to_panel_scale[1]
}

/// Twin of `polar_sample_valid` and the pick shader's `pick_polar_valid`: a
/// radius below the axis start is not drawn, so it is neither picked nor
/// connected. A sample outside the sector keeps its projection; the
/// fragment clip cuts what is drawn there ([`clip_distance`]).
pub(crate) fn sample_valid(transform: &ScatterTransform, panel_ty: f32) -> bool {
    !is_polar(transform) || radius_fraction(transform, panel_ty) >= 0.0
}

/// Twin of the draw shaders' `polar_fragment_visible`: whether the fragment
/// at `px` (pixels from the polar centre, y up) survives the clip at the
/// outer ring, the hole and the sector.
fn fragment_visible(px: [f32; 2], limits: [f32; 4], sector: [f32; 4]) -> bool {
    if limits[0] < 0.5 {
        return true;
    }
    let r = (px[0] * px[0] + px[1] * px[1]).sqrt();
    if r > limits[1] + 0.5 || r < limits[2] - 0.5 {
        return false;
    }
    if limits[3] < 0.5 {
        return true;
    }
    let turn = std::f32::consts::TAU / sector[1].abs();
    let mut a = (px[1].atan2(px[0]) - sector[0]) / sector[1];
    a -= ((a - sector[2]) / turn).floor() * turn;
    a <= sector[3] + 1e-6 * turn
}

fn ray_distance(px: [f32; 2], angle: f32) -> f32 {
    let dir = [angle.cos(), angle.sin()];
    let along = px[0] * dir[0] + px[1] * dir[1];
    if along <= 0.0 {
        return (px[0] * px[0] + px[1] * px[1]).sqrt();
    }
    (px[0] * dir[1] - px[1] * dir[0]).abs()
}

/// Twin of the pick shader's `pick_polar_clip_distance`: zero where the
/// polar fragment clip keeps the fragment under `cursor` (canvas pixels),
/// else a positive lower bound of the distance to the kept region. Zero for
/// a rectangular chart.
pub(crate) fn clip_distance(
    transform: &ScatterTransform,
    config: &Config,
    cursor: [f32; 2],
) -> f32 {
    if !is_polar(transform) {
        return 0.0;
    }
    let ca = config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;
    let ndc = [
        (cursor[0] - ca.x as f32) / chart_w * 2.0 - 1.0,
        1.0 - (cursor[1] - ca.y as f32) / chart_h * 2.0,
    ];
    let center = [
        (transform.data_to_panel_offset[0] + transform.data_to_panel_scale[0] * 0.5) * 2.0 - 1.0,
        (transform.data_to_panel_offset[1] + transform.data_to_panel_scale[1] * 0.5) * 2.0 - 1.0,
    ];
    let px = [
        (ndc[0] - center[0]) / transform.pixel_to_ndc[0],
        (ndc[1] - center[1]) / transform.pixel_to_ndc[1],
    ];
    let [flag, zero, scale, inner] = transform.polar_angle;
    let [start, end, _, outer] = transform.polar_extent;
    let limits = [
        flag,
        outer,
        outer * inner,
        if end > start { 1.0 } else { 0.0 },
    ];
    let sector = [zero, scale, start, end];
    if fragment_visible(px, limits, sector) {
        return 0.0;
    }
    let r = (px[0] * px[0] + px[1] * px[1]).sqrt();
    let mut d = (r - (limits[1] + 0.5)).max((limits[2] - 0.5) - r);
    if limits[3] >= 0.5 && !fragment_visible(px, [limits[0], f32::MAX, 0.0, limits[3]], sector) {
        let a = zero + scale * start;
        let b = zero + scale * end;
        d = d.max(ray_distance(px, a).min(ray_distance(px, b)));
    }
    d.max(f32::EPSILON)
}

/// Twin of `polar_ndc`: the NDC position of the sample whose angle column
/// value is `x` and whose y axis panel coordinate is `panel_ty`.
pub(crate) fn ndc(transform: &ScatterTransform, x: f32, panel_ty: f32) -> [f32; 2] {
    let [_, zero, scale, inner] = transform.polar_angle;
    let radius_px = transform.polar_extent[3];
    let angle = zero + scale * x;
    let rt = radius_fraction(transform, panel_ty).max(0.0);
    let r = inner + (1.0 - inner) * rt;
    let center = [
        (transform.data_to_panel_offset[0] + transform.data_to_panel_scale[0] * 0.5) * 2.0 - 1.0,
        (transform.data_to_panel_offset[1] + transform.data_to_panel_scale[1] * 0.5) * 2.0 - 1.0,
    ];
    [
        center[0] + angle.cos() * r * radius_px * transform.pixel_to_ndc[0],
        center[1] + angle.sin() * r * radius_px * transform.pixel_to_ndc[1],
    ]
}
