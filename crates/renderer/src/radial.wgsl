// Independent radial geometry in screen/radius coordinates. No Cartesian
// Transform/Style fragments are used here.
struct Globals {
    size: vec4<f32>, // width, height, slice count, AA grid side (2 or 3)
    background: vec4<f32>,
    light: vec4<f32>,
    shadow: vec4<f32>,
    groups: array<vec4<f32>, 2>, // center.xy, radius, cos(tilt)
    group_info: vec4<f32>, // group count, output sRGB, inner radii
};
// [data_render/common/radial_sector.wgsl: struct Sector]
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> sectors: array<Sector>;
@group(0) @binding(2) var annotations: texture_2d<f32>;
// [data_render/common/radial_sector.wgsl: TAU, Hit and the sector ray-march, round_intersection to hit_pixel]
fn crosses_top_wall(a: Hit,b: Hit) -> bool {
    // Classify the visible geometric surface, including the middle of a bevel.
    // Testing first hits avoids drawing hidden edges through the solid.
    return b.t<9999.0 && (a.n.z>=0.70710678)!=(b.n.z>=0.70710678);
}
fn scene(pixel: vec2<f32>) -> vec4<f32> {
    var color=vec4(globals.background.rgb*globals.background.a,globals.background.a);
    // Per-slice floor shadows follow lift without changing the chart bounds.
    var shade=0.0;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let s=sectors[i];
        if s.light.w<=0.0 { continue; }
        let cs=cos(s.arc.z);let mid=s.arc.x+s.arc.y*0.5;
        let elevation=s.arc.w+s.rounding.z;
        let offset=vec2(0.05,0.08)+normalize(-s.light.xy+vec2(0.00001))*elevation*0.3;
        let xy=(pixel-s.geometry.xy)/s.geometry.z-vec2(cos(mid),sin(mid)*cs)*s.detail.x-offset;
        let d=footprint(vec2(xy.x,xy.y/cs),s);
        let blur=0.025+elevation*0.13;
        shade=max(shade,(1.0-smoothstep(-blur,blur*2.0,d))*0.16);
    }
    color=vec4(color.rgb*(1.0-shade),shade+color.a*(1.0-shade));
    var closest=10000.0;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let s=sectors[i];
        let cs=cos(s.arc.z);let sn=sin(s.arc.z);
        let rd=vec3(0.0,-sn,-cs);
        let h=hit_pixel(pixel,s);
        if h.t<closest {
            closest=h.t;
            // The brush of BrushedMetal and SatinMetal runs around the
            // center: its tangent is circumferential in the turned texture
            // frame, its coordinates are (radius, twice the turned angle).
            let ca=cos(s.material.finish.z);let sa=sin(s.material.finish.z);
            let turned=vec2(h.p.x*ca-h.p.y*sa,h.p.x*sa+h.p.y*ca)*s.material.params.w;
            let direction=vec3(-h.p.y*ca-h.p.x*sa,-h.p.y*sa+h.p.x*ca,0.0);
            let tangent=normalize(direction-h.n*dot(h.n,direction)+vec3(0.000001,0.0,0.0));
            var rgb=material_surface(s.material,s.color.rgb,h.p,h.n,-rd,normalize(s.light.xyz),tangent,
                vec2(length(h.p.xy),atan2(turned.y,turned.x)*2.0),s.geometry.z);
            let d=edges(h.p.xy,s);
            let rim_distance=min(abs(d.x),abs(d.y));
            let separation=abs(d.z);
            let width=s.outline.w/s.geometry.z;
            let is_rim=rim_distance<=separation;
            let enabled=select(s.outline.y,s.outline.x,is_rim);
            if max(enabled,s.outline.z)>0.0 && width>0.0 {
                let band=s.outline.w*select(0.5,1.0,is_rim);
                let offsets=array<vec2<f32>,4>(vec2(band,0.0),vec2(-band,0.0),vec2(0.0,band),vec2(0.0,-band));
                var border=false;
                for(var j=0u;j<4u;j++) {
                    let neighbor=hit_pixel(pixel+offsets[j],s);
                    border=neighbor.t>=9999.0;
                    if !border && crosses_top_wall(h,neighbor) {
                        // A crease is inked on both faces, unlike the inward
                        // silhouette band. Keep the total logical width equal.
                        if is_rim {
                            border=crosses_top_wall(h,hit_pixel(pixel+offsets[j]*0.5,s));
                        } else { border=true; }
                    }
                    if border { break; }
                }
                if border { rgb=mix(rgb,s.outline_color.rgb,s.outline_color.a*max(enabled,s.outline.z)); }
            }
            color=vec4(rgb,1.0);
        }
    }
    return color;
}
// Outside labels stay fixed during interaction. Inside glyphs translate with
// their slice by sampling the unchanged full-resolution annotation texture.
fn label_pixel(pixel: vec2<f32>) -> vec4<f32> {
    var text=textureLoad(annotations,vec2<i32>(pixel),0);
    var moving=false;
    for(var i=0u;i<u32(globals.size.z);i++) {
        var s=sectors[i];
        if s.effects.z>0.0 && s.rounding.z>0.0 {
            moving=true;s.rounding.z=0.0;
            let rest=hit_pixel(pixel,s);
            if rest.t<9999.0 && rest.n.z>0.5 { text=vec4(0.0); }
        }
    }
    if !moving { return text; }
    var closest=10000.0;var winner=0u;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let h=hit_pixel(pixel,sectors[i]);
        if h.t<closest {closest=h.t;winner=i;}
    }
    if closest<9999.0 {
        let s=sectors[winner];
        if s.effects.z>0.0 && s.rounding.z>0.0 {
            let offset=vec2(0.0,s.effects.w);
            let location=pixel+offset;
            let lo=vec2<i32>(floor(location-0.5));let f=fract(location-0.5);
            // Subpixel translation only, no rescaling or repeated texture feedback.
            text=mix(mix(textureLoad(annotations,lo,0),textureLoad(annotations,lo+vec2(1,0),0),f.x),
                     mix(textureLoad(annotations,lo+vec2(0,1),0),textureLoad(annotations,lo+vec2(1,1),0),f.x),f.y);
        }
    }
    return text;
}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));return vec4(p[i],0.0,1.0);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    var color=vec4(0.0);let grid=u32(globals.size.w);
    for(var y=0u;y<grid;y++) { for(var x=0u;x<grid;x++) {
        color+=scene(p.xy+(vec2(f32(x),f32(y))+0.5)/f32(grid)-0.5);
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
