// Independent bounded categorical rectangles. No Cartesian shared blocks.
struct Globals { size:vec4<f32>, background:vec4<f32> };
struct Bar {
    rect:vec4<f32>, radii:vec4<f32>, color:vec4<f32>, material:vec4<f32>,
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
fn sdf(p:vec2<f32>,b:Bar)->f32 {
    let q=p-(b.rect.xy+b.rect.zw)*0.5;
    var r=select(b.radii.w,b.radii.z,q.x>=0.0);
    if q.y<0.0 { r=select(b.radii.x,b.radii.y,q.x>=0.0); }
    let d=abs(q)-(b.rect.zw-b.rect.xy)*0.5+r;
    return length(max(d,vec2(0.0)))+min(max(d.x,d.y),0.0)-r;
}
fn hash(p:vec2<f32>)->f32 { return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
fn noise(p:vec2<f32>)->f32 { let i=floor(p);let f=fract(p);let u=f*f*(3.0-2.0*f);return mix(mix(hash(i),hash(i+vec2(1.0,0.0)),u.x),mix(hash(i+vec2(0.0,1.0)),hash(i+1.0),u.x),u.y); }
fn shade(p:vec2<f32>,b:Bar)->vec3<f32> {
    var c=b.color.rgb;let kind=u32(b.material.x);
    let uv=(p-b.rect.xy)/max(b.rect.zw-b.rect.xy,vec2(0.001));
    let across=select(uv.x,uv.y,b.outline.z>0.5);
    let logical=(p-b.rect.xy)/b.effects.y;
    let grain=noise(logical*b.material.z*0.6+vec2(b.outline.w));
    if kind==1u { c*=0.98+(grain-0.5)*b.material.y*0.08+0.04*(1.0-uv.y); }
    if kind==2u {
        let brush=noise(vec2(logical.x*0.06,logical.y*1.5)*b.material.z);
        let light=exp(-pow((across-0.25)/0.23,2.0));
        c=c*(0.82+(brush-0.5)*b.material.y*0.09)+vec3(light*b.material.w*0.25);
    }
    if kind==3u { c=c*(0.92+0.08*(1.0-across))+vec3(exp(-pow((across-0.18)/0.16,2.0))*b.material.w*0.16); }
    if kind==4u { c*=1.0+(grain-0.5)*b.material.y*0.16; }
    return clamp(c+vec3(b.effects.x),vec3(0.0),vec3(1.0));
}
fn output(c:vec4<f32>)->vec4<f32> {
    if globals.size.w>0.5 && c.a>0.0 {
        let rgb=c.rgb/c.a;let linear=select(pow((rgb+0.055)/1.055,vec3(2.4)),rgb/12.92,rgb<=vec3(0.04045));
        return vec4(linear*c.a,c.a);
    }
    return c;
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
