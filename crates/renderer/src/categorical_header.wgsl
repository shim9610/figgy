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
