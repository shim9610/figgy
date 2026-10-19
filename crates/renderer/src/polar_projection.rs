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
/// connected. With `sector`, a sample outside the sector is rejected too: a
/// marker there is clipped away entirely, while a line keeps its pieces,
/// which the clip cuts at the edge.
pub(crate) fn sample_valid(
    transform: &ScatterTransform,
    x: f32,
    panel_ty: f32,
    sector: bool,
) -> bool {
    if !is_polar(transform) {
        return true;
    }
    if radius_fraction(transform, panel_ty) < 0.0 {
        return false;
    }
    let [start, end, _, _] = transform.polar_extent;
    !sector || end <= start || (x >= start && x <= end)
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
