// Test support for the radial shading tolerance test (#349), never product
// code: appended to the assembled radial module (`RADIAL_SHADER`), it adds
// the entry the draw ran before it shaded once per surface (`fs_per_sample`). Every
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
// The Flat draw before its analytic edges: every pixel the fast path does not
// settle by its margin takes the 2 x 2 samples, each the sector it hits or the
// background, as the shipped draw did. The radial flat-edge tolerance test
// compares the shipped draw with it.
@fragment fn fs_supersampled(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    var color=vec4(0.0);let grid=u32(globals.size.w);
    for(var y=0u;y<grid;y++) { for(var x=0u;x<grid;x++) {
        color+=scene_per_sample(p.xy+(vec2(f32(x),f32(y))+0.5)/f32(grid)-0.5);
    }}
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
// The draw before one ray per material pixel (#454): the shipped `fs` of
// #439, every pixel the Flat fast path does not settle taking its 2 x 2
// samples, each shaded once per surface. The radial material-edge tolerance
// test compares the shipped draw with it.
@fragment fn fs_material_supersampled(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    var color=vec4(0.0);let grid=u32(globals.size.w);
    // The samples lie within 0.48 px of the center (a 3x3 grid's corner);
    // 0.75 px covers that times sqrt(2). `flat_pixel` refuses a chart whose
    // first sector is not Flat at that sector, so such a chart skips the call:
    // entering it at every pixel cost a Ceramic donut 5% on Apple M4 (#41).
    var flat=FlatPixel(false,vec4(0.0));
    if globals.size.z<1.0 || u32(sectors[0].material.params.x)==MATERIAL_FLAT { flat=flat_pixel(p.xy,0.75); }
    // Branch once, outside the sample loops: the same choice made per sample
    // inside them left some pixels zero on lavapipe (Mesa 25.2.8).
    if flat.exact {
        // The same sum in the same order as the samples, so a pixel away
        // from every edge is bit-identical to theirs.
        for(var k=0u;k<grid*grid;k++) { color+=flat.color; }
    } else {
        // Supersample coverage, shade once per surface (#349). Every sample
        // finds the sector it hits, or the background, at its own position,
        // and takes its outline there, so edges and ink keep their
        // antialiasing. The lit material is evaluated once for the samples
        // that hit one sector with one normal, at the first of them, and
        // used for each, added in sample order: an interior pixel of a face
        // evaluates one material, while samples across a bevel or a crease
        // keep their own normal and highlight. The floor shadow, a soft
        // blur, is taken once at the pixel center.
        let shade=floor_shadow(p.xy);
        let ground=vec4(globals.background.rgb*globals.background.a*(1.0-shade),
            shade+globals.background.a*(1.0-shade));
        // Indexed by sample: a grid side is at most 3 (`Globals::size.w`).
        var winners=array<u32,9>();var normals=array<vec3<f32>,9>();var surfaces=array<vec3<f32>,9>();
        for(var y=0u;y<grid;y++) { for(var x=0u;x<grid;x++) {
            let k=y*grid+x;
            let at=p.xy+(vec2(f32(x),f32(y))+0.5)/f32(grid)-0.5;
            let c=coverage(at);
            winners[k]=c.winner;normals[k]=c.hit.n;
            if c.winner==0u { color+=ground; continue; }
            // One normal: within about 2.5 degrees, so a face's curvature
            // across a pixel shares and a bevel's turn does not.
            var first=k;
            for(var j=0u;j<k;j++) {
                if winners[j]==c.winner && dot(normals[j],c.hit.n)>0.999 { first=j;break; }
            }
            let s=sectors[c.winner-1u];
            // Every slot is filled: the test is not transitive, so a later
            // sample may share with one that shared with an earlier one.
            if first==k { surfaces[k]=sector_surface(c.hit,s); } else { surfaces[k]=surfaces[first]; }
            color+=vec4(outlined(at,c.hit,s,surfaces[k]),1.0);
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
