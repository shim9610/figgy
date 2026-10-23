// Independent precomputed box plot components. No Cartesian shared blocks.
struct Globals { size:vec4<f32>, background:vec4<f32>, clip:vec4<f32> };
struct Bar {
    rect:vec4<f32>, radii:vec4<f32>, color:vec4<f32>, material:vec4<f32>,
    outline:vec4<f32>, outline_color:vec4<f32>, effects:vec4<f32>, notch:vec4<f32>, body:vec4<f32>,
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
// A box: the notched outline, the diamond, or the shared rounded rectangle.
fn box_sdf(p:vec2<f32>,b:Bar)->f32 {
    if b.effects.z==1.0 {
        let point=select(p,p.yx,b.outline.z>0.5);
        let l=b.body.x;let a=b.body.y;let r=b.body.z;let z=b.body.w;
        let low=b.notch.x;let mid=b.notch.y;let high=b.notch.z;let depth=b.notch.w;
        let vertices=array<vec2<f32>,10>(vec2(l,a),vec2(r,a),vec2(r,low),vec2(r-depth,mid),vec2(r,high),vec2(r,z),vec2(l,z),vec2(l,high),vec2(l+depth,mid),vec2(l,low));
        var inside=false;var distance=1e30;
        for(var i=0u;i<10u;i++) {
            let va=vertices[i];let vb=vertices[(i+1u)%10u];let edge=vb-va;let q=point-va;
            let t=clamp(dot(q,edge)/max(dot(edge,edge),1e-20),0.0,1.0);
            distance=min(distance,length(q-edge*t));
            if (va.y>point.y)!=(vb.y>point.y) {
                if point.x<va.x+(point.y-va.y)*edge.x/edge.y { inside=!inside; }
            }
        }
        return select(distance,-distance,inside);
    }
    if b.effects.z==2.0 {
        let q=p-(b.rect.xy+b.rect.zw)*0.5;
        return (abs(q.x)+abs(q.y)-(b.rect.z-b.rect.x)*0.5)*0.70710677;
    }
    return sdf(p,b);
}
