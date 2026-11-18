// The polar half of the field grid's cell lookup (#208): `field_lookup`
// (`field_grid.wgsl`) hands a polar chart's fragment here. The full set
// of a module that splices `field_grid.wgsl` splices it;
// `field_polar_basic.wgsl` is its twin for a renderer that has drawn no polar
// chart (#299), with the same signature.

/// `field_lookup` on a polar chart, given the rectangular answer at
/// chart-area `t`.
fn field_polar_lookup(t: vec2<f32>, lattice: u32, cartesian: FieldLookup) -> FieldLookup {
    let tf = transform;
    var out = cartesian;
    let px = polar_clip_offset(tf, t * 2.0 - vec2<f32>(1.0, 1.0));
    if (!polar_fragment_visible(px, polar_clip_limits(tf), polar_clip_sector(tf))) {
        out.hit = false;
        return out;
    }
    let u = polar_unproject(tf, px);
    // The angle axis is linear on a polar chart: one angle unit is `kx` of
    // `t`, and a turn `turn_t`.
    let range_x = (transform.data_max.x - transform.data_min.x)
        + (transform.data_max_lo.x - transform.data_min_lo.x);
    let kx = transform.data_to_panel_scale.x / range_x;
    let turn_t = abs(6.283185307179586 / transform.polar_angle.z * kx);
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let count = quad_count(along_cells, lattice);
    let b0 = boundary_t(field.x_base, field.x_len, 0u, 0u, lattice);
    let b1 = boundary_t(field.x_base, field.x_len, count, 0u, lattice);
    var tx = axis_t(vec2<f32>(u.angle, 0.0), 0u);
    if (!f32_is_finite(tx) || !f32_is_finite(turn_t) || turn_t <= 0.0
        || !f32_is_finite(b0) || !f32_is_finite(b1)) {
        out.hit = false;
        return out;
    }
    let low = min(b0, b1);
    tx = tx - floor((tx - low) / turn_t) * turn_t;
    out.t = vec2<f32>(tx, u.ty);
    out.jac_x = u.d_angle * kx;
    out.jac_y = u.d_ty;
    return out;
}
