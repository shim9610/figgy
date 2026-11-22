// Test support for the radial shading tolerance test (#349), never product
// code: appended to the assembled radial module (`RADIAL_SHADER`), it adds
// the entry the draw ran before it shaded once per surface. Every
// antialiasing sample is shaded at its own position, with its own lit
// material and outline, and the floor shadow is taken at every background
// sample, from the same functions the draw calls.
fn scene_per_sample(pixel: vec2<f32>) -> vec4<f32> {
    let shade=floor_shadow(pixel);
    var color=vec4(globals.background.rgb*globals.background.a*(1.0-shade),
        shade+globals.background.a*(1.0-shade));
    let c=coverage(pixel);
    if c.winner!=0u {
        let s=sectors[c.winner-1u];
        color=vec4(outlined(pixel,c.hit,s,sector_surface(c.hit,s)),1.0);
    }
    return color;
}
@fragment fn fs_per_sample(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    var color=vec4(0.0);let grid=u32(globals.size.w);
    var flat=FlatPixel(false,vec4(0.0));
    if globals.size.z<1.0 || u32(sectors[0].material.params.x)==MATERIAL_FLAT { flat=flat_pixel(p.xy,0.75); }
    if flat.exact {
        for(var k=0u;k<grid*grid;k++) { color+=flat.color; }
    } else {
        for(var y=0u;y<grid;y++) { for(var x=0u;x<grid;x++) {
            color+=scene_per_sample(p.xy+(vec2(f32(x),f32(y))+0.5)/f32(grid)-0.5);
        }}
    }
    color/=f32(grid*grid);
    let text=label_pixel(p.xy);
    color=text+color*(1.0-text.a);
    if globals.group_info.y>0.0 && color.a>0.0 {
        let c=color.rgb/color.a;
        let linear=select(pow((c+0.055)/1.055,vec3(2.4)),c/12.92,c<=vec3(0.04045));
        color=vec4(linear*color.a,color.a);
    }
    return color;
}
