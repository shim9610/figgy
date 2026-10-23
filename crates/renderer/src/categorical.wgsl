// Independent bounded categorical rectangles. No Cartesian shared blocks.
struct Globals { size:vec4<f32>, background:vec4<f32> };
struct Bar {
    rect:vec4<f32>, radii:vec4<f32>, color:vec4<f32>, material:Material,
    outline:vec4<f32>, outline_color:vec4<f32>, effects:vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals:Globals;
@group(0) @binding(1) var<storage,read> bars:array<Bar>;
@group(0) @binding(2) var annotations:texture_2d<f32>;
struct Vertex { @builtin(position) position:vec4<f32>, @location(0) @interpolate(flat) layer:u32 };
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->Vertex {
    let uv=array<vec2<f32>,6>(vec2(0.0,0.0),vec2(0.0,1.0),vec2(1.0,0.0),vec2(1.0,0.0),vec2(0.0,1.0),vec2(1.0,1.0));
    var lo=vec2(0.0);var hi=globals.size.xy;
    if instance>0u && instance<=u32(globals.size.z) { let b=bars[instance-1u];lo=b.rect.xy-1.0;hi=b.rect.zw+1.0; }
    let p=mix(lo,hi,uv[vertex]);
    return Vertex(vec4(p.x/globals.size.x*2.0-1.0,1.0-p.y/globals.size.y*2.0,0.0,1.0),instance);
}

@fragment fn fs(v:Vertex)->@location(0) vec4<f32> {
    let p=v.position.xy;
    if v.layer==0u {
        let ink=textureLoad(annotations,vec2<i32>(p),0);
        let bg=vec4(globals.background.rgb*globals.background.a,globals.background.a);
        return output(ink+bg*(1.0-ink.a));
    }
    if v.layer>u32(globals.size.z) { return output(textureLoad(annotations,vec2<i32>(p)+vec2(0,i32(globals.size.y)),0)); }
    let b=bars[v.layer-1u];var coverage=0.0;var border=0.0;
    // Half-open ownership at shared stack edges; AA remains on the silhouette.
    // Two separately alpha-blended partial coverages must not expose background.
    let edge_mask=u32(b.effects.z);
    if ((edge_mask&1u)!=0u && p.x<b.rect.x) || ((edge_mask&2u)!=0u && p.x>=b.rect.z)
        || ((edge_mask&4u)!=0u && p.y<b.rect.y) || ((edge_mask&8u)!=0u && p.y>=b.rect.w) { discard; }
    for(var y=0u;y<2u;y++) { for(var x=0u;x<2u;x++) {
        var sample=p+(vec2(f32(x),f32(y))+0.5)*0.5-0.5;
        if (edge_mask&1u)!=0u { sample.x=max(sample.x,b.rect.x); }
        if (edge_mask&2u)!=0u { sample.x=min(sample.x,b.rect.z); }
        if (edge_mask&4u)!=0u { sample.y=max(sample.y,b.rect.y); }
        if (edge_mask&8u)!=0u { sample.y=min(sample.y,b.rect.w); }
        let d=sdf(sample,b);
        coverage+=select(0.0,0.25,d<=0.0);
        border+=select(0.0,0.25,d<=0.0 && d>=-b.outline.y && b.outline.x>0.5);
    }}
    let c=mix(shade(p,b),b.outline_color.rgb,b.outline_color.a*border/max(coverage,0.001));
    return output(vec4(c*coverage,coverage));
}
