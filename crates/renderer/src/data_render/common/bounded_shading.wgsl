fn sdf(p:vec2<f32>,b:Bar)->f32 {
    let q=p-(b.rect.xy+b.rect.zw)*0.5;
    var r=select(b.radii.w,b.radii.z,q.x>=0.0);
    if q.y<0.0 { r=select(b.radii.x,b.radii.y,q.x>=0.0); }
    let d=abs(q)-(b.rect.zw-b.rect.xy)*0.5+r;
    return length(max(d,vec2(0.0)))+min(max(d.x,d.y),0.0)-r;
}
// A bar or a box is a flat body shaded like a flat (zero depth, zero tilt)
// pie top: constant normal and view (0,0,1), RadialStyle's default light,
// and the bar's own object coordinates (SHADER_COMMON.md §5.9).
// One object unit in logical pixels. It plays the sector radius: Matte grain
// cells (1/90 unit) are 1.67 logical px, as on bars before the material was
// shared, and a bar's texture matches a sector of this radius.
const BAR_UNIT_PX: f32 = 150.0;
// `RadialStyle::light`'s default, normalized at the call.
const BAR_LIGHT: vec3<f32> = vec3(-0.5,-0.6,1.0);
fn bar_surface(p:vec2<f32>,b:Bar)->vec3<f32> {
    // Object coordinates: logical pixels from the bar's top-left corner, in
    // units of BAR_UNIT_PX.
    let o=(p-b.rect.xy)/(b.effects.y*BAR_UNIT_PX);
    let horizontal=b.outline.z>0.5;
    let along=select(o.y,o.x,horizontal);
    let across=select(o.x,o.y,horizontal);
    // The metal brush runs along the bar.
    let tangent=select(vec3(0.0,1.0,0.0),vec3(1.0,0.0,0.0),horizontal);
    let up=vec3(0.0,0.0,1.0);
    return material_surface(b.material,b.color.rgb,vec3(o,0.0),up,up,normalize(BAR_LIGHT),tangent,
        vec2(across,along*2.0),BAR_UNIT_PX*b.effects.y);
}
