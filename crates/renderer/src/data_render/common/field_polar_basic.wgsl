// The basic twin of `field_polar.wgsl` (#299): the same signature with no
// polar lookup, for a renderer that has drawn no polar chart, so its field
// pipelines do not compile it. A polar chart is never drawn or picked
// through this fragment (`shader_assembly::ShaderSet`).

/// Unreached on a rectangular chart (`field_lookup` tests the polar flag):
/// a miss, so a polar field draws nothing here.
fn field_polar_lookup(t: vec2<f32>, lattice: u32, cartesian: FieldLookup) -> FieldLookup {
    var out = cartesian;
    out.hit = false;
    return out;
}
